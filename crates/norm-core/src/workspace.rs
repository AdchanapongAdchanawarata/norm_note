//! Ties the user's folder of Markdown files to the CRDT and the oplog.
//!
//! # The ordering that matters
//!
//! A cycle is always: read the disk, sync, write the disk.
//!
//! ```text
//! pull_from_disk()   local typing becomes CRDT changes
//! sync_through()     chunks out, chunks in, remote changes applied
//! push_to_disk()     the merged result is written back
//! ```
//!
//! Getting this backwards destroys data. If a remote change has been absorbed
//! but not yet written to disk, and we then read the stale file and treat it as
//! the truth, `set_text` deletes the remote edit. The rule that prevents it:
//! **never treat the disk as authoritative for a note whose current CRDT state
//! has not been written there.**
//!
//! # Telling a user's edit from our own write
//!
//! Both look identical on disk. The difference is whether we put it there, so
//! every write records the hash of what was written, and that record is
//! persisted. Without it, a restart cannot tell "the user edited this in vim
//! while we were closed" from "we never got round to writing this out", and
//! those two need opposite responses.
//!
//! # Deletion (v0.2)
//!
//! A file that vanishes from disk while we have a record of writing it is
//! treated as an intentional deletion. The note's content is saved to
//! `.norm/trash/` and its CRDT content is set to the empty string, which
//! propagates as a tombstone to all devices.
//!
//! A file that was never written by us (no entry in `materialized`) is still
//! restored from the CRDT — it may be arriving from another device for the
//! first time.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::doc::DocId;
use crate::oplog::{store::ChunkStore, DeviceId, VaultKey};
use crate::replica::{Replica, SyncOutcome};
use crate::vault::Vault;
use crate::{Error, Result, NORM_DIR};

/// What we last saw of a file, so an unchanged one can be skipped without
/// reading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub hash: String,
    /// Nanoseconds since the Unix epoch, or 0 when unknown.
    pub mtime: u128,
    pub size: u64,
}

pub struct Workspace {
    vault: Vault,
    replica: Replica,
    /// What we last wrote to, or read from, each file. Persisted.
    materialized: HashMap<DocId, Seen>,
}

/// How recent an mtime has to be before it stops being trusted.
///
/// A file changed in the same tick as our scan can end up with the mtime we
/// already recorded, and if its length happens to match too, the edit would be
/// invisible — permanently, because nothing would ever look at that file again.
///
/// The window has to cover the filesystem's mtime resolution. NTFS and APFS
/// are sub-microsecond, but HFS+ stores whole seconds, so one second is the
/// figure to beat. Anything inside the window is read properly; the cost is
/// that a file just touched is examined once more, which is exactly when we
/// should be looking at it anyway.
const RACY_WINDOW_NANOS: u128 = 1_000_000_000;

/// What one pass over the vault found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiskScan {
    /// Notes the user edited outside the app, now folded in.
    pub external_edits: Vec<DocId>,
    /// Notes seen for the first time.
    pub new_notes: Vec<DocId>,
    /// Notes where the disk and the CRDT both had content and no record said
    /// which came first. The CRDT version was copied to `.norm/rescued/` and
    /// the file on disk was kept.
    pub rescued: Vec<DocId>,
    /// Notes deleted by the user (file removed from disk). NEW.
    pub deleted: Vec<DocId>,
}

impl DiskScan {
    pub fn is_empty(&self) -> bool {
        self.external_edits.is_empty() && self.new_notes.is_empty() && self.rescued.is_empty() && self.deleted.is_empty()
    }
}

impl Workspace {
    pub fn open(device: DeviceId, key: VaultKey, root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let mut ws = Self {
            vault: Vault::new(root.clone()),
            replica: Replica::open(device, key, root),
            materialized: HashMap::new(),
        };
        ws.materialized = ws.load_state()?;
        ws.replica.absorb()?;
        // After absorbing, so the CRDT can supply replacements for anything a
        // crash left torn.
        ws.recover_inflight()?;
        Ok(ws)
    }

    /// Repairs note files that a crash may have left half-written.
    ///
    /// [`Self::push_to_disk`] records what it is about to write before writing
    /// it. If that record survives, the batch did not finish, and any file in
    /// it that does not match what we meant to write is damage rather than a
    /// user's edit. Rewriting from the CRDT is always right here: the CRDT is
    /// what the file was going to be made from in the first place.
    ///
    /// Returns the notes it had to repair.
    fn recover_inflight(&mut self) -> Result<Vec<DocId>> {
        let Some(pending) = read_pairs(&self.inflight_path())? else {
            return Ok(Vec::new());
        };

        let mut repaired = Vec::new();
        for (id, intended) in pending {
            let Some(text) = self.replica.text(&id)? else {
                continue;
            };
            let on_disk = self.vault.read(&id)?;
            if let Some(landed) = &on_disk {
                if hash_of(landed) == intended.hash {
                    // It landed. Record it so it is not mistaken for an edit.
                    self.record(id, landed);
                    continue;
                }
            }
            self.vault.write(&id, &text)?;
            self.record(id.clone(), &text);
            repaired.push(id);
        }

        self.save_state()?;
        let _ = fs::remove_file(self.inflight_path());
        Ok(repaired)
    }

