//! The append-only encrypted operation log.
//!
//! This is the piece that makes a dumb folder — Dropbox, iCloud Drive, a NAS
//! mount, an S3 bucket — a safe place to sync through.
//!
//! # Why this format cannot be corrupted by a syncing folder
//!
//! Cloud storage products resolve concurrent writes to the same path by
//! last-write-wins, or by dropping a `file (conflicted copy).md` next to it.
//! That is precisely how Obsidian vaults get mangled: two devices write
//! `note.md`, and the folder has to pick a winner.
//!
//! Here, a chunk's path contains the id of the device that wrote it:
//!
//! ```text
//! .norm/sync/ops/<device-hex>/<seq:020>.op
//! ```
//!
//! No two devices ever write the same path, so there is never a write to
//! resolve. The only operation the folder has to perform correctly is "a new
//! file appeared" — the one operation every one of these products gets right.
//! Chunks are immutable once written; state moves forward only by appending.
//!
//! # Nonce derivation depends on append-only
//!
//! The XChaCha20-Poly1305 nonce is `device_id || seq`, which is exactly 24
//! bytes and unique by construction — no RNG, no nonce database, no risk of a
//! repeat across devices.
//!
//! This is safe **only** while chunks are immutable. Encrypting different
//! plaintext under the same (key, nonce) pair breaks the cipher outright. So
//! [`ChunkStore::append`] must refuse to overwrite an existing chunk, and
//! writes must be atomic (temp file, then rename) so a crash cannot leave a
//! partial chunk that a retry would rewrite with different bytes.

pub mod payload;
pub mod store;

use std::fmt;

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    Key, XChaCha20Poly1305, XNonce,
};

use crate::{Error, Result};

pub const MAGIC: [u8; 4] = *b"NRM1";
pub const FORMAT_VERSION: u16 = 1;
pub const HEADER_LEN: usize = 4 + 2 + 16 + 8 + 1 + 24;

/// Stable identifier for one installation on one device.
///
/// Random at first launch and never reused. It is not a secret — it appears in
/// chunk paths in plaintext — so it must not encode anything about the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId(pub [u8; 16]);

impl DeviceId {
    pub fn to_hex(self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 32 {
            return None;
        }
        let mut out = [0u8; 16];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        Some(DeviceId(out))
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// What a chunk carries.
///
/// The distinction lives in the filename so that pruning — working out which
/// chunks a snapshot has made redundant — needs no key. Tidying up a sync
/// target should never require decrypting anything in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChunkKind {
    /// Changes since the writer's previous chunk.
    Op,
    /// The writer's complete state for the notes it names. Supersedes that
    /// writer's earlier chunks.
    Snapshot,
}

impl ChunkKind {
    pub fn extension(self) -> &'static str {
        match self {
            ChunkKind::Op => "op",
            ChunkKind::Snapshot => "snap",
        }
    }

    fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "op" => Some(ChunkKind::Op),
            "snap" => Some(ChunkKind::Snapshot),
            _ => None,
        }
    }

    fn tag(self) -> u8 {
        match self {
            ChunkKind::Op => 0,
            ChunkKind::Snapshot => 1,
        }
    }

    fn from_tag(t: u8) -> Option<Self> {
        match t {
            0 => Some(ChunkKind::Op),
            1 => Some(ChunkKind::Snapshot),
            _ => None,
        }
    }
}

/// Identifies one chunk: which device wrote it, and where in that device's
/// sequence it sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkId {
    pub device: DeviceId,
    pub seq: u64,
    pub kind: ChunkKind,
}

impl ChunkId {
    pub fn op(device: DeviceId, seq: u64) -> Self {
        Self {
            device,
            seq,
            kind: ChunkKind::Op,
        }
    }

    pub fn snapshot(device: DeviceId, seq: u64) -> Self {
        Self {
            device,
            seq,
            kind: ChunkKind::Snapshot,
        }
    }

    /// Relative path within the vault.
    ///
    /// `seq` is zero-padded to 20 digits (`u64::MAX` is 20 digits) so that a
    /// plain lexical directory listing is already in sequence order. Every
    /// filesystem and object store sorts this correctly without us reading or
    /// parsing anything.
    pub fn relative_path(&self) -> String {
        format!(
            "{}/ops/{}/{:020}.{}",
            SYNC_DIR,
            self.device.to_hex(),
            self.seq,
            self.kind.extension()
        )
    }

