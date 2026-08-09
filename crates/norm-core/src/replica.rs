//! One device's view of a vault: notes, their CRDT state, and the oplog that
//! carries edits between machines.
//!
//! This is where the pieces meet. An edit becomes a set of Automerge changes,
//! the changes become an encrypted oplog chunk, the chunk is copied to whatever
//! folder the user picked, and another device folds it back into its own notes.
//!
//! Nothing here coordinates with anything. Devices never negotiate, never take
//! a lock, and never need to be online at the same time. Two replicas that have
//! seen the same set of chunks hold the same notes, regardless of the order the
//! chunks arrived in or how long they took.

use std::collections::{HashMap, HashSet};

use automerge::ChangeHash;

use crate::doc::{DocId, NoteDoc};
use crate::oplog::payload::{Batch, Entry};
use crate::oplog::{store::ChunkStore, ChunkId, ChunkKind, DeviceId, VaultKey};
use crate::Result;

pub struct Replica {
    device: DeviceId,
    key: VaultKey,
    store: ChunkStore,
    docs: HashMap<DocId, NoteDoc>,
    /// Heads already represented in the oplog, so an edit publishes only what
    /// is new rather than the note's whole history.
    published: HashMap<DocId, Vec<ChangeHash>>,
    /// Chunks already folded into `docs`.
    ///
    /// In memory only. A restart re-reads the log from the beginning, which is
    /// wasteful but not wrong: applying a change twice is a no-op. Persisting
    /// this is an optimisation, not a correctness fix.
    applied: HashSet<ChunkId>,
    /// Notes whose CRDT state has moved since they were last written out.
    ///
    /// Without this, writing the vault back means considering every note on
    /// every pass. At a few hundred notes that is invisible; at several
    /// thousand — which is what a few years of daily note-taking produces — it
    /// is thousands of filesystem calls every few seconds to discover that one
    /// note changed.
    dirty: HashSet<DocId>,
}

