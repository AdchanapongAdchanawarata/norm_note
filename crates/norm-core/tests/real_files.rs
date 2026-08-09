//! Tests against real files on disk, which is where G3 either holds or does
//! not: *delete the app and every note still opens in Notepad; edit one in vim
//! while nothing is running and it merges correctly.*

use norm_core::doc::DocId;
use norm_core::oplog::{store::ChunkStore, DeviceId, VaultKey};
use norm_core::workspace::Workspace;
use std::fs;
use std::path::Path;

const LAPTOP: DeviceId = DeviceId([0xa1; 16]);
const PHONE: DeviceId = DeviceId([0xb2; 16]);

fn key() -> VaultKey {
    VaultKey::new([5u8; 32])
}

fn note(p: &str) -> DocId {
    DocId::from_relative_path(Path::new(p))
}

/// Writes a file the way an outside editor would: straight to disk, with the
/// app none the wiser.
fn edit_externally(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn read_file(root: &Path, rel: &str) -> Option<String> {
    fs::read_to_string(root.join(rel)).ok()
}

#[test]
fn a_note_created_on_one_device_becomes_a_real_file_on_the_other() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    edit_externally(a.path(), "hello.md", "# Hello\n\nfrom the laptop\n");

    let mut laptop = Workspace::open(LAPTOP, key(), a.path()).unwrap();
    laptop.cycle(&cloud).unwrap();

    let mut phone = Workspace::open(PHONE, key(), b.path()).unwrap();
    phone.cycle(&cloud).unwrap();

    assert_eq!(
        read_file(b.path(), "hello.md").as_deref(),
        Some("# Hello\n\nfrom the laptop\n")
    );
}

#[test]
fn a_file_edited_while_nothing_was_running_is_picked_up() {
    let dir = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    edit_externally(dir.path(), "n.md", "original\n");
    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    ws.cycle(&cloud).unwrap();
    drop(ws);

    // The daemon is stopped. The user opens the file in vim.
    edit_externally(dir.path(), "n.md", "original\nadded in vim\n");

    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    let out = ws.cycle(&cloud).unwrap();

    assert_eq!(out.scan.external_edits, vec![note("n.md")]);
    assert_eq!(
        ws.replica().text(&note("n.md")).unwrap().as_deref(),
        Some("original\nadded in vim\n")
    );
}

#[test]
fn restarting_does_not_mistake_our_own_files_for_user_edits() {
    // The persisted record of what we wrote is what makes this possible. If it
    // were lost, every restart would republish the whole vault.
    let dir = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    edit_externally(dir.path(), "a.md", "one\n");
    edit_externally(dir.path(), "sub/b.md", "two\n");
    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    ws.cycle(&cloud).unwrap();
    drop(ws);

    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    let out = ws.cycle(&cloud).unwrap();

    assert!(
        out.scan.is_empty(),
        "a restart reported work to do: {:?}",
        out.scan
    );
    assert!(out.is_idle(), "a restart was not a no-op: {out:?}");
}

#[test]
fn an_untouched_file_is_left_byte_identical() {
    // G3: syncing a note the user did not edit must not rewrite their file.
    let dir = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    let original = "---\nb: 2\na: 1\n# a comment\n---\nbody\n";
    edit_externally(dir.path(), "keep.md", original);

    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    for _ in 0..3 {
        ws.cycle(&cloud).unwrap();
    }

    assert_eq!(read_file(dir.path(), "keep.md").as_deref(), Some(original));
}

#[test]
fn a_remote_edit_and_a_local_external_edit_both_survive() {
    // The ordering test. The laptop has an unmerged edit sitting in a file
    // while a change from the phone is waiting in the oplog. Handling these in
    // the wrong order silently deletes one of them.
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    edit_externally(a.path(), "shared.md", "line one\nline two\n");
    let mut laptop = Workspace::open(LAPTOP, key(), a.path()).unwrap();
    laptop.cycle(&cloud).unwrap();

    let mut phone = Workspace::open(PHONE, key(), b.path()).unwrap();
    phone.cycle(&cloud).unwrap();

    // Phone edits and publishes. The laptop has not seen it yet.
    edit_externally(b.path(), "shared.md", "line one\nline two\nfrom phone\n");
    phone.cycle(&cloud).unwrap();

    // Meanwhile the laptop's file was edited outside the app.
    edit_externally(a.path(), "shared.md", "from laptop\nline one\nline two\n");

    laptop.cycle(&cloud).unwrap();
    laptop.cycle(&cloud).unwrap();

    let text = read_file(a.path(), "shared.md").unwrap();
    assert!(text.contains("from laptop"), "laptop edit lost: {text:?}");
    assert!(text.contains("from phone"), "phone edit lost: {text:?}");
    assert!(text.contains("line one"), "{text:?}");
}

