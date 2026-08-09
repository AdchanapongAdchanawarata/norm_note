//! On-disk storage for the operation log.
//!
//! Upholds the invariant the whole encryption scheme rests on:
//!
//! > A given `(device, seq)` is never used for two different payloads.
//!
//! [`super`] derives the AEAD nonce as `device_id || seq`. Reusing a nonce with
//! different plaintext under the same key does not degrade XChaCha20-Poly1305,
//! it breaks it. So sequence allocation has to survive a crash.
//!
//! # Why the obvious allocation strategy is wrong
//!
//! The natural approach — scan the directory, take the highest `seq`, add one —
//! is unsafe. Write chunk `N` to a temp file, crash before the rename, and the
//! next scan sees a maximum of `N-1` and hands out `N` again, now for different
//! content. Same key, same nonce, different plaintext.
//!
//! So the counter is durable and advances *before* the chunk is written:
//!
//! ```text
//! 1. n = read(state/<device>.seq)
//! 2. write(state/<device>.seq, n+1) and fsync   <- the commit point
//! 3. write chunk to <seq>.op.tmp and fsync
//! 4. rename to <seq>.op
//! ```
//!
//! A crash at any point burns a sequence number but can never reuse one.
//!
//! # Consequence: gaps are legal
//!
//! Because a crash between steps 2 and 4 leaves a hole, readers must treat the
//! sequence as strictly increasing but **not** contiguous. A missing `seq` is
//! not evidence of a lost chunk and must never be waited on — doing so would
//! stall sync forever after one badly-timed power cut.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{seal, ChunkId, ChunkKind, DeviceId, VaultKey, SYNC_DIR};
use crate::{Error, Result};

const TMP_SUFFIX: &str = ".tmp";

/// Reads and appends chunks under a vault (or under a sync target folder —
/// the layout is identical in both places).
pub struct ChunkStore {
    root: PathBuf,
}