    pub fn replica(&self) -> &Replica {
        &self.replica
    }

    /// Direct access for measurement and for callers driving the stages
    /// themselves. Ordinary use should go through [`Self::cycle`], which gets
    /// the disk/sync/disk ordering right.
    pub fn replica_mut(&mut self) -> &mut Replica {
        &mut self.replica
    }

    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    /// Read the disk, sync, write the disk. The whole loop.
    pub fn cycle(&mut self, target: &ChunkStore) -> Result<CycleOutcome> {
        let scan = self.pull_from_disk()?;
        let sync = self.replica.sync_through(target)?;
        let written = self.push_to_disk()?;
        Ok(CycleOutcome {
            scan,
            sync,
            written,
        })
    }

    /// Files that differ from what we last wrote, without changing anything.
    ///
    /// This is what `status` reports. It must never mutate: a user running
    /// `status` to find out what is going on should not cause a sync.
    pub fn pending_external_edits(&self) -> Result<Vec<DocId>> {
        let now = now_nanos();
        let mut pending = Vec::new();
        for id in self.vault.scan()? {
            if self.looks_unchanged(&id, now) {
                continue;
            }
            let Some(text) = self.vault.read(&id)? else {
                continue;
            };
            if self.materialized.get(&id).map(|s| s.hash.as_str()) != Some(hash_of(&text).as_str())
            {
                pending.push(id);
            }
        }
        Ok(pending)
    }

    /// True when a file's size and modification time are exactly what we
    /// recorded, and the timestamp is old enough to be believed.
    ///
    /// A hint, never a decision: saying "changed" when it has not costs one
    /// read, and the racy-window rule is there so it can never say "unchanged"
    /// about a file that was.
    fn looks_unchanged(&self, id: &DocId, now: u128) -> bool {
        let Some(seen) = self.materialized.get(id) else {
            return false;
        };
        if seen.mtime == 0 {
            return false;
        }
        let Ok(meta) = fs::metadata(self.vault.path_of(id)) else {
            return false;
        };
        let Some(mtime) = mtime_nanos(&meta) else {
            return false;
        };
        mtime == seen.mtime
            && meta.len() == seen.size
            && now.saturating_sub(mtime) > RACY_WINDOW_NANOS
    }

    fn record(&mut self, id: DocId, text: &str) {
        let meta = fs::metadata(self.vault.path_of(&id)).ok();
        let seen = Seen {
            hash: hash_of(text),
            mtime: meta.as_ref().and_then(mtime_nanos).unwrap_or(0),
            size: meta.map(|m| m.len()).unwrap_or(text.len() as u64),
        };
        self.materialized.insert(id, seen);
    }