#[test]
fn hidden_directories_are_never_touched() {
    // `.obsidian` and `.git` are not the user's notes and must not be swept up.
    let dir = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    edit_externally(dir.path(), "real.md", "a note\n");
    edit_externally(dir.path(), ".obsidian/plugin.md", "not a note\n");
    edit_externally(dir.path(), ".git/COMMIT_EDITMSG.md", "not a note\n");

    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    ws.cycle(&cloud).unwrap();

    assert_eq!(ws.replica().notes(), vec![note("real.md")]);
    assert_eq!(
        read_file(dir.path(), ".obsidian/plugin.md").as_deref(),
        Some("not a note\n")
    );
}

#[test]
fn non_markdown_files_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    edit_externally(dir.path(), "note.md", "a note\n");
    edit_externally(dir.path(), "attachments/data.csv", "1,2,3\n");

    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    ws.cycle(&cloud).unwrap();

    assert_eq!(ws.replica().notes(), vec![note("note.md")]);
}

#[test]
fn a_deleted_file_is_restored_rather_than_propagated() {
    // Documented behaviour, not an accident: v0.1 has no delete operation, and
    // guessing that a missing file means "delete everywhere" is unrecoverable
    // when the guess is wrong. This test exists so the decision cannot be
    // changed by accident.
    let dir = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    edit_externally(dir.path(), "n.md", "valuable\n");
    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    ws.cycle(&cloud).unwrap();

    fs::remove_file(dir.path().join("n.md")).unwrap();
    ws.cycle(&cloud).unwrap();

    assert_eq!(read_file(dir.path(), "n.md").as_deref(), Some("valuable\n"));
}

#[test]
fn losing_our_state_file_rescues_rather_than_overwrites() {
    // If `.norm/state` is lost, we cannot tell which side is newer. Neither
    // version may be discarded.
    let dir = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    edit_externally(dir.path(), "n.md", "the synced version\n");
    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    ws.cycle(&cloud).unwrap();
    drop(ws);

    fs::remove_dir_all(dir.path().join(".norm").join("state")).unwrap();
    edit_externally(dir.path(), "n.md", "what the user has now\n");

    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    let out = ws.cycle(&cloud).unwrap();

    assert_eq!(out.scan.rescued, vec![note("n.md")]);
    assert_eq!(
        read_file(dir.path(), "n.md").as_deref(),
        Some("what the user has now\n"),
        "the user's file must be the one that stays"
    );

    let rescued: Vec<_> = fs::read_dir(dir.path().join(".norm").join("rescued"))
        .unwrap()
        .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    assert!(
        rescued.iter().any(|r| r.contains("the synced version")),
        "the other version was not kept anywhere: {rescued:?}"
    );
}

#[test]
fn a_crash_mid_write_repairs_the_file_instead_of_publishing_the_damage() {
    // Note files are written without fsync, so a power cut can leave one torn.
    // The journal is what stops the next pass from reading that damage as
    // something the user typed and pushing it to every other device.
    let dir = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    edit_externally(dir.path(), "n.md", "the real content\n");
    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    ws.cycle(&cloud).unwrap();
    drop(ws);

    // Reproduce what a crash during `push_to_disk` leaves behind: the journal
    // says a write was in progress, and the file itself is garbage.
    let state = dir.path().join(".norm").join("state");
    let intended = blake3::hash(b"the real content\n").to_hex().to_string();
    fs::write(state.join("inflight"), format!("{intended} 0 0 n.md\n")).unwrap();
    fs::write(dir.path().join("n.md"), "\0\0\0 truncated garbage").unwrap();

    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();

    assert_eq!(
        read_file(dir.path(), "n.md").as_deref(),
        Some("the real content\n"),
        "the damaged file was not repaired"
    );

    let out = ws.cycle(&cloud).unwrap();
    assert!(
        out.scan.is_empty(),
        "the damage was treated as a user edit: {:?}",
        out.scan
    );
    assert!(
        !dir.path()
            .join(".norm")
            .join("state")
            .join("inflight")
            .exists(),
        "the journal was not cleared after recovery"
    );
}