    pub fn from_relative_path(path: &str) -> Option<Self> {
        let rest = path.replace('\\', "/");
        let rest = rest.strip_prefix(SYNC_DIR)?.strip_prefix("/ops/")?;
        let (device_hex, file) = rest.split_once('/')?;
        let (seq, ext) = file.rsplit_once('.')?;
        Some(ChunkId {
            device: DeviceId::from_hex(device_hex)?,
            seq: seq.parse().ok()?,
            kind: ChunkKind::from_extension(ext)?,
        })
    }
}

pub const SYNC_DIR: &str = ".norm/sync";

/// Plaintext preamble of a chunk. Authenticated but not encrypted: the device
/// id and sequence are already visible in the path, and binding them into the
/// AEAD means a chunk moved or renamed inside the folder fails to open rather
/// than being silently accepted in the wrong place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkHeader {
    pub format: u16,
    pub id: ChunkId,
    pub nonce: [u8; 24],
}

impl ChunkHeader {
    /// Nonce is `device_id || seq` — unique by construction. See module docs
    /// for why this is sound and what it depends on.
    pub fn new(id: ChunkId) -> Self {
        let mut nonce = [0u8; 24];
        nonce[..16].copy_from_slice(&id.device.0);
        nonce[16..].copy_from_slice(&id.seq.to_le_bytes());
        Self {
            format: FORMAT_VERSION,
            id,
            nonce,
        }
    }

    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0..4].copy_from_slice(&MAGIC);
        out[4..6].copy_from_slice(&self.format.to_le_bytes());
        out[6..22].copy_from_slice(&self.id.device.0);
        out[22..30].copy_from_slice(&self.id.seq.to_le_bytes());
        // Authenticated, so renaming a snapshot to `.op` cannot make a reader
        // treat a whole document as if it were a delta.
        out[30] = self.id.kind.tag();
        out[31..55].copy_from_slice(&self.nonce);
        out
    }

    pub fn decode(bytes: &[u8], name: &str) -> Result<Self> {
        let err = |reason: &str| Error::Chunk {
            name: name.to_owned(),
            reason: reason.to_owned(),
        };
        if bytes.len() < HEADER_LEN {
            return Err(err("shorter than a header"));
        }
        if bytes[0..4] != MAGIC {
            return Err(err("bad magic"));
        }
        let format = u16::from_le_bytes([bytes[4], bytes[5]]);
        if format != FORMAT_VERSION {
            return Err(err(&format!("unsupported format version {format}")));
        }
        let mut device = [0u8; 16];
        device.copy_from_slice(&bytes[6..22]);
        let mut seq = [0u8; 8];
        seq.copy_from_slice(&bytes[22..30]);
        let kind = ChunkKind::from_tag(bytes[30]).ok_or_else(|| err("unknown chunk kind"))?;
        let mut nonce = [0u8; 24];
        nonce.copy_from_slice(&bytes[31..55]);

        Ok(ChunkHeader {
            format,
            id: ChunkId {
                device: DeviceId(device),
                seq: u64::from_le_bytes(seq),
                kind,
            },
            nonce,
        })
    }
}

/// Symmetric key for one vault. Derived from the user's recovery phrase and
/// held outside the vault; it never leaves the device.
///
/// # Deliberately awkward
///
/// The bytes are private and there is no accessor, no `Clone`, no `Copy` and
/// no useful `Debug`. Key material that can be read out of a struct ends up
/// read out of a struct — into a log line, an error message, a debug print
/// added while chasing something else. Encryption whose key leaks into a log
/// file is decoration.
///
/// Everything that legitimately needs the key takes a `&VaultKey` and hands it
/// to [`seal`] or [`open`], which are the only places the bytes are visible.
pub struct VaultKey([u8; 32]);

impl VaultKey {
    pub fn new(bytes: [u8; 32]) -> Self {
        VaultKey(bytes)
    }
}

impl Drop for VaultKey {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.0.zeroize();
    }
}

impl fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VaultKey(redacted)")
    }
}

/// Encrypts `payload` into a self-describing chunk: `header || ciphertext`.
pub fn seal(key: &VaultKey, id: ChunkId, payload: &[u8]) -> Result<Vec<u8>> {
    let header = ChunkHeader::new(id);
    let aad = header.encode();

    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key.0));
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&header.nonce),
            Payload {
                msg: payload,
                aad: &aad,
            },
        )
        .map_err(|_| Error::Chunk {
            name: id.relative_path(),
            reason: "encryption failed".to_owned(),
        })?;

    let mut out = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    out.extend_from_slice(&aad);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypts a chunk and checks it is the chunk we expected to find at `id`.
