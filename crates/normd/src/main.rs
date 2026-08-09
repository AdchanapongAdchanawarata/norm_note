//! norm_note sync daemon and CLI.
//!
//! v0.1 syncs a Markdown vault through a folder the user already owns — a
//! Dropbox or iCloud Drive folder, a NAS mount, an external disk. There is no
//! server, and no network code in this binary at all (see `deny.toml`).

mod keyring;
mod recovery;
mod status;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use norm_core::config::VaultConfig;
use norm_core::oplog::store::ChunkStore;
use norm_core::workspace::Workspace;

#[derive(Parser)]
#[command(name = "normd", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Print more detail about what is happening.
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Prepare a vault for syncing and print its recovery phrase.
    Init {
        /// Vault directory. Defaults to the current directory.
        #[arg(default_value = ".")]
        vault: PathBuf,

        /// Folder to sync through — a path inside Dropbox, iCloud Drive, a NAS
        /// mount, anywhere you control. Nothing readable is written there.
        #[arg(long)]
        target: Option<PathBuf>,

        /// Encrypt the vault key on this device with a passphrase.
        #[arg(long)]
        encrypt: bool,
    },

    /// Join a vault that already exists on another device.
    Join {
        #[arg(default_value = ".")]
        vault: PathBuf,

        /// Folder the other device syncs through.
        #[arg(long)]
        target: PathBuf,

        /// Recovery phrase printed by `normd init` on the first device.
        #[arg(long)]
        recovery: String,

        /// Encrypt the vault key on this device with a passphrase.
        #[arg(long)]
        encrypt: bool,
    },

    /// Watch the vault and keep it in sync. This is the daemon.
    Watch {
        #[arg(default_value = ".")]
        vault: PathBuf,

        /// Seconds between passes.
        #[arg(long, default_value_t = 5)]
        interval: u64,
    },

    /// Run a single sync pass and stop.
    Sync {
        #[arg(default_value = ".")]
        vault: PathBuf,
    },

    /// Show what is in sync and what is not, per device, with no rounding up.
    Status {
        #[arg(default_value = ".")]
        vault: PathBuf,
    },

    /// Check vault integrity: every chunk decodes, state is consistent.
    Doctor {
        #[arg(default_value = ".")]
        vault: PathBuf,
    },

    /// Manage soft-deleted notes in the local trash.
    Trash {
        #[command(subcommand)]
        command: TrashCommand,
    },
}