    /// Folds edits made outside the app into the CRDT and publishes them.
    pub fn pull_from_disk(&mut self) -> Result<DiskScan> {
        let mut found = DiskScan::default();
        let mut edits: Vec<(DocId, String)> = Vec::new();

        let now = now_nanos();
        let on_disk = self.vault.scan()?;

        // --- DELETION HANDLING (v0.2) ---
        // A note whose file is gone and whose last state we recorded was us
        // writing it means the user deleted it outside the app. That is
        // different from "the file was never here" (partial sync) or
        // "we never wrote it" (first scan).
        let present: std::collections::HashSet<&DocId> = on_disk.iter().collect();
        for id in self.replica.notes() {
            if !present.contains(&id) {
                if self.materialized.contains_key(&id) {
                    // We wrote this file before and now it is gone.
                    // The user deleted it outside the app — treat as deletion.
                    if !self.replica.is_deleted(&id)? {
                        // Save to trash before tombstoning
                        if let Some(text) = self.replica.text(&id)? {
                            if !text.is_empty() {
                                crate::trash::save(self.vault.root(), &id, &text)?;
                            }
                        }
                        self.replica.delete(&id)?;
                        self.materialized.remove(&id);
                        tracing::info!("deleted {id} (file removed from disk)");
                        found.deleted.push(id.clone());
                    }
                } else {
                    // Never wrote this file — it hasn't arrived yet or was
                    // never on this device. Restore it from the CRDT.
                    if !self.replica.is_deleted(&id)? {
                        self.replica.mark_dirty(&id);
                    }
                }
            }
        }

        for id in on_disk {
            // Skips reading the file at all when its size and timestamp say
            // nothing has happened to it.
            if self.looks_unchanged(&id, now) {
                continue;
            }

            let Some(disk_text) = self.vault.read(&id)? else {
                continue;
            };
            let disk_hash = hash_of(&disk_text);

            match self.materialized.get(&id) {
                // Exactly what we last put there. Nothing happened.
                Some(known) if known.hash == disk_hash => {
                    // Refresh the timestamp so the fast path can take over
                    // next time round.
                    self.record(id, &disk_text);
                    continue;
                }

                // We have written this file before, so the CRDT state is
                // already reflected on disk and the difference is the user's.
                Some(_) => found.external_edits.push(id.clone()),

                None => {
                    let in_crdt = self
                        .replica
                        .text(&id)?
                        .filter(|t| !t.is_empty())
                        .filter(|t| *t != disk_text);

                    match in_crdt {
                        // Never seen: a note the user just created, or the
                        // first scan of an existing vault.
                        None => found.new_notes.push(id.clone()),

                        // Both sides have content and nothing records which we
                        // put there. Keep the user's file — it is the one they
                        // can see — but never discard the other version.
                        Some(crdt_text) => {
                            self.rescue(&id, &crdt_text)?;
                            found.rescued.push(id.clone());
                        }
                    }
                }
            }

            edits.push((id, disk_text));
        }

        if !edits.is_empty() {
            let refs: Vec<(DocId, &str)> =
                edits.iter().map(|(i, t)| (i.clone(), t.as_str())).collect();
            self.replica.write_many(&refs)?;
            for (id, text) in &edits {
                self.record(id.clone(), text);
            }
            self.save_state()?;
        }

        Ok(found)
    }

    /// Writes the current CRDT state of every note back to disk.
    ///
    /// A note whose file has been edited since we last wrote it is skipped:
    /// that edit has not been folded in yet, and overwriting would throw it
    /// away. The next [`Self::pull_from_disk`] picks it up.
    pub fn push_to_disk(&mut self) -> Result<usize> {
        // Decide everything first, then journal it, then write. The individual
        // writes are not flushed — see `Vault::write` — so the journal is what
        // makes a crash recoverable, and it has to be on disk beforehand to be
        // worth anything.
        let now = now_nanos();
        let mut planned: Vec<(DocId, String, String)> = Vec::new();

        // Only notes whose CRDT state has actually moved. Walking every note
        // instead means thousands of filesystem calls per pass on a vault that
        // has been in use for a few years, to find the one that changed.
        for id in self.replica.dirty_notes() {
            // Don't write tombstoned notes to disk.
            if self.replica.is_deleted(&id)? {
                // If the file still exists, remove it.
                let file_path = self.vault.path_of(&id);
                if file_path.exists() {
                    let _ = std::fs::remove_file(&file_path);
                    self.materialized.remove(&id);
                }
                continue;
            }
            let Some(text) = self.replica.text(&id)? else {
                continue;
            };
            let wanted = hash_of(&text);

            // The common case in a large vault: the file is already what the
            // CRDT says and nothing has touched it, so it needs no read.
            if let Some(seen) = self.materialized.get(&id) {
                if seen.hash == wanted && self.looks_unchanged(&id, now) {
                    continue;
                }
            }

            let on_disk = self.vault.read(&id)?;
            if let Some(current) = &on_disk {
                let current_hash = hash_of(current);
                if current_hash == wanted {
                    self.record(id, current);
                    continue;
                }
                if self.materialized.get(&id).map(|s| s.hash.as_str())
                    != Some(current_hash.as_str())
                {
                    // Somebody changed the file behind us. Leave it alone.
                    continue;
                }
            }

            planned.push((id, text, wanted));
        }

        // Everything considered has now either been dealt with or deliberately
        // skipped, so the list starts again from here.
        self.replica.clear_dirty();

        if planned.is_empty() {
            return Ok(0);
        }

        // The journal records only the intended hash; timestamp and size are
        // not known until the file exists, and recovery does not need them.
        let journal: Vec<(DocId, Seen)> = planned
            .iter()
            .map(|(id, _, hash)| {
                (
                    id.clone(),
                    Seen {
                        hash: hash.clone(),
                        mtime: 0,
                        size: 0,
                    },
                )
            })
            .collect();
        write_pairs(&self.inflight_path(), &journal)?;

        for (id, text, _) in planned {
            self.vault.write(&id, &text)?;
            self.record(id, &text);
        }

        self.save_state()?;
        let _ = fs::remove_file(self.inflight_path());
        Ok(journal.len())
    }