impl Replica {
    pub fn open(device: DeviceId, key: VaultKey, root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            device,
            key,
            store: ChunkStore::new(root),
            docs: HashMap::new(),
            published: HashMap::new(),
            applied: HashSet::new(),
            dirty: HashSet::new(),
        }
    }

    /// Notes that have changed since [`Self::clear_dirty`] was last called.
    pub fn dirty_notes(&self) -> Vec<DocId> {
        let mut ids: Vec<_> = self.dirty.iter().cloned().collect();
        ids.sort();
        ids
    }

    /// Forces a note to be reconsidered even though its content has not moved
    /// — used when its file has gone missing from disk.
    pub fn mark_dirty(&mut self, id: &DocId) {
        self.dirty.insert(id.clone());
    }

    pub fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    pub fn device(&self) -> DeviceId {
        self.device
    }

    pub fn store(&self) -> &ChunkStore {
        &self.store
    }

    /// Current text of a note, or `None` if this replica has never seen it.
    pub fn text(&self, id: &DocId) -> Result<Option<String>> {
        self.docs.get(id).map(|d| d.text()).transpose()
    }

    pub fn notes(&self) -> Vec<DocId> {
        let mut ids: Vec<_> = self.docs.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Marks a note as deleted by setting its content to the empty string.
    ///
    /// The empty string is the tombstone convention: it propagates through
    /// the CRDT like any other edit, so every device eventually sees it.
    /// This is backward compatible — a v0.1 client will see an empty note
    /// rather than crash.
    pub fn delete(&mut self, id: &DocId) -> Result<Option<ChunkId>> {
        self.write(id, "")
    }

    /// True when a note exists in the CRDT but has been tombstoned.
    pub fn is_deleted(&self, id: &DocId) -> Result<bool> {
        match self.text(id)? {
            Some(t) => Ok(t.is_empty()),
            None => Ok(false),
        }
    }

    /// All notes that are not tombstoned.
    pub fn live_notes(&self) -> Result<Vec<DocId>> {
        let mut live = Vec::new();
        for id in self.notes() {
            if !self.is_deleted(&id)? {
                live.push(id);
            }
        }
        Ok(live)
    }

    /// Records an edit and publishes it to the oplog.
    ///
    /// Writing the same text again is free: `update_text` produces no change,
    /// so no chunk is appended. Editors that save on a timer therefore do not
    /// fill the log with noise.
    pub fn write(&mut self, id: &DocId, text: &str) -> Result<Option<ChunkId>> {
        self.write_many(&[(id.clone(), text)])
    }

    /// Records several edits and publishes them as a **single** chunk.
    ///
    /// This is what makes importing an existing vault practical. One chunk per
    /// note would mean one durable write per note — thousands of fsyncs for a
    /// real Obsidian vault — where this is one.
    pub fn write_many(&mut self, edits: &[(DocId, &str)]) -> Result<Option<ChunkId>> {
        let mut batch = Batch::default();
        let mut new_heads = Vec::new();

        for (id, text) in edits {
            let doc = self.doc_mut(id)?;
            doc.set_text(text)?;

            let since = self.published.get(id).cloned().unwrap_or_default();
            let doc = self.doc_mut(id)?;
            let changes = doc.changes_since(&since);
            if changes.is_empty() {
                continue;
            }
            new_heads.push((id.clone(), doc.heads()));
            self.dirty.insert(id.clone());
            batch.entries.push(Entry {
                doc: id.as_str().to_owned(),
                changes,
            });
        }

        if batch.is_empty() {
            return Ok(None);
        }

        let chunk = self
            .store
            .append(&self.key, self.device, ChunkKind::Op, &batch.encode())?;

        // Only after the chunk is durably on disk. If the append had failed,
        // treating these heads as published would drop the edits from the next
        // delta and lose them silently.
        for (id, heads) in new_heads {
            self.published.insert(id, heads);
        }
        Ok(Some(chunk))
    }

    /// Folds every chunk in the local store that has not been applied yet into
    /// the in-memory notes.
    ///
    /// Safe to call at any time and any number of times.
    pub fn absorb(&mut self) -> Result<usize> {
        let mut applied_now = 0;

        for device in self.store.devices()? {
            for chunk in self.store.list(device)? {
                if self.applied.contains(&chunk) {
                    continue;
                }

                let bytes = self.store.read(&self.key, chunk)?;
                let batch = Batch::decode(&bytes)?;

                for entry in batch.entries {
                    let id = DocId::from_relative_path(std::path::Path::new(&entry.doc));
                    let doc = self.doc_mut(&id)?;
                    match chunk.kind {
                        ChunkKind::Op => doc.apply_changes(&entry.changes)?,
                        // A snapshot carries one whole saved document per note
                        // instead of a list of deltas.
                        ChunkKind::Snapshot => {
                            for saved in &entry.changes {
                                doc.merge_saved(saved)?;
                            }
                        }
                    }
                    let heads = doc.heads();
                    self.published.insert(id.clone(), heads);
                    self.dirty.insert(id);
                }

                self.applied.insert(chunk);
                applied_now += 1;
            }
        }
        self.save_applied()?;

        Ok(applied_now)
    }

    /// Writes a snapshot: this device's complete state for every note it knows.
    ///
    /// # Why the log needs this
    ///
    /// Every save appends. Two thousand saves to one note produced an 18 KB
    /// file and a 449 KB log — and nothing ever removed any of it, so both the
    /// disk cost and the time to read the log back grow without limit. A
    /// snapshot says the same thing in one chunk, and lets the chunks before it
    /// be deleted.
    ///
    /// Snapshots do not change what any device sees, only how much it has to
    /// read to see it. Never taking one is a size problem; it is never a
    /// correctness problem.
    pub fn snapshot(&mut self) -> Result<Option<ChunkId>> {
        let ids = self.notes();
        if ids.is_empty() {
            return Ok(None);
        }

        let mut batch = Batch::default();
        let mut heads = Vec::new();
        for id in ids {
            let doc = self.doc_mut(&id)?;
            let saved = doc.save();
            heads.push((id.clone(), doc.heads()));
            batch.entries.push(Entry {
                doc: id.as_str().to_owned(),
                changes: vec![saved],
            });
        }

        let chunk =
            self.store
                .append(&self.key, self.device, ChunkKind::Snapshot, &batch.encode())?;

        for (id, h) in heads {
            self.published.insert(id, h);
        }
        // The snapshot is one of our own chunks and absorbing it again would be
        // wasted work; it already reflects exactly what we hold.
        self.applied.insert(chunk);
        Ok(Some(chunk))
    }

    /// How many of this device's own chunks a snapshot would replace.
    ///
    /// Used to decide whether taking one is worth it. Counting only our own is
    /// deliberate: a snapshot cannot retire another device's chunks.
    pub fn compactable_chunks(&self) -> Result<usize> {
        let mine = self.store.list(self.device)?;
        let floor = self.store.latest_snapshot(self.device)?.unwrap_or(0);
        Ok(mine.iter().filter(|id| id.seq > floor).count())
    }

    /// Takes a snapshot and removes what it replaced, if the log has grown
    /// enough to be worth it.
    ///
    /// Pruning only happens once the snapshot has reached `target`, so a device
    /// that has been away for months still finds either the old chunks or the
    /// snapshot that stands in for them.
    pub fn compact_if_needed(
        &mut self,
        threshold: usize,
        target: Option<&ChunkStore>,
    ) -> Result<Compaction> {
        if self.compactable_chunks()? < threshold {
            return Ok(Compaction::default());
        }

        let snapshot = self.snapshot()?;
        if let (Some(_), Some(t)) = (snapshot, target) {
            t.replicate_from(&self.store)?;
        }

        // Every device's superseded chunks, not just ours — and in the target
        // as well as here, or the next machine to sync would push them all
        // back. See `prune_all_superseded`.
        let mut pruned = self.store.prune_all_superseded(target)?;
        if let Some(t) = target {
            pruned += t.prune_all_superseded(None)?;
        }

        Ok(Compaction { snapshot, pruned })
    }

    /// Pushes local chunks to a sync target and pulls back whatever else is
    /// there, then applies it.
    ///
    /// The target is a plain folder — a Dropbox or iCloud directory, a NAS
    /// mount, an external disk. It only ever holds ciphertext and is never
    /// given the key.
    pub fn sync_through(&mut self, target: &ChunkStore) -> Result<SyncOutcome> {
        let pushed = target.replicate_from(&self.store)?.len();
        let pulled = self.store.replicate_from(target)?.len();
        let applied = self.absorb()?;
        Ok(SyncOutcome {
            pushed,
            pulled,
            applied,
        })
    }

    /// Persists the set of applied chunk ids so a restart can skip them.
    ///
    /// This is an optimisation, not a correctness fix: applying the same
    /// change twice is a no-op in Automerge. But for a vault with thousands
    /// of chunks, skipping them shaves seconds off startup.
    pub fn save_applied(&self) -> Result<()> {
        let dir = self.store.root().join(".norm").join("state");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("applied");

        let mut lines = String::new();
        let mut sorted: Vec<_> = self.applied.iter().collect();
        sorted.sort_by_key(|id| (id.device, id.seq));
        for id in sorted {
            lines.push_str(&format!("{} {}\n", id.device.to_hex(), id.seq));
        }

        let tmp = {
            let mut s = path.clone().into_os_string();
            s.push(".tmp");
            std::path::PathBuf::from(s)
        };
        let mut f = std::fs::File::create(&tmp)?;
        use std::io::Write;
        f.write_all(lines.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Loads previously persisted applied chunk ids.
    pub fn load_applied(&mut self) -> Result<()> {
        let path = self.store.root().join(".norm").join("state").join("applied");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() { continue; }
            let mut parts = line.splitn(2, ' ');
            let (Some(dev_hex), Some(seq_str)) = (parts.next(), parts.next()) else {
                continue;
            };
            if let (Some(device), Ok(seq)) = (
                DeviceId::from_hex(dev_hex),
                seq_str.parse::<u64>()
            ) {
                self.applied.insert(ChunkId {
                    device,
                    seq,
                    kind: ChunkKind::Op, // kind doesn't matter for the applied set
                });
            }
        }
        Ok(())
    }

    fn doc_mut(&mut self, id: &DocId) -> Result<&mut NoteDoc> {
        if !self.docs.contains_key(id) {
            let fresh = NoteDoc::new(self.device)?;
            self.docs.insert(id.clone(), fresh);
        }
        Ok(self.docs.get_mut(id).expect("inserted above"))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Compaction {
    pub snapshot: Option<ChunkId>,
    pub pruned: usize,
}

impl Compaction {
    pub fn happened(&self) -> bool {
        self.snapshot.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncOutcome {
    pub pushed: usize,
    pub pulled: usize,
    pub applied: usize,
}

impl SyncOutcome {
    pub fn is_idle(&self) -> bool {
        self.pushed == 0 && self.pulled == 0 && self.applied == 0
    }
}
