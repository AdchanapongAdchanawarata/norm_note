//! The CRDT view of a note.
//!
//! # One text object, not a structured document
//!
//! A note is stored in Automerge as a single `Text` object holding the entire
//! file, frontmatter fence and all. The obvious alternative — frontmatter as a
//! `Map`, body as `Text` — would merge frontmatter key by key, which is nicer
//! in the cases where it applies.
//!
//! It is rejected because it cannot coexist with G3. Materialising a `Map` back
//! to YAML means re-serialising, and that reorders keys, drops comments and
//! rewrites quoting. A user who syncs a note without editing it would find the
//! file rewritten. Fidelity to the user's own file outranks merge elegance.
//!
//! The cost is real and worth stating: two devices editing *the same*
//! frontmatter key concurrently merge at the text level, which can produce a
//! line that no longer parses as YAML. Nothing is lost — both edits are in the
//! file — but a human has to look at it. That is the Merge Inbox case, and it
//! is a far better failure than a silently reordered file.
//!
//! # Edits go through `update_text`
//!
//! Automerge diffs the new text against the current one and emits the minimal
//! splices. Replacing the whole text instead would make every save look like
//! "deleted everything, typed everything", and two such saves would merge into
//! duplicated content rather than a sensible result.

use std::path::Path;

use automerge::{
    transaction::{CommitOptions, Transactable},
    ActorId, AutoCommit, Change, ChangeHash, ObjId, ObjType, ReadDoc, ROOT,
};
use unicode_normalization::UnicodeNormalization;

use crate::oplog::DeviceId;
use crate::{Error, Result};

const CONTENT: &str = "content";

/// Identity of a note within a vault.
///
/// # Why paths are normalised
///
/// The same file can reach two devices under byte-different names:
///
/// * macOS stores filenames decomposed (NFD) while Windows and Linux keep what
///   they were given (usually NFC). `café.md` typed on one and synced to the
///   other is the same file with different bytes. Thai filenames with combining
///   marks hit this the same way.
/// * Windows uses `\` as a separator, everything else uses `/`.
///
/// Without normalisation each device would treat the other's file as a new
/// note, and the user would watch their vault quietly fill with duplicates.
///
/// Case is deliberately *not* folded. Windows and macOS are usually
/// case-insensitive and Linux is not, so folding would merge two genuinely
/// distinct files on Linux. Case-only collisions are reported by `doctor`
/// rather than resolved by guessing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocId(String);

impl DocId {
    /// `path` must be relative to the vault root.
    pub fn from_relative_path(path: &Path) -> Self {
        let joined = path
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .collect::<Vec<_>>()
            .join("/");
        DocId(joined.nfc().collect())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DocId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Actor used for the genesis change only. Not a real device.
const GENESIS_ACTOR: [u8; 16] = *b"norm_note-gen-01";

/// The first change of every note, byte-identical on every device.
///
/// # Why this has to exist
///
/// Creating the content object locally on each device is the obvious
/// implementation and it is wrong. Two devices that independently create
/// `meeting.md` end up with two different `Text` objects competing for
/// `ROOT["content"]`. Automerge resolves the map conflict deterministically by
/// picking one — and the loser's text, along with everything written into it,
/// stops being visible.
///
/// It is a nasty failure because it hides. A device that only ever *receives* a
/// note after loading a peer's saved document shares the peer's genesis and
/// behaves perfectly. The bug appears when two devices each create the note on
/// their own — which is what happens the moment a note arrives as oplog changes
/// rather than as a whole document.
///
/// So genesis is fixed: a constant actor, a constant timestamp, no message.
/// Every device produces the same change hash, the same object id, and
/// independent creation of the same note converges instead of colliding.
fn genesis() -> &'static [u8] {
    static SEED: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    SEED.get_or_init(|| {
        let mut doc = AutoCommit::new().with_actor(ActorId::from(GENESIS_ACTOR.as_slice()));
        doc.put_object(ROOT, CONTENT, ObjType::Text)
            .expect("genesis put_object on a fresh document cannot fail");
        // A wall-clock timestamp here would make the hash differ per device and
        // reintroduce the collision this function exists to prevent.
        doc.commit_with(CommitOptions::default().with_time(0));
        doc.save()
    })
}

/// One note as a mergeable document.
pub struct NoteDoc {
    doc: AutoCommit,
}

impl NoteDoc {
    /// Creates an empty note authored by `device`.
    ///
    /// The Automerge actor is the device id, so a change is always attributable
    /// to the machine that made it — which is what lets `status` report per
    /// device instead of pretending sync is one global state.
    pub fn new(device: DeviceId) -> Result<Self> {
        Self::load(device, genesis())
    }

