//! `normd status`.
//!
//! # Nothing here rounds up
//!
//! A green tick that means "probably fine" is how sync tools lose people's
//! trust: a user who saw a tick and then lost a note does not come back. What
//! this prints is only what was actually observed — how many chunks each device
//! has produced, how many have not reached the target yet, and how many files
//! changed behind our back.
//!
//! "Not synced yet" is a normal state, said plainly, and it is a far better
//! thing to show than a reassuring symbol that might be wrong.

use std::path::PathBuf;

use anyhow::Result;
use norm_core::config::VaultConfig;
use norm_core::oplog::store::ChunkStore;
use norm_core::workspace::Workspace;

pub fn show(vault: &PathBuf) -> Result<()> {
    let config = VaultConfig::load(vault)?.ok_or_else(|| {
        anyhow::anyhow!(
            "{} is not a norm_note vault. Run `normd init` there first.",
            vault.display()
        )
    })?;

    let key = crate::keyring::load(config.vault_id)?;
    let ws = Workspace::open(config.device, key, vault)?;
    let local = ws.replica().store();

    println!("vault    {}", vault.display());
    println!("device   {}  (this machine)", config.device);
    println!("notes    {}", ws.replica().notes().len());
    println!();

    println!("devices");
    let devices = local.devices()?;
    if devices.is_empty() {
        println!("  nothing written yet");
    }
    for d in &devices {
        let count = local.list(*d)?.len();
        let who = if *d == config.device {
            "this machine"
        } else {
            "another device"
        };
        println!("  {d}  {count:>6} chunks  {who}");
    }
    println!();

    match &config.target {
        None => {
            println!("target   none — this vault is local-only");
        }
        Some(path) => {
            println!("target   {}", path.display());
            if !path.exists() {
                // Being unreachable is not an error. A NAS is not always
                // mounted and a cloud folder is not always present.
                println!("         not reachable right now; nothing has been lost");
            } else {
                let remote = ChunkStore::new(path);
                let (mut to_push, mut to_pull) = (0usize, 0usize);

                for d in local.devices()? {
                    for id in local.list(d)? {
                        if !remote.has(id) {
                            to_push += 1;
                        }
                    }
                }
                for d in remote.devices()? {
                    for id in remote.list(d)? {
                        if !local.has(id) {
                            to_pull += 1;
                        }
                    }
                }

                match (to_push, to_pull) {
                    (0, 0) => println!("         everything this machine knows about is there"),
                    _ => println!("         {to_push} chunk(s) to push, {to_pull} to pull"),
                }
            }
        }
    }
    println!();

    let pending = ws.pending_external_edits()?;
    if pending.is_empty() {
        println!("files    no changes outside the app");
    } else {
        println!(
            "files    {} changed outside the app, not folded in yet",
            pending.len()
        );
        for id in pending.iter().take(10) {
            println!("           {id}");
        }
        if pending.len() > 10 {
            println!("           ... and {} more", pending.len() - 10);
        }
        println!("         run `normd sync` to fold them in");
    }

    Ok(())
}