impl ChunkStore {
    /// `root` is the directory *containing* `.norm/sync`, i.e. the vault root
    /// or the sync target root.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn sync_dir(&self) -> PathBuf {
        self.root.join(SYNC_DIR)
    }

    fn ops_dir(&self, device: DeviceId) -> PathBuf {
        self.sync_dir().join("ops").join(device.to_hex())
    }

    fn seq_file(&self, device: DeviceId) -> PathBuf {
        self.sync_dir()
            .join("state")
            .join(format!("{}.seq", device.to_hex()))
    }

    /// Encrypts and appends `payload` as this device's next chunk.
    ///
    /// The sequence number is committed durably before the chunk is written, so
    /// an interrupted append leaves a gap rather than a reusable number.
    pub fn append(
        &self,
        key: &VaultKey,
        device: DeviceId,
        kind: ChunkKind,
        payload: &[u8],
    ) -> Result<ChunkId> {
        let seq = self.reserve_seq(device)?;
        let id = ChunkId { device, seq, kind };

        let dir = self.ops_dir(device);
        fs::create_dir_all(&dir)?;

        let final_path = self.root.join(id.relative_path());
        let tmp_path = with_suffix(&final_path, TMP_SUFFIX);

        write_durably(&tmp_path, &seal(key, id, payload)?)?;

        // Never clobber. If the destination somehow exists, the sequence
        // counter and the directory disagree; that is a bug worth surfacing
        // loudly rather than resolving by overwriting a chunk other devices
        // may already have replicated.
        if final_path.exists() {
            let _ = fs::remove_file(&tmp_path);
            return Err(Error::Chunk {
                name: id.relative_path(),
                reason: "chunk already exists; refusing to overwrite".to_owned(),
            });
        }

        fs::rename(&tmp_path, &final_path)?;
        sync_dir_best_effort(&dir);
        Ok(id)
    }

    /// Reads and decrypts one chunk.
    pub fn read(&self, key: &VaultKey, id: ChunkId) -> Result<Vec<u8>> {
        let bytes = fs::read(self.chunk_path(id))?;
        super::open(key, id, &bytes)
    }

    pub fn chunk_path(&self, id: ChunkId) -> PathBuf {
        self.root.join(id.relative_path())
    }

    pub fn has(&self, id: ChunkId) -> bool {
        self.chunk_path(id).exists()
    }

    /// Copies every chunk in `source` that is missing here.
    ///
    /// This is the whole of replication. Chunks are immutable and their paths
    /// are unique per device, so bringing two stores into agreement is a file
    /// copy — there is nothing to merge, nothing to order, and no winner to
    /// pick. Running it twice, or halfway, or in either direction, converges to
    /// the same result.
    ///
    /// Bytes are copied without decrypting: a sync target only ever relays
    /// ciphertext, and never needs the vault key.
    ///
    /// # Sequence counters are deliberately untouched
    ///
    /// A device only ever allocates sequence numbers for *itself*. Advancing
    /// the local counter to match replicated chunks would be a subtle way to
    /// reintroduce nonce reuse: this device would start issuing numbers that
    /// another device has already used under the same key.
    pub fn replicate_from(&self, source: &ChunkStore) -> Result<Vec<ChunkId>> {
        let mut copied = Vec::new();

        for device in source.devices()? {
            for id in source.list(device)? {
                if self.has(id) {
                    continue;
                }
                let bytes = fs::read(source.chunk_path(id))?;
                self.write_chunk_raw(id, &bytes)?;
                copied.push(id);
            }
        }

        copied.sort();
        Ok(copied)
    }

    /// Places already-sealed bytes at `id`. Used only by replication; new
    /// content must go through [`Self::append`] so it gets a reserved sequence.
    fn write_chunk_raw(&self, id: ChunkId, bytes: &[u8]) -> Result<()> {
        let dir = self.ops_dir(id.device);
        fs::create_dir_all(&dir)?;

        let final_path = self.chunk_path(id);
        if final_path.exists() {
            return Ok(());
        }

        let tmp_path = with_suffix(&final_path, TMP_SUFFIX);
        write_durably(&tmp_path, bytes)?;

        // A concurrent replication may have won the race; that is fine, the
        // bytes are identical either way.
        if final_path.exists() {
            let _ = fs::remove_file(&tmp_path);
            return Ok(());
        }

        fs::rename(&tmp_path, &final_path)?;
        sync_dir_best_effort(&dir);
        Ok(())
    }

    /// Every chunk this device has written, in sequence order. Gaps are
    /// expected; see the module docs.
    pub fn list(&self, device: DeviceId) -> Result<Vec<ChunkId>> {
        let dir = self.ops_dir(device);
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut ids = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let name = entry?.file_name();
            let Some(name) = name.to_str() else { continue };
            // Leftover temp files are crash debris, not chunks.
            if name.ends_with(TMP_SUFFIX) {
                continue;
            }
            let Some((seq, ext)) = name.rsplit_once('.') else {
                continue;
            };
            let (Ok(seq), Some(kind)) = (seq.parse::<u64>(), ChunkKind::from_extension(ext)) else {
                continue;
            };
            ids.push(ChunkId { device, seq, kind });
        }
        ids.sort();
        Ok(ids)
    }

    /// The highest sequence at which `device` has written a snapshot, if any.
    /// Everything that device wrote before it is redundant.
    pub fn latest_snapshot(&self, device: DeviceId) -> Result<Option<u64>> {
        Ok(self
            .list(device)?
            .into_iter()
            .filter(|id| id.kind == ChunkKind::Snapshot)
            .map(|id| id.seq)
            .max())
    }

    /// Deletes chunks from every device that a snapshot has replaced.
    ///
    /// # Pruning only our own chunks does not work
    ///
    /// That was the first attempt, on the reasoning that another device's log
    /// was not ours to judge. Five simulated years showed why it fails:
    /// machine A snapshots and deletes its old chunks from both its own store
    /// and the target — and then machine B, which still holds copies of them,
    /// pushes them straight back. A pulls them down again. Nothing is ever
    /// actually removed, and the log grows for ever.
    ///
    /// A snapshot *is* the judgement, and it comes from the device whose
    /// chunks it replaces, so any holder of it can act on it. Every device
    /// prunes the same set and nothing resurrects.
    ///
    /// `mirrored_in` is the sync target. Chunks go only when the snapshot that
    /// stands in for them is already there — otherwise a machine that has not
    /// synced in months would return to a hole where its history used to be.
    ///
    /// Pruning is never needed for correctness, only for size. Skipping it
    /// costs disk; doing it too eagerly costs notes.
    pub fn prune_all_superseded(&self, mirrored_in: Option<&ChunkStore>) -> Result<usize> {
        let mut removed = 0;
        for device in self.devices()? {
            removed += self.prune_superseded(device, mirrored_in)?;
        }
        Ok(removed)
    }

    /// Prunes one device's superseded chunks. See [`Self::prune_all_superseded`].
    pub fn prune_superseded(
        &self,
        device: DeviceId,
        mirrored_in: Option<&ChunkStore>,
    ) -> Result<usize> {
        let Some(snapshot_seq) = self.latest_snapshot(device)? else {
            return Ok(0);
        };

        if let Some(remote) = mirrored_in {
            if !remote.has(ChunkId::snapshot(device, snapshot_seq)) {
                return Ok(0);
            }
        }

        let mut removed = 0;
        for id in self.list(device)? {
            if id.seq >= snapshot_seq {
                continue;
            }
            fs::remove_file(self.chunk_path(id))?;
            removed += 1;
        }
        Ok(removed)
    }

    /// Every device that has written to this store.
    pub fn devices(&self) -> Result<Vec<DeviceId>> {
        let dir = self.sync_dir().join("ops");
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut devices = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let name = entry?.file_name();
            if let Some(d) = name.to_str().and_then(DeviceId::from_hex) {
                devices.push(d);
            }
        }
        devices.sort();
        Ok(devices)
    }

    /// Chunks from `device` after `after_seq`, for pulling into another store.
    pub fn list_since(&self, device: DeviceId, after_seq: Option<u64>) -> Result<Vec<ChunkId>> {
        let all = self.list(device)?;
        Ok(match after_seq {
            Some(n) => all.into_iter().filter(|id| id.seq > n).collect(),
            None => all,
        })
    }

    /// Deletes `.tmp` files left behind by an interrupted append. Safe to run
    /// at any time: a temp file is never referenced by anything.
    pub fn clean_temp_files(&self, device: DeviceId) -> Result<usize> {
        let dir = self.ops_dir(device);
        if !dir.exists() {
            return Ok(0);
        }

        let mut removed = 0;
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.to_str().is_some_and(|p| p.ends_with(TMP_SUFFIX)) {
                fs::remove_file(&path)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Durably claims the next sequence number. This is the commit point that
    /// makes nonce reuse impossible across crashes.
    fn reserve_seq(&self, device: DeviceId) -> Result<u64> {
        let path = self.seq_file(device);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let next = match fs::read_to_string(&path) {
            Ok(s) => s.trim().parse::<u64>().map_err(|_| Error::Chunk {
                name: path.display().to_string(),
                reason: "sequence counter is not a number".to_owned(),
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(e) => return Err(e.into()),
        };

        // Advance and flush to disk *before* returning, so a crash loses a
        // number rather than handing it out twice.
        let tmp = with_suffix(&path, TMP_SUFFIX);
        write_durably(&tmp, (next + 1).to_string().as_bytes())?;
        fs::rename(&tmp, &path)?;
        if let Some(parent) = path.parent() {
            sync_dir_best_effort(parent);
        }

        Ok(next)
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn write_durably(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

/// Flushing a directory entry is how POSIX guarantees a rename survives a power
/// cut. Windows has no equivalent call and returns an error for a directory
/// handle here, so this is deliberately best-effort rather than fallible.
fn sync_dir_best_effort(dir: &Path) {
    #[cfg(unix)]
    if let Ok(f) = fs::File::open(dir) {
        let _ = f.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(n: u8) -> DeviceId {
        DeviceId([n; 16])
    }

    fn key() -> VaultKey {
        VaultKey::new([42u8; 32])
    }

    fn store() -> (tempfile::TempDir, ChunkStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ChunkStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn append_then_read_round_trips() {
        let (_d, s) = store();
        let id = s.append(&key(), dev(1), ChunkKind::Op, b"first").unwrap();
        assert_eq!(s.read(&key(), id).unwrap(), b"first");
    }

    #[test]
    fn sequence_starts_at_zero_and_increments() {
        let (_d, s) = store();
        let a = s.append(&key(), dev(1), ChunkKind::Op, b"a").unwrap();
        let b = s.append(&key(), dev(1), ChunkKind::Op, b"b").unwrap();
        assert_eq!((a.seq, b.seq), (0, 1));
    }

    #[test]
    fn each_device_has_its_own_sequence() {
        let (_d, s) = store();
        assert_eq!(
            s.append(&key(), dev(1), ChunkKind::Op, b"a").unwrap().seq,
            0
        );
        assert_eq!(
            s.append(&key(), dev(2), ChunkKind::Op, b"a").unwrap().seq,
            0
        );
        assert_eq!(
            s.append(&key(), dev(1), ChunkKind::Op, b"b").unwrap().seq,
            1
        );
    }

    #[test]
    fn a_crash_between_reserve_and_write_burns_a_number_but_never_reuses_it() {
        // The property the whole encryption scheme depends on. Simulates the
        // crash by reserving a number and never writing its chunk.
        let (_d, s) = store();
        let d = dev(1);

        let burned = s.reserve_seq(d).unwrap();
        let next = s
            .append(&key(), d, ChunkKind::Op, b"after the crash")
            .unwrap();

        assert_eq!(burned, 0);
        assert_eq!(
            next.seq, 1,
            "a reserved number must never be handed out twice"
        );
        assert_eq!(s.list(d).unwrap(), vec![next], "the gap at 0 is expected");
    }

    #[test]
    fn listing_is_sorted_and_tolerates_gaps() {
        let (_d, s) = store();
        let d = dev(1);
        s.append(&key(), d, ChunkKind::Op, b"a").unwrap();
        s.reserve_seq(d).unwrap(); // gap
        s.append(&key(), d, ChunkKind::Op, b"c").unwrap();

        let seqs: Vec<_> = s.list(d).unwrap().into_iter().map(|i| i.seq).collect();
        assert_eq!(seqs, vec![0, 2]);
    }

    #[test]
    fn temp_files_are_not_mistaken_for_chunks() {
        let (dir, s) = store();
        let d = dev(1);
        s.append(&key(), d, ChunkKind::Op, b"a").unwrap();

        let debris = dir
            .path()
            .join(SYNC_DIR)
            .join("ops")
            .join(d.to_hex())
            .join("00000000000000000009.op.tmp");
        fs::write(&debris, b"partial").unwrap();

        assert_eq!(s.list(d).unwrap().len(), 1);
        assert_eq!(s.clean_temp_files(d).unwrap(), 1);
        assert!(!debris.exists());
    }

    #[test]
    fn refuses_to_overwrite_an_existing_chunk() {
        let (_d, s) = store();
        let d = dev(1);
        let id = s.append(&key(), d, ChunkKind::Op, b"original").unwrap();

        // Force the counter backwards, the way a restored-from-backup
        // `.norm/` directory could.
        fs::write(s.seq_file(d), "0").unwrap();

        let err = s
            .append(&key(), d, ChunkKind::Op, b"different payload")
            .unwrap_err();
        assert!(matches!(err, Error::Chunk { .. }), "got {err:?}");
        assert_eq!(
            s.read(&key(), id).unwrap(),
            b"original",
            "original must survive"
        );
    }

    #[test]
    fn devices_are_discovered_from_the_directory() {
        let (_d, s) = store();
        s.append(&key(), dev(2), ChunkKind::Op, b"a").unwrap();
        s.append(&key(), dev(1), ChunkKind::Op, b"a").unwrap();
        assert_eq!(s.devices().unwrap(), vec![dev(1), dev(2)]);
    }

    #[test]
    fn list_since_filters_what_a_peer_already_has() {
        let (_d, s) = store();
        let d = dev(1);
        for _ in 0..4 {
            s.append(&key(), d, ChunkKind::Op, b"x").unwrap();
        }
        let seqs: Vec<_> = s
            .list_since(d, Some(1))
            .unwrap()
            .into_iter()
            .map(|i| i.seq)
            .collect();
        assert_eq!(seqs, vec![2, 3]);
    }
}