#[test]
fn a_journal_left_by_a_write_that_actually_landed_is_harmless() {
    // The other half: the crash happened after the file was written. There is
    // nothing to repair and nothing should be republished.
    let dir = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    edit_externally(dir.path(), "n.md", "content\n");
    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    ws.cycle(&cloud).unwrap();
    drop(ws);

    let intended = blake3::hash(b"content\n").to_hex().to_string();
    fs::write(
        dir.path().join(".norm").join("state").join("inflight"),
        format!("{intended} 0 0 n.md\n"),
    )
    .unwrap();

    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    let out = ws.cycle(&cloud).unwrap();

    assert!(out.is_idle(), "recovery did unnecessary work: {out:?}");
    assert_eq!(read_file(dir.path(), "n.md").as_deref(), Some("content\n"));
}

#[test]
fn a_corrupt_recovery_journal_is_refused_rather_than_ignored() {
    // Skipping a line it cannot parse would mean a torn file goes unrepaired
    // and is then published as though the user had typed it. Failing loudly is
    // the only safe reading of a journal that does not make sense.
    let dir = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    edit_externally(dir.path(), "n.md", "content\n");
    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    ws.cycle(&cloud).unwrap();
    drop(ws);

    fs::write(
        dir.path().join(".norm").join("state").join("inflight"),
        "this is not a journal line\n",
    )
    .unwrap();

    assert!(
        Workspace::open(LAPTOP, key(), dir.path()).is_err(),
        "a journal that could not be parsed was quietly ignored"
    );
}

#[test]
fn an_unreadable_state_file_does_not_stop_the_vault_opening() {
    // The other side of the same coin. Losing what we knew about the files is
    // survivable — everything just looks unfamiliar and the rescue path keeps
    // both versions — so it must not lock the user out of their notes.
    let dir = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    edit_externally(dir.path(), "n.md", "content\n");
    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    ws.cycle(&cloud).unwrap();
    drop(ws);

    fs::write(
        dir.path().join(".norm").join("state").join("materialized"),
        "garbage that is not a state file\n",
    )
    .unwrap();

    let ws = Workspace::open(LAPTOP, key(), dir.path()).expect("vault refused to open");
    assert_eq!(ws.replica().notes().len(), 1);
    assert_eq!(read_file(dir.path(), "n.md").as_deref(), Some("content\n"));
}

#[test]
fn thai_filenames_round_trip_through_the_filesystem() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    edit_externally(a.path(), "บันทึก/ประชุมทีม.md", "# วาระ\n\n- ข้อหนึ่ง\n");

    let mut laptop = Workspace::open(LAPTOP, key(), a.path()).unwrap();
    laptop.cycle(&cloud).unwrap();
    let mut phone = Workspace::open(PHONE, key(), b.path()).unwrap();
    phone.cycle(&cloud).unwrap();

    assert_eq!(
        read_file(b.path(), "บันทึก/ประชุมทีม.md").as_deref(),
        Some("# วาระ\n\n- ข้อหนึ่ง\n")
    );
}

#[test]
fn a_vault_of_files_survives_deleting_the_whole_norm_directory() {
    // G3: everything under `.norm/` is derived. Losing it must cost history,
    // never a note.
    let dir = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    for i in 0..10 {
        edit_externally(dir.path(), &format!("n{i}.md"), &format!("note {i}\n"));
    }
    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    ws.cycle(&cloud).unwrap();
    drop(ws);

    fs::remove_dir_all(dir.path().join(".norm")).unwrap();

    let mut ws = Workspace::open(LAPTOP, key(), dir.path()).unwrap();
    ws.cycle(&cloud).unwrap();

    assert_eq!(ws.replica().notes().len(), 10);
    for i in 0..10 {
        assert_eq!(
            read_file(dir.path(), &format!("n{i}.md")).as_deref(),
            Some(format!("note {i}\n").as_str())
        );
    }
}