    pub fn from_text(device: DeviceId, text: &str) -> Result<Self> {
        let mut d = Self::new(device)?;
        d.set_text(text)?;
        Ok(d)
    }

    /// Restores a document from [`Self::save`].
    pub fn load(device: DeviceId, saved: &[u8]) -> Result<Self> {
        let doc = AutoCommit::load(saved)?.with_actor(ActorId::from(device.0.as_slice()));
        Ok(Self { doc })
    }

    pub fn save(&mut self) -> Vec<u8> {
        self.doc.save()
    }

    fn content(&self) -> Result<ObjId> {
        self.doc
            .get(ROOT, CONTENT)?
            .map(|(_, id)| id)
            .ok_or_else(|| Error::Crdt("document has no content object".to_owned()))
    }

    /// The note's full text, exactly as it should appear on disk.
    pub fn text(&self) -> Result<String> {
        Ok(self.doc.text(self.content()?)?)
    }

    /// Records an edit as the minimal diff against the current text.
    pub fn set_text(&mut self, new_text: &str) -> Result<()> {
        let obj = self.content()?;
        self.doc.update_text(&obj, new_text)?;
        Ok(())
    }

    pub fn heads(&mut self) -> Vec<ChangeHash> {
        self.doc.get_heads()
    }

    /// Serialised changes this document has that `have` does not. These are the
    /// payloads that go into oplog chunks.
    pub fn changes_since(&mut self, have: &[ChangeHash]) -> Vec<Vec<u8>> {
        self.doc
            .get_changes(have)
            .into_iter()
            .map(|c| c.raw_bytes().to_vec())
            .collect()
    }

    /// Applies changes received from another device.
    ///
    /// Applying the same change twice, or out of order, or interleaved with
    /// local edits, all converge to the same result — which is what makes a
    /// late sync merely late rather than lossy.
    pub fn apply_changes(&mut self, changes: &[Vec<u8>]) -> Result<()> {
        let parsed = changes
            .iter()
            .map(|b| Change::from_bytes(b.clone()))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Crdt(format!("undecodable change: {e}")))?;
        self.doc.apply_changes(parsed)?;
        Ok(())
    }

    pub fn merge(&mut self, other: &mut NoteDoc) -> Result<()> {
        self.doc.merge(&mut other.doc)?;
        Ok(())
    }

    /// Merges a whole saved document, as carried by a snapshot chunk.
    ///
    /// Merging rather than replacing matters: a device reading a snapshot may
    /// already hold edits the snapshot's author had not seen. Loading over the
    /// top would discard them.
    pub fn merge_saved(&mut self, saved: &[u8]) -> Result<()> {
        let mut other = AutoCommit::load(saved)?;
        self.doc.merge(&mut other)?;
        Ok(())
    }

