//! Per-vault settings, stored in `.norm/config`.
//!
//! Nothing secret lives here. `.norm/` sits inside the vault, and plenty of
//! people keep their whole vault in a synced folder, so anything written here
//! should be assumed to reach that folder. The vault key deliberately does not
//! — see the key handling in `normd`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::oplog::DeviceId;
use crate::{Error, Result, NORM_DIR};

/// Identifies a vault across devices, so a machine can find the right key for
/// it without being told. Not a secret, and not derived from anything about
/// the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultId(pub [u8; 16]);

impl VaultId {
    pub fn to_hex(self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        parse_hex16(s).map(VaultId)
    }
}

#[derive(Debug, Clone)]
pub struct VaultConfig {
    pub vault_id: VaultId,
    pub format_version: u32,
    /// Identity of *this* installation. Two machines sharing a vault must never
    /// share this, or their oplog chunks would collide on the same paths.
    pub device: DeviceId,
    /// Folder to sync through: a Dropbox or iCloud directory, a NAS mount, an
    /// external disk. `None` means this vault is local-only, which is a
    /// perfectly good way to use it.
    pub target: Option<PathBuf>,
}

impl VaultConfig {
    pub fn path(root: &Path) -> PathBuf {
        root.join(NORM_DIR).join("config")
    }

    pub fn create(target: Option<PathBuf>) -> Result<Self> {
        Ok(Self {
            vault_id: VaultId(random16()?),
            format_version: 1,
            device: DeviceId(random16()?),
            target,
        })
    }

    pub fn load(root: &Path) -> Result<Option<Self>> {
        let text = match fs::read_to_string(Self::path(root)) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        let mut vault_id = None;
        let mut format_version = 1;
        let mut device = None;
        let mut target = None;

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim();
            match k.trim() {
                "vault_id" => vault_id = VaultId::from_hex(v),
                "format_version" => format_version = v.parse().unwrap_or(1),
                "device" => device = parse_hex16(v).map(DeviceId),
                "target" if !v.is_empty() => target = Some(PathBuf::from(v)),
                _ => {}
            }
        }

        match (vault_id, device) {
            (Some(vault_id), Some(device)) => Ok(Some(Self {
                vault_id,
                format_version,
                device,
                target,
            })),
            _ => Err(Error::Chunk {
                name: Self::path(root).display().to_string(),
                reason: "config is missing vault_id or device".to_owned(),
            }),
        }
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let path = Self::path(root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut body = String::new();
        body.push_str("# norm_note vault config. No secrets belong in this file.\n");
        body.push_str(&format!("vault_id = {}\n", self.vault_id.to_hex()));
        body.push_str(&format!("format_version = {}\n", self.format_version));
        body.push_str(&format!("device = {}\n", self.device.to_hex()));
        if let Some(t) = &self.target {
            body.push_str(&format!("target = {}\n", t.display()));
        }

        let tmp = {
            let mut s = path.clone().into_os_string();
            s.push(".tmp");
            PathBuf::from(s)
        };
        let mut f = fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, &path)?;
        Ok(())
    }
}

pub fn random16() -> Result<[u8; 16]> {
    let mut out = [0u8; 16];
    getrandom::fill(&mut out).map_err(|e| Error::Crdt(format!("no system randomness: {e}")))?;
    Ok(out)
}

pub fn random32() -> Result<[u8; 32]> {
    let mut out = [0u8; 32];
    getrandom::fill(&mut out).map_err(|e| Error::Crdt(format!("no system randomness: {e}")))?;
    Ok(out)
}

fn parse_hex16(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = VaultConfig::create(Some(PathBuf::from("D:/Dropbox/norm"))).unwrap();
        cfg.save(dir.path()).unwrap();

        let back = VaultConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(back.vault_id, cfg.vault_id);
        assert_eq!(back.device, cfg.device);
        assert_eq!(back.target, cfg.target);
    }

    #[test]
    fn a_local_only_vault_needs_no_target() {
        let dir = tempfile::tempdir().unwrap();
        VaultConfig::create(None).unwrap().save(dir.path()).unwrap();
        assert_eq!(VaultConfig::load(dir.path()).unwrap().unwrap().target, None);
    }

    #[test]
    fn missing_config_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(VaultConfig::load(dir.path()).unwrap().is_none());
    }

    #[test]
    fn a_truncated_config_is_reported_not_guessed() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(NORM_DIR)).unwrap();
        fs::write(VaultConfig::path(dir.path()), "vault_id = abc\n").unwrap();
        assert!(VaultConfig::load(dir.path()).is_err());
    }

    #[test]
    fn two_vaults_never_share_an_id() {
        let a = VaultConfig::create(None).unwrap();
        let b = VaultConfig::create(None).unwrap();
        assert_ne!(a.vault_id, b.vault_id);
        assert_ne!(a.device, b.device);
    }
}
