//! Where the vault key lives.
//!
//! # Not in the vault
//!
//! `.norm/` sits inside the vault folder, and a great many people keep that
//! whole folder in Dropbox or iCloud Drive. A key stored there would be
//! uploaded next to the ciphertext it protects, which is the same as having no
//! encryption at all — the sync target is exactly the party the encryption
//! exists to keep out.
//!
//! So the key goes in the OS configuration directory, which is not a place
//! people sync, indexed by vault id.
//!
//! # Still plaintext on disk (by default)
//!
//! This is a real limitation and it is stated rather than hidden. Anyone who
//! can read your user profile can read the key. Using the platform keychain
//! (Keychain on macOS, DPAPI on Windows, Secret Service on Linux) belongs with
//! the GUI applications, where there is a user session to unlock it. A daemon
//! that must start unattended has no good way to reach those stores.
//!
//! # Optional Passphrase Encryption (v0.2)
//!
//! The user can choose to encrypt the local key file with a passphrase using
//! Argon2 and ChaCha20Poly1305. If enabled, the daemon will prompt for the
//! passphrase interactively on startup.
//!
//! The threat this does defend against is the one the design is actually about:
//! the contents of your notes reaching a cloud provider.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use norm_core::config::VaultId;
use norm_core::oplog::VaultKey;

use argon2::Argon2;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand_core::{OsRng, RngCore};

/// Per-user configuration directory, resolved without a dependency so the
/// behaviour is auditable in one screen.
fn config_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            if !appdata.is_empty() {
                return Ok(PathBuf::from(appdata).join("norm_note"));
            }
        }
        bail!("APPDATA is not set, so there is nowhere to keep the key")
    }

    #[cfg(not(windows))]
    {
        if cfg!(target_os = "macos") {
            if let Ok(home) = std::env::var("HOME") {
                if !home.is_empty() {
                    return Ok(PathBuf::from(home)
                        .join("Library")
                        .join("Application Support")
                        .join("norm_note"));
                }
            }
        }
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            if !xdg.is_empty() {
                return Ok(PathBuf::from(xdg).join("norm_note"));
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                return Ok(PathBuf::from(home).join(".config").join("norm_note"));
            }
        }
        bail!("neither XDG_CONFIG_HOME nor HOME is set, so there is nowhere to keep the key")
    }
}

pub fn path(vault: VaultId) -> Result<PathBuf> {
    Ok(config_dir()?
        .join("keys")
        .join(format!("{}.key", vault.to_hex())))
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(s: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 2);
    for i in 0..(s.len() / 2) {
        let b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .with_context(|| "Invalid hex sequence")?;
        out.push(b);
    }
    Ok(out)
}

pub fn store(vault: VaultId, key: &[u8; 32], passphrase: Option<&str>) -> Result<()> {
    let path = path(vault)?;
    let dir = path.parent().expect("key path always has a parent");
    fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;

    let mut file =
        fs::File::create(&path).with_context(|| format!("could not write {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }

    if let Some(pass) = passphrase {
        let mut salt = [0u8; 16];
        OsRng.fill_bytes(&mut salt);

        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);

        let argon2 = Argon2::default();
        let mut kek = [0u8; 32];
        argon2
            .hash_password_into(pass.as_bytes(), &salt, &mut kek)
            .map_err(|_| anyhow::anyhow!("Failed to derive key"))?;

        let cipher = ChaCha20Poly1305::new(Key::from_slice(&kek));
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, key.as_ref())
            .map_err(|_| anyhow::anyhow!("Encryption failed"))?;

        writeln!(
            file,
            "ENCRYPTED {} {} {}",
            to_hex(&salt),
            to_hex(&nonce_bytes),
            to_hex(&ciphertext)
        )?;
    } else {
        file.write_all(to_hex(key).as_bytes())?;
        file.write_all(b"\n")?;
    }

    file.sync_all()?;
    Ok(())
}

pub fn load(vault: VaultId) -> Result<VaultKey> {
    let path = path(vault)?;
    let text = fs::read_to_string(&path).with_context(|| {
        format!(
            "no key for this vault at {}. If the vault came from another \
             device, run `normd join --recovery <phrase>`.",
            path.display()
        )
    })?;

    if text.starts_with("ENCRYPTED ") {
        let parts: Vec<&str> = text.trim().split(' ').collect();
        if parts.len() != 4 {
            bail!("Malformed encrypted key file at {}", path.display());
        }

        let salt = from_hex(parts[1])?;
        let nonce_bytes = from_hex(parts[2])?;
        let ciphertext = from_hex(parts[3])?;

        let pass = rpassword::prompt_password("Enter passphrase to unlock the vault key: ")?;

        let argon2 = Argon2::default();
        let mut kek = [0u8; 32];
        argon2
            .hash_password_into(pass.as_bytes(), &salt, &mut kek)
            .map_err(|_| anyhow::anyhow!("Failed to derive key"))?;

        let cipher = ChaCha20Poly1305::new(Key::from_slice(&kek));
        let nonce = Nonce::from_slice(&nonce_bytes);

        let decrypted = cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|_| anyhow::anyhow!("Incorrect passphrase or corrupted key file"))?;

        if decrypted.len() != 32 {
            bail!("Decrypted key is not 32 bytes");
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&decrypted);
        return Ok(VaultKey::new(out));
    }

    let cleaned: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() != 64 {
        bail!("{} does not contain a 32-byte key", path.display());
    }

    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&cleaned[i * 2..i * 2 + 2], 16)
            .with_context(|| format!("{} is not valid hex", path.display()))?;
    }
    Ok(VaultKey::new(out))
}
