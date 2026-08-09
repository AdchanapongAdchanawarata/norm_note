//! Soft-delete trash bin for notes.
//!
//! When a note is explicitly deleted through the app, its content is saved
//! here before the CRDT tombstone propagates. This gives the user 30 days
//! to change their mind.
//!
//! Trash is local: it is not synced. Each device keeps its own copy of what
//! it saw at deletion time. The CRDT tombstone is what travels.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::doc::DocId;
use crate::{Result, NORM_DIR};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    pub doc_id: String,
    pub hash: String,
    pub deleted_at: String,
    pub path: PathBuf,
}

fn trash_dir(root: &Path) -> PathBuf {
    root.join(NORM_DIR).join("trash")
}

pub fn save(root: &Path, id: &DocId, content: &str) -> Result<()> {
    let dir = trash_dir(root);
    fs::create_dir_all(&dir)?;

    let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
    let path = dir.join(format!("{}.md", hash));

    // Use an atomic write (write to .tmp, then rename)
    let tmp = {
        let mut s = path.clone().into_os_string();
        s.push(".tmp");
        PathBuf::from(s)
    };

    let mut f = fs::File::create(&tmp)?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    writeln!(f, "<!-- deleted: {} at {} -->", id.as_str(), timestamp)?;
    f.write_all(content.as_bytes())?;
    f.sync_all()?;
    drop(f);

    fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn list(root: &Path) -> Result<Vec<TrashEntry>> {
    let dir = trash_dir(root);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("md") {
            let hash = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            if let Ok(content) = fs::read_to_string(&path) {
                if let Some(line) = content.lines().next() {
                    if line.starts_with("<!-- deleted: ") && line.ends_with(" -->") {
                        let inner = &line["<!-- deleted: ".len()..line.len() - " -->".len()];
                        if let Some(at_idx) = inner.rfind(" at ") {
                            let doc_id = inner[..at_idx].to_string();
                            let deleted_at = inner[at_idx + 4..].to_string();
                            entries.push(TrashEntry {
                                doc_id,
                                hash,
                                deleted_at,
                                path,
                            });
                        }
                    }
                }
            }
        }
    }
    Ok(entries)
}

pub fn restore(root: &Path, hash: &str) -> Result<Option<(DocId, String)>> {
    let dir = trash_dir(root);
    let path = dir.join(format!("{}.md", hash));

    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path)?;
    let mut lines = content.lines();
    if let Some(line) = lines.next() {
        if line.starts_with("<!-- deleted: ") && line.ends_with(" -->") {
            let inner = &line["<!-- deleted: ".len()..line.len() - " -->".len()];
            if let Some(at_idx) = inner.rfind(" at ") {
                let doc_id = inner[..at_idx].to_string();
                let rest = content[line.len()..]
                    .trim_start_matches('\r')
                    .trim_start_matches('\n');
                return Ok(Some((
                    DocId::from_relative_path(Path::new(&doc_id)),
                    rest.to_string(),
                )));
            }
        }
    }

    Ok(None)
}

pub fn purge(root: &Path, max_age_days: u32) -> Result<usize> {
    let entries = list(root)?;
    let mut removed = 0;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let max_age_secs = (max_age_days as u64) * 24 * 60 * 60;

    for entry in entries {
        if let Ok(ts) = entry.deleted_at.parse::<u64>() {
            if now.saturating_sub(ts) >= max_age_secs {
                if fs::remove_file(&entry.path).is_ok() {
                    removed += 1;
                }
            }
        }
    }

    Ok(removed)
}