pub fn open(key: &VaultKey, id: ChunkId, bytes: &[u8]) -> Result<Vec<u8>> {
    let name = id.relative_path();
    let header = ChunkHeader::decode(bytes, &name)?;

    if header.id != id {
        return Err(Error::Chunk {
            name,
            reason: "header does not match the path the chunk was found at".to_owned(),
        });
    }

    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key.0));
    cipher
        .decrypt(
            XNonce::from_slice(&header.nonce),
            Payload {
                msg: &bytes[HEADER_LEN..],
                aad: &bytes[..HEADER_LEN],
            },
        )
        .map_err(|_| Error::Decrypt { name })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(n: u8) -> DeviceId {
        DeviceId([n; 16])
    }

    fn key() -> VaultKey {
        VaultKey::new([7u8; 32])
    }

    #[test]
    fn device_id_hex_round_trips() {
        let d = dev(0xab);
        assert_eq!(DeviceId::from_hex(&d.to_hex()), Some(d));
        assert_eq!(DeviceId::from_hex("nothex"), None);
    }

    #[test]
    fn chunk_path_round_trips() {
        for id in [ChunkId::op(dev(1), 42), ChunkId::snapshot(dev(1), 42)] {
            let path = id.relative_path();
            assert!(path.contains("/00000000000000000042."));
            assert_eq!(ChunkId::from_relative_path(&path), Some(id));
        }
    }

    #[test]
    fn an_op_and_a_snapshot_are_different_files() {
        assert_ne!(
            ChunkId::op(dev(1), 7).relative_path(),
            ChunkId::snapshot(dev(1), 7).relative_path()
        );
    }

    #[test]
    fn paths_sort_lexically_in_sequence_order() {
        let d = dev(1);
        let mut paths: Vec<_> = [10u64, 2, 100, 1]
            .into_iter()
            .map(|seq| ChunkId::op(d, seq).relative_path())
            .collect();
        paths.sort();
        let seqs: Vec<_> = paths
            .iter()
            .map(|p| ChunkId::from_relative_path(p).unwrap().seq)
            .collect();
        assert_eq!(seqs, vec![1, 2, 10, 100]);
    }

    #[test]
    fn two_devices_never_collide_on_a_path() {
        assert_ne!(
            ChunkId::op(dev(1), 5).relative_path(),
            ChunkId::op(dev(2), 5).relative_path(),
            "path collision would hand conflict resolution to the cloud"
        );
    }

    #[test]
    fn nonce_is_unique_per_chunk() {
        let n1 = ChunkHeader::new(ChunkId::op(dev(1), 1)).nonce;
        let n2 = ChunkHeader::new(ChunkId::op(dev(1), 2)).nonce;
        let n3 = ChunkHeader::new(ChunkId::op(dev(2), 1)).nonce;
        assert_ne!(n1, n2);
        assert_ne!(n1, n3);
    }

    #[test]
    fn seal_open_round_trips() {
        for id in [ChunkId::op(dev(3), 9), ChunkId::snapshot(dev(3), 9)] {
            let sealed = seal(&key(), id, b"hello oplog").unwrap();
            assert_eq!(open(&key(), id, &sealed).unwrap(), b"hello oplog");
        }
    }

    #[test]
    fn tampering_is_detected() {
        let id = ChunkId::op(dev(3), 9);
        let mut sealed = seal(&key(), id, b"hello oplog").unwrap();
        *sealed.last_mut().unwrap() ^= 0xff;
        assert!(matches!(
            open(&key(), id, &sealed),
            Err(Error::Decrypt { .. })
        ));
    }

    #[test]
    fn a_chunk_moved_to_another_path_will_not_open() {
        // Guards against a folder-sync product relocating or duplicating files.
        let id = ChunkId::op(dev(3), 9);
        let sealed = seal(&key(), id, b"payload").unwrap();
        assert!(open(&key(), ChunkId::op(dev(3), 10), &sealed).is_err());
    }

    #[test]
    fn renaming_an_op_to_a_snapshot_will_not_open() {
        // The kind is authenticated, so a reader can never be tricked into
        // treating a delta as a whole document or the other way round.
        let id = ChunkId::op(dev(3), 9);
        let sealed = seal(&key(), id, b"payload").unwrap();
        assert!(open(&key(), ChunkId::snapshot(dev(3), 9), &sealed).is_err());
    }

    #[test]
    fn wrong_key_does_not_open() {
        let id = ChunkId::op(dev(3), 9);
        let sealed = seal(&key(), id, b"payload").unwrap();
        let other = VaultKey::new([8u8; 32]);
        assert!(matches!(
            open(&other, id, &sealed),
            Err(Error::Decrypt { .. })
        ));
    }
}