    /// Keeps a copy of a version that would otherwise have been overwritten.
    fn rescue(&self, id: &DocId, text: &str) -> Result<()> {
        let dir = self.vault.root().join(NORM_DIR).join("rescued");
        fs::create_dir_all(&dir)?;
        let name = format!("{}.md", hash_of(text));
        let mut f = fs::File::create(dir.join(name))?;
        writeln!(f, "<!-- rescued copy of {id} -->")?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        Ok(())
    }

    fn state_dir(&self) -> PathBuf {
        self.vault.root().join(NORM_DIR).join("state")
    }

    fn state_path(&self) -> PathBuf {
        self.state_dir().join("materialized")
    }

    /// Notes a write batch is part-way through. Present only after a crash.
    fn inflight_path(&self) -> PathBuf {
        self.state_dir().join("inflight")
    }

    /// Loads what we know about the files on disk.
    ///
    /// An unreadable state file is survivable and is therefore survived: with
    /// nothing recorded, every file looks unfamiliar, and the rescue path in
    /// [`Self::pull_from_disk`] keeps both versions of anything ambiguous.
    /// Refusing to open the vault would be the worse answer.
    fn load_state(&self) -> Result<HashMap<DocId, Seen>> {
        match read_pairs(&self.state_path()) {
            Ok(entries) => Ok(entries.unwrap_or_default().into_iter().collect()),
            Err(e) => {
                tracing::warn!(
                    "{} could not be read ({e}); treating every file as unfamiliar",
                    self.state_path().display()
                );
                Ok(HashMap::new())
            }
        }
    }

    fn save_state(&self) -> Result<()> {
        let mut entries: Vec<(DocId, Seen)> = self
            .materialized
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        write_pairs(&self.state_path(), &entries)
    }
}

/// Reads `<hash> <mtime> <size> <path>` lines. `None` when the file is absent,
/// which is not an error — a vault that has never been written has no state.
fn read_pairs(path: &Path) -> Result<Option<Vec<(DocId, Seen)>>> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    let mut out = Vec::new();
    for line in text.lines() {
        // Three fixed fields, then the path — which is the whole rest of the
        // line, because note names contain spaces.
        let mut parts = line.splitn(4, ' ');
        let (Some(hash), Some(mtime), Some(size), Some(rest)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            // Loud, not skipped. Which file this is decides how much that
            // matters: an unreadable `materialized` costs a rescan, but an
            // unreadable `inflight` would mean a torn file goes unrepaired and
            // gets published as though the user had typed it. The caller
            // decides which of those it can live with.
            return Err(Error::Chunk {
                name: path.display().to_string(),
                reason: format!("malformed line: {line:?}"),
            });
        };
        // Every field is checked. Accepting whatever happens to have four
        // space-separated pieces is how a file full of unrelated text passes
        // for a journal — and a journal that "parses" but means nothing is
        // worse than one that fails, because it silently skips the repair.
        let bad = |what: &str| Error::Chunk {
            name: path.display().to_string(),
            reason: format!("{what} in line: {line:?}"),
        };

        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad("first field is not a 64-character hash"));
        }
        let mtime = mtime.parse::<u128>().map_err(|_| bad("bad timestamp"))?;
        let size = size.parse::<u64>().map_err(|_| bad("bad size"))?;

        out.push((
            DocId::from_relative_path(Path::new(rest)),
            Seen {
                hash: hash.to_owned(),
                mtime,
                size,
            },
        ));
    }
    Ok(Some(out))
}

/// Writes the state file durably. This one *is* flushed: it is the record that
/// makes the unflushed note writes recoverable.
fn write_pairs(path: &Path, entries: &[(DocId, Seen)]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut body = String::new();
    for (id, seen) in entries {
        if id.as_str().contains('\n') {
            return Err(Error::Frontmatter {
                path: id.to_string(),
                reason: "note paths containing a newline are not supported".to_owned(),
            });
        }
        body.push_str(&format!(
            "{} {} {} {}\n",
            seen.hash,
            seen.mtime,
            seen.size,
            id.as_str()
        ));
    }

    let tmp = {
        let mut s = path.to_path_buf().into_os_string();
        s.push(".tmp");
        PathBuf::from(s)
    };
    let mut f = fs::File::create(&tmp)?;
    f.write_all(body.as_bytes())?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, path)?;
    Ok(())
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn mtime_nanos(meta: &fs::Metadata) -> Option<u128> {
    meta.modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos())
}

fn hash_of(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleOutcome {
    pub scan: DiskScan,
    pub sync: SyncOutcome,
    pub written: usize,
}

impl CycleOutcome {
    pub fn is_idle(&self) -> bool {
        self.scan.is_empty() && self.sync.is_idle() && self.written == 0
    }
}
