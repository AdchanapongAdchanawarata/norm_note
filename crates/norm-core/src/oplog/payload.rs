//! Wire format for what an oplog chunk carries.
//!
//! # The format
//!
//! All integers little-endian. This is the plaintext that goes inside a chunk's
//! AEAD envelope; the envelope handles authentication, so nothing here needs a
//! checksum.
//!
//! ```text
//! batch   := u16 version | u32 cover_count | cover* | u32 entry_count | entry*
//! cover   := 16 bytes device_id | u64 seq
//! entry   := u32 doc_len  | doc_utf8       | u32 change_count | change*
//! change  := u32 len      | change_bytes
//! ```
//!
//! # Coverage
//!
//! `cover` is empty for an ordinary chunk of changes. A snapshot fills it in
//! with the highest sequence number it has seen from each device, which is what
//! makes the log shrinkable: without it, a snapshot can only retire chunks its
//! own author wrote, even though the state it carries already includes
//! everything that author had absorbed from everyone else.
//!
//! Five simulated years with that missing left 25,407 chunks for 13,137 edits.
//!
//! # Why hand-written rather than serde
//!
//! G2 promises the sync protocol is specified in the open so a vault outlives
//! this project. A format defined by whichever serialisation crate we happened
//! to pick is not a specification — it is a dependency, and its stability is
//! someone else's decision. Twelve lines of length-prefixing costs less than
//! that obligation.
//!
//! The previous JSON encoding also expanded every byte of Automerge change data
//! into a decimal number plus a comma, roughly quadrupling the log.
//!
//! # A chunk carries many notes
//!
//! One chunk per edited note would mean 5,000 chunks — and 5,000 fsyncs — to
//! import a vault. Batching makes that one chunk, and costs nothing in the
//! common case of a single note being saved.

use crate::oplog::DeviceId;
use crate::{Error, Result};

pub const FORMAT_VERSION: u16 = 2;

/// Edits to one note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub doc: String,
    pub changes: Vec<Vec<u8>>,
}

/// Everything one chunk carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batch {
    /// For a snapshot: the highest sequence number from each device that this
    /// state already includes. Empty for a chunk of changes.
    pub covers: Vec<(DeviceId, u64)>,
    pub entries: Vec<Entry>,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.entries.iter().all(|e| e.changes.is_empty())
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());

        out.extend_from_slice(&(self.covers.len() as u32).to_le_bytes());
        for (device, seq) in &self.covers {
            out.extend_from_slice(&device.0);
            out.extend_from_slice(&seq.to_le_bytes());
        }

        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());

        for entry in &self.entries {
            let doc = entry.doc.as_bytes();
            out.extend_from_slice(&(doc.len() as u32).to_le_bytes());
            out.extend_from_slice(doc);
            out.extend_from_slice(&(entry.changes.len() as u32).to_le_bytes());
            for change in &entry.changes {
                out.extend_from_slice(&(change.len() as u32).to_le_bytes());
                out.extend_from_slice(change);
            }
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);

        let version = r.u16()?;
        if version != FORMAT_VERSION {
            return Err(bad(&format!("unsupported payload version {version}")));
        }

        // Each cover is 24 bytes, so `counted`'s four-bytes-minimum bound is
        // conservative here, which is the safe direction.
        let cover_count = r.counted()?;
        let mut covers = Vec::with_capacity(cover_count);
        for _ in 0..cover_count {
            let mut device = [0u8; 16];
            device.copy_from_slice(r.take(16)?);
            let mut seq = [0u8; 8];
            seq.copy_from_slice(r.take(8)?);
            covers.push((DeviceId(device), u64::from_le_bytes(seq)));
        }

        let entry_count = r.counted()?;
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            let doc = r.string()?;
            let change_count = r.counted()?;
            let mut changes = Vec::with_capacity(change_count);
            for _ in 0..change_count {
                changes.push(r.bytes()?.to_vec());
            }
            entries.push(Entry { doc, changes });
        }

        if !r.is_empty() {
            return Err(bad("trailing bytes after the last entry"));
        }
        Ok(Batch { covers, entries })
    }
}