    /// Parses the materialised text into frontmatter and body.
    pub fn note(&self, path: &Path) -> Result<crate::vault::Note> {
        crate::vault::Note::parse(path, &self.text()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const LAPTOP: DeviceId = DeviceId([0xa1; 16]);
    const PHONE: DeviceId = DeviceId([0xb2; 16]);

    #[test]
    fn genesis_is_byte_identical_every_time() {
        // If this ever drifts, independently-created copies of a note stop
        // merging and start silently shadowing each other.
        let a = NoteDoc::new(LAPTOP).unwrap().save();
        let b = NoteDoc::new(PHONE).unwrap().save();
        assert_eq!(a, b, "two devices produced different genesis documents");
    }

    #[test]
    fn two_devices_creating_the_same_note_independently_converge() {
        // Neither device ever saw the other's document — each made the note
        // from scratch, as happens when a note arrives as oplog changes.
        let mut laptop = NoteDoc::new(LAPTOP).unwrap();
        let mut phone = NoteDoc::new(PHONE).unwrap();

        laptop.set_text("written on the laptop\n").unwrap();
        phone.set_text("written on the phone\n").unwrap();

        laptop.merge(&mut phone).unwrap();
        let text = laptop.text().unwrap();

        assert!(text.contains("laptop"), "laptop text vanished: {text:?}");
        assert!(text.contains("phone"), "phone text vanished: {text:?}");
    }

    #[test]
    fn text_round_trips() {
        let mut d = NoteDoc::from_text(LAPTOP, "---\ntitle: hi\n---\nbody\n").unwrap();
        assert_eq!(d.text().unwrap(), "---\ntitle: hi\n---\nbody\n");
        let saved = d.save();
        let reloaded = NoteDoc::load(PHONE, &saved).unwrap();
        assert_eq!(reloaded.text().unwrap(), "---\ntitle: hi\n---\nbody\n");
    }

    #[test]
    fn concurrent_edits_to_different_lines_both_survive() {
        let mut laptop = NoteDoc::from_text(LAPTOP, "one\ntwo\nthree\n").unwrap();
        let mut phone = NoteDoc::load(PHONE, &laptop.save()).unwrap();

        laptop.set_text("one EDITED\ntwo\nthree\n").unwrap();
        phone.set_text("one\ntwo\nthree APPENDED\n").unwrap();

        laptop.merge(&mut phone).unwrap();
        let merged = laptop.text().unwrap();

        assert!(
            merged.contains("one EDITED"),
            "laptop edit lost: {merged:?}"
        );
        assert!(
            merged.contains("three APPENDED"),
            "phone edit lost: {merged:?}"
        );
    }

    #[test]
    fn merging_is_order_independent() {
        let base = NoteDoc::from_text(LAPTOP, "start\n").unwrap().save();

        let build = |first_a: bool| {
            let mut a = NoteDoc::load(LAPTOP, &base).unwrap();
            let mut b = NoteDoc::load(PHONE, &base).unwrap();
            a.set_text("start\nfrom a\n").unwrap();
            b.set_text("from b\nstart\n").unwrap();
            let (mut x, mut y) = if first_a { (a, b) } else { (b, a) };
            x.merge(&mut y).unwrap();
            x.text().unwrap()
        };

        assert_eq!(build(true), build(false));
    }

    #[test]
    fn applying_the_same_change_twice_is_harmless() {
        let mut laptop = NoteDoc::from_text(LAPTOP, "hello\n").unwrap();
        let mut phone = NoteDoc::load(PHONE, &laptop.save()).unwrap();

        let before = phone.heads();
        laptop.set_text("hello world\n").unwrap();
        let changes = laptop.changes_since(&before);

        phone.apply_changes(&changes).unwrap();
        let once = phone.text().unwrap();
        phone.apply_changes(&changes).unwrap();

        assert_eq!(phone.text().unwrap(), once);
        assert_eq!(once, "hello world\n");
    }

    #[test]
    fn an_untouched_note_produces_no_changes() {
        let mut d = NoteDoc::from_text(LAPTOP, "unchanged\n").unwrap();
        let heads = d.heads();
        d.set_text("unchanged\n").unwrap();
        assert!(
            d.changes_since(&heads).is_empty(),
            "saving without editing must not generate a change"
        );
    }

    #[test]
    fn doc_id_normalises_unicode_and_separators() {
        // The same filename as macOS stores it (decomposed) and as Windows or
        // Linux stores it (composed). Treating these as two notes is how a
        // vault fills up with duplicates.
        let nfd = DocId::from_relative_path(Path::new("notes/cafe\u{0301}.md"));
        let nfc = DocId::from_relative_path(Path::new("notes/caf\u{00e9}.md"));
        assert_eq!(nfd, nfc);

        let windows = DocId::from_relative_path(&PathBuf::from("notes").join("a.md"));
        assert_eq!(windows.as_str(), "notes/a.md");
    }

    #[test]
    fn doc_id_keeps_case_distinct() {
        // Folding case would merge two genuinely different files on Linux.
        assert_ne!(
            DocId::from_relative_path(Path::new("Note.md")),
            DocId::from_relative_path(Path::new("note.md"))
        );
    }

    #[test]
    fn thai_filenames_normalise_consistently() {
        let a = DocId::from_relative_path(Path::new("บันทึก/ประชุม.md"));
        let b = DocId::from_relative_path(Path::new("บันทึก/ประชุม.md"));
        assert_eq!(a, b);
        assert_eq!(a.as_str(), "บันทึก/ประชุม.md");
    }
}