#[derive(Subcommand)]
enum TrashCommand {
    /// List all notes in the trash.
    List {
        #[arg(default_value = ".")]
        vault: PathBuf,
    },
    /// Restore a note from the trash.
    Restore {
        #[arg(default_value = ".")]
        vault: PathBuf,
        /// The hash of the trash entry to restore.
        hash: String,
    },
    /// Permanently delete notes in the trash older than max-age.
    Empty {
        #[arg(default_value = ".")]
        vault: PathBuf,
        /// Number of days to keep trash.
        #[arg(long, default_value_t = 30)]
        days: u32,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(if cli.verbose { "debug" } else { "info" })
        .with_target(false)
        .without_time()
        .init();

    match cli.command {
        Command::Init {
            vault,
            target,
            encrypt,
        } => init(&vault, target, encrypt),
        Command::Join {
            vault,
            target,
            recovery,
            encrypt,
        } => join(&vault, target, &recovery, encrypt),
        Command::Watch { vault, interval } => watch(&vault, interval),
        Command::Sync { vault } => sync_once(&vault),
        Command::Status { vault } => status::show(&vault),
        Command::Doctor { vault } => doctor(&vault),
        Command::Trash { command } => match command {
            TrashCommand::List { vault } => trash_list(&vault),
            TrashCommand::Restore { vault, hash } => trash_restore(&vault, &hash),
            TrashCommand::Empty { vault, days } => trash_empty(&vault, days),
        },
    }
}

fn init(vault: &PathBuf, target: Option<PathBuf>, encrypt: bool) -> Result<()> {
    if VaultConfig::load(vault)?.is_some() {
        bail!(
            "{} is already a norm_note vault. Use `normd status` to inspect it.",
            vault.display()
        );
    }

    std::fs::create_dir_all(vault)
        .with_context(|| format!("could not create {}", vault.display()))?;

    let config = VaultConfig::create(target)?;
    let key = norm_core::config::random32()?;

    let passphrase = if encrypt {
        let p1 = rpassword::prompt_password("Enter passphrase to encrypt the key: ")?;
        let p2 = rpassword::prompt_password("Confirm passphrase: ")?;
        if p1 != p2 {
            bail!("Passphrases do not match.");
        }
        Some(p1)
    } else {
        None
    };

    keyring::store(config.vault_id, &key, passphrase.as_deref())?;
    config.save(vault)?;

    println!("Vault ready at {}", vault.display());
    match &config.target {
        Some(t) => println!("Syncing through {}", t.display()),
        None => println!(
            "No sync target set. This vault is local-only; re-run with --target to change that."
        ),
    }
    println!();
    println!("\x1b[1;33m⚠  RECOVERY PHRASE — WRITE THIS DOWN NOW\x1b[0m");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();
    println!("    {}", recovery::encode(&key));
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("This is the \x1b[1mONLY\x1b[0m way to read this vault on another device.");
    println!("If you lose it, your encrypted sync data cannot be recovered.");
    println!("Nobody can reset it for you, because nobody else has it.");
    println!();
    println!(
        "Key stored at: {}",
        keyring::path(config.vault_id)?.display()
    );
    println!("(Outside the vault, so syncing the vault folder does not");
    println!(" upload the key alongside the encrypted data.)");

    Ok(())
}

fn join(vault: &PathBuf, target: PathBuf, recovery: &str, encrypt: bool) -> Result<()> {
    if VaultConfig::load(vault)?.is_some() {
        bail!("{} is already a norm_note vault.", vault.display());
    }

    let key = parse_recovery(recovery)?;

    std::fs::create_dir_all(vault)?;

    // `vault_id` only indexes the local key file; devices do not need to agree
    // on it. What they must share is the key, and that comes from the phrase.
    // A fresh device id is essential though — two machines writing under the
    // same device id would collide on oplog paths, which is the one thing the
    // format is built to make impossible.
    let config = VaultConfig::create(Some(target))?;

    let passphrase = if encrypt {
        let p1 = rpassword::prompt_password("Enter passphrase to encrypt the key: ")?;
        let p2 = rpassword::prompt_password("Confirm passphrase: ")?;
        if p1 != p2 {
            bail!("Passphrases do not match.");
        }
        Some(p1)
    } else {
        None
    };

    keyring::store(config.vault_id, &key, passphrase.as_deref())?;
    config.save(vault)?;

    println!("Joined as a new device ({}).", config.device);
    println!("Run `normd sync` to pull everything down.");
    Ok(())
}

fn open(vault: &PathBuf) -> Result<(VaultConfig, Workspace)> {
    let config = VaultConfig::load(vault)?.ok_or_else(|| {
        anyhow::anyhow!(
            "{} is not a norm_note vault. Run `normd init` there first.",
            vault.display()
        )
    })?;
    let key = keyring::load(config.vault_id)?;
    let ws = Workspace::open(config.device, key, vault)?;
    Ok((config, ws))
}

fn sync_once(vault: &PathBuf) -> Result<()> {
    let (config, mut ws) = open(vault)?;

    let Some(target) = &config.target else {
        // Still worth doing: local edits are folded in and published.
        let scan = ws.pull_from_disk()?;
        ws.push_to_disk()?;
        println!(
            "Local-only vault. {} file(s) folded in.",
            scan.external_edits.len() + scan.new_notes.len()
        );
        return Ok(());
    };

    let store = ChunkStore::new(target);
    let out = ws.cycle(&store)?;
    println!(
        "{} in, {} out, {} file(s) written.",
        out.sync.pulled, out.sync.pushed, out.written
    );
    for id in &out.scan.rescued {
        println!("rescued an older version of {id} into .norm/rescued/");
    }

    let done = ws
        .replica_mut()
        .compact_if_needed(COMPACT_THRESHOLD, Some(&store))?;
    if done.happened() {
        println!("compacted the log, replacing {} chunk(s).", done.pruned);
    }
    Ok(())
}

/// How many of this device's own chunks may pile up before the log is
/// collapsed into a snapshot.
///
/// Measured rather than guessed: 2,000 saves to one note left a 451 KB log
/// that took 3.4 s to read at startup; after compaction the same content was
/// 4 KB and 39 ms. A few hundred is small enough that neither number ever gets
/// uncomfortable, and large enough that ordinary editing does not trigger a
/// snapshot every few minutes.
const COMPACT_THRESHOLD: usize = 200;

fn watch(vault: &PathBuf, interval: u64) -> Result<()> {
    let (config, mut ws) = open(vault)?;
    let target = config.target.as_ref().map(ChunkStore::new);

    // Graceful shutdown: finish the current pass, then exit.
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        eprintln!("\nShutting down after current pass...");
        r.store(false, std::sync::atomic::Ordering::SeqCst);
    })
    .context("could not set Ctrl-C handler")?;

    println!(
        "Watching {} every {interval}s. Ctrl-C to stop.",
        vault.display()
    );
    if target.is_none() {
        println!("No sync target configured; changes are recorded locally only.");
    }

    // Interrupting this is safe at any point: every write is atomic and an
    // interrupted append leaves a gap rather than a reusable sequence number.
    while running.load(std::sync::atomic::Ordering::SeqCst) {
        let result = match &target {
            Some(t) => ws.cycle(t).map(|o| (o.scan, o.sync.pulled, o.written)),
            None => {
                let scan = ws.pull_from_disk()?;
                let written = ws.push_to_disk()?;
                Ok((scan, 0, written))
            }
        };

        match result {
            Ok((scan, pulled, written)) => {
                if !scan.is_empty() || pulled > 0 || written > 0 {
                    println!(
                        "{} local, {} in, {} written",
                        scan.external_edits.len() + scan.new_notes.len(),
                        pulled,
                        written
                    );
                }
                for id in &scan.rescued {
                    println!("rescued an older version of {id} into .norm/rescued/");
                }
                match ws
                    .replica_mut()
                    .compact_if_needed(COMPACT_THRESHOLD, target.as_ref())
                {
                    Ok(done) if done.happened() => {
                        println!("compacted the log, replacing {} chunk(s)", done.pruned)
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("compaction skipped: {e}"),
                }
            }
            // A pass failing is not a reason to stop watching: the target may
            // be a network mount that came and went. Say so and try again.
            Err(e) => eprintln!("pass failed, will retry: {e}"),
        }

        for _ in 0..interval.max(1) {
            if !running.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    println!("Stopped.");
    Ok(())
}

fn doctor(vault: &PathBuf) -> Result<()> {
    let (config, ws) = open(vault)?;
    let key = keyring::load(config.vault_id)?;
    let store = ws.replica().store();

    let mut chunks = 0usize;
    let mut broken = Vec::new();

    for device in store.devices()? {
        for id in store.list(device)? {
            chunks += 1;
            if let Err(e) = store.read(&key, id) {
                broken.push(format!("{} — {e}", id.relative_path()));
            }
        }
    }

    let pending = ws.pending_external_edits()?;

    println!("chunks    {chunks} checked, {} unreadable", broken.len());
    println!("notes     {}", ws.replica().notes().len());
    println!(
        "pending   {} file(s) changed outside the app",
        pending.len()
    );

    for b in &broken {
        println!("  unreadable: {b}");
    }

    if broken.is_empty() {
        println!("\nNo problems found.");
        Ok(())
    } else {
        bail!("{} chunk(s) could not be read", broken.len())
    }
}

fn parse_recovery(s: &str) -> Result<[u8; 32]> {
    recovery::decode(s)
}

fn trash_list(vault: &PathBuf) -> Result<()> {
    let entries = norm_core::trash::list(vault)?;
    if entries.is_empty() {
        println!("Trash is empty.");
        return Ok(());
    }
    println!("{:<20} {:<64} {}", "TIMESTAMP", "HASH", "NOTE ID");
    for entry in entries {
        let ts = entry.deleted_at.parse::<u64>().unwrap_or(0);
        println!("{:<20} {:<64} {}", ts, entry.hash, entry.doc_id);
    }
    Ok(())
}

fn trash_restore(vault: &PathBuf, hash: &str) -> Result<()> {
    if let Some((id, text)) = norm_core::trash::restore(vault, hash)? {
        let (_config, mut ws) = open(vault)?;
        ws.replica_mut().write(&id, &text)?;
        ws.push_to_disk()?;
        println!("Restored {} from trash.", id.as_str());
    } else {
        bail!("Trash entry {} not found.", hash);
    }
    Ok(())
}

fn trash_empty(vault: &PathBuf, days: u32) -> Result<()> {
    let removed = norm_core::trash::purge(vault, days)?;
    println!(
        "Emptied {} notes from trash older than {} days.",
        removed, days
    );
    Ok(())
}