fn bad(reason: &str) -> Error {
    Error::Chunk {
        name: "<payload>".to_owned(),
        reason: reason.to_owned(),
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn is_empty(&self) -> bool {
        self.pos == self.bytes.len()
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(bad("payload ends mid-field"));
        }
        let out = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A count of items still to be read.
    ///
    /// Checked against the bytes actually left, so a corrupt or hostile length
    /// cannot make us reserve gigabytes. Every item costs at least four bytes,
    /// which is the bound used here.
    fn counted(&mut self) -> Result<usize> {
        let n = self.u32()? as usize;
        if n.saturating_mul(4) > self.remaining() {
            return Err(bad("declared count exceeds the bytes available"));
        }
        Ok(n)
    }

    fn bytes(&mut self) -> Result<&'a [u8]> {
        let len = self.u32()? as usize;
        self.take(len)
    }

    fn string(&mut self) -> Result<String> {
        let b = self.bytes()?;
        String::from_utf8(b.to_vec()).map_err(|_| bad("document name is not valid utf-8"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Batch {
        Batch {
            covers: vec![(DeviceId([1; 16]), 42), (DeviceId([2; 16]), u64::MAX)],
            entries: vec![
                Entry {
                    doc: "notes/a.md".into(),
                    changes: vec![vec![1, 2, 3], vec![4]],
                },
                Entry {
                    doc: "บันทึก/ประชุม.md".into(),
                    changes: vec![vec![9; 300]],
                },
            ],
        }
    }

    #[test]
    fn round_trips() {
        assert_eq!(Batch::decode(&sample().encode()).unwrap(), sample());
    }

    #[test]
    fn empty_batch_round_trips() {
        let b = Batch::default();
        assert_eq!(Batch::decode(&b.encode()).unwrap(), b);
    }

    #[test]
    fn coverage_survives_the_round_trip() {
        // If this were silently dropped, snapshots would stop retiring other
        // devices' chunks and the log would quietly grow for ever again.
        let back = Batch::decode(&sample().encode()).unwrap();
        assert_eq!(back.covers, sample().covers);
    }

    #[test]
    fn an_entry_with_no_changes_round_trips() {
        let b = Batch {
            covers: Vec::new(),
            entries: vec![Entry {
                doc: "a.md".into(),
                changes: vec![],
            }],
        };
        assert_eq!(Batch::decode(&b.encode()).unwrap(), b);
        assert!(b.is_empty());
    }

    #[test]
    fn truncation_is_an_error_not_a_panic() {
        let full = sample().encode();
        for cut in 0..full.len() {
            // Must never panic, and must never silently return a short batch.
            match Batch::decode(&full[..cut]) {
                Err(_) => {}
                Ok(b) => panic!("accepted a truncated payload at {cut}: {b:?}"),
            }
        }
    }

    #[test]
    fn a_hostile_length_does_not_allocate() {
        // version 1, entry_count = u32::MAX, nothing else.
        let mut bytes = FORMAT_VERSION.to_le_bytes().to_vec();
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(Batch::decode(&bytes).is_err());
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut bytes = sample().encode();
        bytes.push(0);
        assert!(Batch::decode(&bytes).is_err());
    }

    #[test]
    fn a_future_version_is_refused_rather_than_misread() {
        let mut bytes = sample().encode();
        bytes[0..2].copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
        assert!(Batch::decode(&bytes).is_err());
    }

    #[test]
    fn is_far_smaller_than_the_json_it_replaces() {
        let batch = Batch {
            covers: Vec::new(),
            entries: vec![Entry {
                doc: "a.md".into(),
                changes: vec![vec![200u8; 1000]],
            }],
        };
        let binary = batch.encode().len();
        // What serde_json produced: every byte became a decimal plus a comma.
        let json_ish = 1000 * 4;
        assert!(
            binary < json_ish / 3,
            "binary {binary} was not much smaller than json {json_ish}"
        );
    }
}
