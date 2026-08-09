//! Simulated usage, written against the pain points the research turned up.
//!
//! Each test names a real complaint from r/ObsidianMD, the Obsidian forum or
//! Notion reviews, and checks that the design actually answers it. They run the
//! production `Replica` API, not test-only wiring.

use norm_core::doc::DocId;
use norm_core::oplog::{store::ChunkStore, DeviceId, VaultKey};
use norm_core::replica::Replica;
use std::path::Path;

const LAPTOP: DeviceId = DeviceId([0xa1; 16]);
const PHONE: DeviceId = DeviceId([0xb2; 16]);
const DESKTOP: DeviceId = DeviceId([0xc3; 16]);

fn key() -> VaultKey {
    VaultKey::new([77u8; 32])
}

fn note(path: &str) -> DocId {
    DocId::from_relative_path(Path::new(path))
}

/// A laptop, a phone, and the Dropbox folder between them.
struct World {
    dirs: Vec<tempfile::TempDir>,
    laptop: Replica,
    phone: Replica,
    cloud: ChunkStore,
}

fn world() -> World {
    let dirs: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
    World {
        laptop: Replica::open(LAPTOP, key(), dirs[0].path()),
        phone: Replica::open(PHONE, key(), dirs[1].path()),
        cloud: ChunkStore::new(dirs[2].path()),
        dirs,
    }
}

impl World {
    fn text(&self, which: &Replica, path: &str) -> String {
        which.text(&note(path)).unwrap().unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Obsidian complaint #1: sync conflicts producing `(conflicted copy)` files
// ---------------------------------------------------------------------------

#[test]
fn both_devices_edit_the_same_note_while_apart_and_nothing_is_lost() {
    let mut w = world();
    let n = note("meeting.md");

    w.laptop.write(&n, "# Meeting\n\n- agenda item\n").unwrap();
    w.laptop.sync_through(&w.cloud).unwrap();
    w.phone.sync_through(&w.cloud).unwrap();

    // Both go offline and edit. Neither can see the other.
    w.laptop
        .write(&n, "# Meeting\n\n- agenda item\n- laptop added this\n")
        .unwrap();
    w.phone
        .write(&n, "# Meeting\n\n- phone added this\n- agenda item\n")
        .unwrap();

    // Days later, both reconnect.
    w.laptop.sync_through(&w.cloud).unwrap();
    w.phone.sync_through(&w.cloud).unwrap();
    w.laptop.sync_through(&w.cloud).unwrap();

    for (name, r) in [("laptop", &w.laptop), ("phone", &w.phone)] {
        let text = w.text(r, "meeting.md");
        assert!(text.contains("laptop added this"), "{name}: {text:?}");
        assert!(text.contains("phone added this"), "{name}: {text:?}");
        assert!(text.contains("agenda item"), "{name}: {text:?}");
    }

    assert_eq!(
        w.text(&w.laptop, "meeting.md"),
        w.text(&w.phone, "meeting.md"),
        "devices disagree after syncing"
    );
    assert_eq!(w.laptop.notes(), vec![n], "a conflicted copy was created");
}

// ---------------------------------------------------------------------------
// iOS background limits: sync can be hours late. Late must not mean lossy.
// ---------------------------------------------------------------------------

#[test]
fn a_week_of_phone_only_writing_survives_one_late_sync() {
    let mut w = world();
    let n = note("journal.md");

    // The laptop is closed all week. The phone cannot reach it, and iOS will
    // not let a sync daemon run in the background anyway.
    let mut text = String::new();
    for day in 1..=7 {
        text.push_str(&format!("day {day}\n"));
        w.phone.write(&n, &text).unwrap();
    }

    // Sunday: the laptop is opened for the first time in a week.
    w.phone.sync_through(&w.cloud).unwrap();
    let outcome = w.laptop.sync_through(&w.cloud).unwrap();

    assert!(outcome.applied > 0);
    let got = w.text(&w.laptop, "journal.md");
    for day in 1..=7 {
        assert!(got.contains(&format!("day {day}")), "lost day {day}");
    }
}

// ---------------------------------------------------------------------------
// Notion complaint: offline is a degraded mode. Here it is the normal one.
// ---------------------------------------------------------------------------

#[test]
fn a_device_that_never_syncs_still_works_completely() {
    let w = world();
    let mut solo = Replica::open(LAPTOP, key(), w.dirs[0].path());

    for i in 0..20 {
        solo.write(&note(&format!("note-{i}.md")), &format!("content {i}"))
            .unwrap();
    }

    assert_eq!(solo.notes().len(), 20);
    assert_eq!(solo.text(&note("note-7.md")).unwrap().unwrap(), "content 7");
}

// ---------------------------------------------------------------------------
// Restart: everything must be recoverable from disk alone.
// ---------------------------------------------------------------------------

#[test]
fn a_restarted_replica_reaches_exactly_the_same_state() {
    let w = world();
    let root = w.dirs[0].path();

    let mut first = Replica::open(LAPTOP, key(), root);
    first.write(&note("a.md"), "first version\n").unwrap();
    first.write(&note("a.md"), "second version\n").unwrap();
    first.write(&note("b.md"), "other note\n").unwrap();
    let before: Vec<_> = first
        .notes()
        .iter()
        .map(|id| (id.clone(), first.text(id).unwrap().unwrap()))
        .collect();

    // Process dies. Nothing was kept in memory.
    drop(first);

    let mut restarted = Replica::open(LAPTOP, key(), root);
    restarted.absorb().unwrap();
    let after: Vec<_> = restarted
        .notes()
        .iter()
        .map(|id| (id.clone(), restarted.text(id).unwrap().unwrap()))
        .collect();

    assert_eq!(before, after);
}

#[test]
fn a_restarted_replica_does_not_republish_history() {
    let w = world();
    let root = w.dirs[0].path();

    let mut first = Replica::open(LAPTOP, key(), root);
    first.write(&note("a.md"), "one\n").unwrap();
    first.write(&note("a.md"), "two\n").unwrap();
    drop(first);

    let mut restarted = Replica::open(LAPTOP, key(), root);
    restarted.absorb().unwrap();

    // Re-saving the text it already has must not append anything. An editor
    // that autosaves every few seconds would otherwise grow the log forever.
    assert_eq!(restarted.write(&note("a.md"), "two\n").unwrap(), None);
}

// ---------------------------------------------------------------------------
// Autosave noise: writing unchanged text must cost nothing.
// ---------------------------------------------------------------------------

#[test]
fn saving_unchanged_text_appends_no_chunk() {
    let mut w = world();
    let n = note("idle.md");

    assert!(w.laptop.write(&n, "hello\n").unwrap().is_some());
    for _ in 0..50 {
        assert_eq!(
            w.laptop.write(&n, "hello\n").unwrap(),
            None,
            "an unchanged save produced a chunk"
        );
    }
}

#[test]
fn syncing_when_nothing_changed_is_a_no_op() {
    let mut w = world();
    w.laptop.write(&note("a.md"), "x\n").unwrap();
    w.laptop.sync_through(&w.cloud).unwrap();

    assert!(
        w.laptop.sync_through(&w.cloud).unwrap().is_idle(),
        "a second sync did work it did not need to"
    );
}

// ---------------------------------------------------------------------------
// Migration: someone points norm_note at the Obsidian vault they already have.
// ---------------------------------------------------------------------------

fn fake_vault(n: usize) -> Vec<(DocId, String)> {
    (0..n)
        .map(|i| {
            (
                note(&format!("folder-{}/note-{i}.md", i % 20)),
                format!("---\ntags: [imported]\n---\n\n# Note {i}\n\nSome body text.\n"),
            )
        })
        .collect()
}

#[test]
fn importing_a_whole_vault_costs_one_chunk() {
    // One chunk per note would mean one durable write per note. A real vault
    // is thousands of notes, and thousands of fsyncs is the difference between
    // an import that feels instant and one the user assumes has hung.
    let w = world();
    let mut laptop = Replica::open(LAPTOP, key(), w.dirs[0].path());

    let vault = fake_vault(1000);
    let edits: Vec<(DocId, &str)> = vault.iter().map(|(d, t)| (d.clone(), t.as_str())).collect();

    let chunk = laptop.write_many(&edits).unwrap();
    assert!(chunk.is_some());

    let written: usize = laptop
        .store()
        .devices()
        .unwrap()
        .iter()
        .map(|d| laptop.store().list(*d).unwrap().len())
        .sum();
    assert_eq!(written, 1, "the import wrote {written} chunks instead of 1");
    assert_eq!(laptop.notes().len(), 1000);
}

#[test]
fn an_imported_vault_arrives_intact_on_a_second_device() {
    let mut w = world();
    let vault = fake_vault(300);
    let edits: Vec<(DocId, &str)> = vault.iter().map(|(d, t)| (d.clone(), t.as_str())).collect();

    w.laptop.write_many(&edits).unwrap();
    w.laptop.sync_through(&w.cloud).unwrap();
    w.phone.sync_through(&w.cloud).unwrap();

    assert_eq!(w.phone.notes().len(), 300);
    for (id, text) in &vault {
        assert_eq!(
            w.phone.text(id).unwrap().as_deref(),
            Some(text.as_str()),
            "{id} did not survive the trip"
        );
    }
}

#[test]
fn re_importing_an_unchanged_vault_writes_nothing() {
    // Pointing the importer at the same folder twice is something users do.
    // It must not double the log.
    let w = world();
    let mut laptop = Replica::open(LAPTOP, key(), w.dirs[0].path());
    let vault = fake_vault(50);
    let edits: Vec<(DocId, &str)> = vault.iter().map(|(d, t)| (d.clone(), t.as_str())).collect();

    laptop.write_many(&edits).unwrap();
    assert_eq!(
        laptop.write_many(&edits).unwrap(),
        None,
        "a second import of identical content appended a chunk"
    );
}

#[test]
fn a_partly_changed_vault_only_publishes_what_changed() {
    let w = world();
    let mut laptop = Replica::open(LAPTOP, key(), w.dirs[0].path());
    let vault = fake_vault(50);
    let edits: Vec<(DocId, &str)> = vault.iter().map(|(d, t)| (d.clone(), t.as_str())).collect();
    laptop.write_many(&edits).unwrap();

    let mut second = vault.clone();
    second[7].1.push_str("edited later\n");
    let edits: Vec<(DocId, &str)> = second
        .iter()
        .map(|(d, t)| (d.clone(), t.as_str()))
        .collect();

    let chunk = laptop.write_many(&edits).unwrap().expect("should publish");
    let bytes = std::fs::read(laptop.store().chunk_path(chunk)).unwrap();
    assert!(
        bytes.len() < 4096,
        "publishing one small edit produced {} bytes; the delta is not a delta",
        bytes.len()
    );
}

// ---------------------------------------------------------------------------
// Years of use. Every save appends, so without compaction the log only grows.
// ---------------------------------------------------------------------------

fn chunk_count(r: &Replica) -> usize {
    r.store()
        .devices()
        .unwrap()
        .iter()
        .map(|d| r.store().list(*d).unwrap().len())
        .sum()
}

#[test]
fn two_years_of_daily_journalling_stays_bounded() {
    // 700 entries appended to one note, the way someone keeping a daily log
    // would. Uncompacted this was hundreds of chunks and a log twenty times
    // the size of the note it described.
    let mut w = world();
    let n = note("journal.md");
    let mut text = String::new();

    // 250 rather than 700 keeps this inside a normal test run. What is being
    // checked is that the chunk count stops growing, and that shows up just as
    // clearly here — see `tests/scale.rs` for the full-size measurement.
    const DAYS: usize = 250;
    for day in 0..DAYS {
        text.push_str(&format!("## day {day}\n\nsomething that happened.\n\n"));
        w.laptop.write(&n, &text).unwrap();

        if day % 25 == 0 {
            w.laptop.sync_through(&w.cloud).unwrap();
            w.laptop.compact_if_needed(20, Some(&w.cloud)).unwrap();
        }
    }
    w.laptop.sync_through(&w.cloud).unwrap();
    w.laptop.compact_if_needed(20, Some(&w.cloud)).unwrap();

    let chunks = chunk_count(&w.laptop);
    assert!(
        chunks < DAYS / 4,
        "{DAYS} entries left {chunks} chunks; compaction is not keeping up"
    );

    // And nothing was lost along the way.
    let got = w.laptop.text(&n).unwrap().unwrap();
    for day in [0, 1, DAYS / 2, DAYS - 2, DAYS - 1] {
        assert!(got.contains(&format!("## day {day}\n")), "lost day {day}");
    }
}

#[test]
fn a_device_offline_across_a_compaction_still_catches_up() {
    // The dangerous case. The laptop snapshots and deletes the chunks it
    // replaced while the phone is away. If pruning were too eager, the phone
    // would come back to a hole in the history.
    let mut w = world();
    let n = note("shared.md");

    w.laptop.write(&n, "before the phone left\n").unwrap();
    w.laptop.sync_through(&w.cloud).unwrap();
    w.phone.sync_through(&w.cloud).unwrap();

    // The phone is switched off for months.
    let mut text = String::from("before the phone left\n");
    for i in 0..300 {
        text.push_str(&format!("laptop line {i}\n"));
        w.laptop.write(&n, &text).unwrap();
    }
    w.laptop.sync_through(&w.cloud).unwrap();

    let c = w.laptop.compact_if_needed(50, Some(&w.cloud)).unwrap();
    assert!(c.happened(), "no snapshot was taken");
    assert!(c.pruned > 0, "nothing was pruned, so this proves nothing");
    w.laptop.sync_through(&w.cloud).unwrap();

    // The phone comes back.
    w.phone.sync_through(&w.cloud).unwrap();
    let got = w.phone.text(&n).unwrap().unwrap();

    assert!(got.contains("before the phone left"));
    for i in [0, 150, 299] {
        assert!(got.contains(&format!("laptop line {i}\n")), "lost line {i}");
    }
    assert_eq!(got, w.laptop.text(&n).unwrap().unwrap());
}

#[test]
fn a_devices_own_edits_survive_absorbing_someone_elses_snapshot() {
    // A snapshot is merged, not loaded over the top. The phone has edits the
    // laptop never saw; the laptop's snapshot must not erase them.
    let mut w = world();
    let n = note("both.md");

    w.laptop.write(&n, "shared start\n").unwrap();
    w.laptop.sync_through(&w.cloud).unwrap();
    w.phone.sync_through(&w.cloud).unwrap();

    w.phone
        .write(&n, "shared start\nonly the phone knows\n")
        .unwrap();

    let mut text = String::from("shared start\n");
    for i in 0..60 {
        text.push_str(&format!("laptop {i}\n"));
        w.laptop.write(&n, &text).unwrap();
    }
    w.laptop.sync_through(&w.cloud).unwrap();
    w.laptop.compact_if_needed(20, Some(&w.cloud)).unwrap();
    w.laptop.sync_through(&w.cloud).unwrap();

    w.phone.sync_through(&w.cloud).unwrap();
    let got = w.phone.text(&n).unwrap().unwrap();

    assert!(
        got.contains("only the phone knows"),
        "the snapshot overwrote a local edit: {got:?}"
    );
    assert!(got.contains("laptop 59"), "{got:?}");
}

#[test]
fn nothing_is_pruned_until_the_snapshot_is_safely_at_the_target() {
    // Deleting local chunks before their replacement has left the machine
    // would strand every other device.
    let mut w = world();
    let n = note("a.md");
    let mut text = String::new();
    for i in 0..60 {
        text.push_str(&format!("line {i}\n"));
        w.laptop.write(&n, &text).unwrap();
    }

    let before = chunk_count(&w.laptop);
    // Never pushed, so the target has neither the chunks nor the snapshot.
    let c = w.laptop.compact_if_needed(20, None).unwrap();
    assert!(c.happened());

    let unreachable = ChunkStore::new(w.dirs[2].path().join("not-mounted"));
    assert_eq!(
        w.laptop
            .store()
            .prune_superseded(w.laptop.device(), Some(&unreachable))
            .unwrap(),
        0,
        "chunks were deleted while the target did not have the snapshot"
    );
    assert!(chunk_count(&w.laptop) > before - 60);
}

#[test]
fn a_pruned_chunk_is_not_pushed_back_by_another_device() {
    // Found by the five-year simulation: with each device pruning only its own
    // chunks, the laptop would delete its old ones from the target and the
    // phone — still holding copies — would push them straight back on its next
    // sync. Nothing was ever really removed and the log grew for ever.
    let mut w = world();
    let n = note("a.md");

    w.laptop.write(&n, "start\n").unwrap();
    w.laptop.sync_through(&w.cloud).unwrap();
    w.phone.sync_through(&w.cloud).unwrap();

    let mut text = String::from("start\n");
    for i in 0..80 {
        text.push_str(&format!("line {i}\n"));
        w.laptop.write(&n, &text).unwrap();
    }
    w.laptop.sync_through(&w.cloud).unwrap();

    let c = w.laptop.compact_if_needed(30, Some(&w.cloud)).unwrap();
    assert!(c.happened() && c.pruned > 0);

    let after_prune = chunk_count(&w.laptop);

    // The phone still holds every one of those chunks locally.
    for _ in 0..3 {
        w.phone.sync_through(&w.cloud).unwrap();
        w.laptop.sync_through(&w.cloud).unwrap();
    }

    assert!(
        chunk_count(&w.laptop) <= after_prune + 2,
        "pruned chunks came back: {} before, {} after the phone synced",
        after_prune,
        chunk_count(&w.laptop)
    );

    // And nothing was lost in the process.
    let got = w.laptop.text(&n).unwrap().unwrap();
    for i in [0, 40, 79] {
        assert!(got.contains(&format!("line {i}\n")), "lost line {i}");
    }
    assert_eq!(got, w.phone.text(&n).unwrap().unwrap());
}

#[test]
fn compaction_is_skipped_when_the_log_is_short() {
    let mut w = world();
    w.laptop.write(&note("a.md"), "one\n").unwrap();
    let c = w.laptop.compact_if_needed(50, Some(&w.cloud)).unwrap();
    assert!(!c.happened(), "snapshotted a log that did not need it");
}

// ---------------------------------------------------------------------------
// A third device, added months later.
// ---------------------------------------------------------------------------

#[test]
fn a_new_device_catches_up_on_everything() {
    let mut w = world();

    w.laptop
        .write(&note("old.md"), "written long ago\n")
        .unwrap();
    w.laptop.sync_through(&w.cloud).unwrap();
    w.phone.sync_through(&w.cloud).unwrap();
    w.phone
        .write(&note("phone.md"), "from the phone\n")
        .unwrap();
    w.phone.sync_through(&w.cloud).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let mut desktop = Replica::open(DESKTOP, key(), dir.path());
    desktop.sync_through(&w.cloud).unwrap();

    assert_eq!(desktop.notes(), vec![note("old.md"), note("phone.md")]);
    assert_eq!(
        desktop.text(&note("phone.md")).unwrap().unwrap(),
        "from the phone\n"
    );
}

// ---------------------------------------------------------------------------
// The sync target is untrusted storage.
// ---------------------------------------------------------------------------

#[test]
fn the_sync_folder_never_holds_readable_notes() {
    let mut w = world();
    w.laptop
        .write(&note("secret.md"), "salary negotiation notes\n")
        .unwrap();
    w.laptop.sync_through(&w.cloud).unwrap();

    let mut found_any = false;
    for device in w.cloud.devices().unwrap() {
        for chunk in w.cloud.list(device).unwrap() {
            found_any = true;
            let raw = std::fs::read(w.cloud.chunk_path(chunk)).unwrap();
            assert!(
                !raw.windows(6).any(|win| win == b"salary"),
                "plaintext reached the sync folder"
            );
            assert!(
                !raw.windows(6).any(|win| win == b"secret"),
                "the note's filename reached the sync folder in the clear"
            );
        }
    }
    assert!(found_any, "nothing was pushed, so nothing was proven");
}

// ---------------------------------------------------------------------------
// Non-Latin content, which is where naive text handling usually breaks.
// ---------------------------------------------------------------------------

#[test]
fn thai_content_edited_on_two_devices_merges_intact() {
    let mut w = world();
    let n = note("บันทึก/ประชุม.md");

    w.laptop.write(&n, "# ประชุมทีม\n\n- วาระที่หนึ่ง\n").unwrap();
    w.laptop.sync_through(&w.cloud).unwrap();
    w.phone.sync_through(&w.cloud).unwrap();

    w.laptop
        .write(&n, "# ประชุมทีม\n\n- วาระที่หนึ่ง\n- เพิ่มจากโน้ตบุ๊ก\n")
        .unwrap();
    w.phone
        .write(&n, "# ประชุมทีม\n\n- เพิ่มจากมือถือ\n- วาระที่หนึ่ง\n")
        .unwrap();

    w.laptop.sync_through(&w.cloud).unwrap();
    w.phone.sync_through(&w.cloud).unwrap();
    w.laptop.sync_through(&w.cloud).unwrap();

    let text = w.text(&w.laptop, "บันทึก/ประชุม.md");
    assert!(text.contains("เพิ่มจากโน้ตบุ๊ก"), "{text:?}");
    assert!(text.contains("เพิ่มจากมือถือ"), "{text:?}");
    assert!(text.contains("วาระที่หนึ่ง"), "{text:?}");
    assert_eq!(text, w.text(&w.phone, "บันทึก/ประชุม.md"));
}

// ---------------------------------------------------------------------------
// Sustained realistic use across three devices.
// ---------------------------------------------------------------------------

#[test]
fn thirty_days_of_three_device_use_stays_consistent() {
    let dirs: Vec<_> = (0..4).map(|_| tempfile::tempdir().unwrap()).collect();
    let cloud = ChunkStore::new(dirs[3].path());
    let mut replicas = [
        Replica::open(LAPTOP, key(), dirs[0].path()),
        Replica::open(PHONE, key(), dirs[1].path()),
        Replica::open(DESKTOP, key(), dirs[2].path()),
    ];

    let daily = note("daily.md");
    let mut expected_lines = Vec::new();

    for day in 0..30 {
        // A different device is used each day, and it has not necessarily
        // synced since the last time it was touched.
        let who = day % 3;
        replicas[who].sync_through(&cloud).unwrap();

        let current = replicas[who].text(&daily).unwrap().unwrap_or_default();
        let line = format!("day {day} on device {who}\n");
        expected_lines.push(line.clone());
        replicas[who]
            .write(&daily, &format!("{current}{line}"))
            .unwrap();

        // Devices sync at irregular intervals, not every day.
        if day % 4 == 0 {
            replicas[who].sync_through(&cloud).unwrap();
        }
    }

    // Everyone comes online. Two passes: push everything, then pull everything.
    for _ in 0..2 {
        for r in replicas.iter_mut() {
            r.sync_through(&cloud).unwrap();
        }
    }

    let reference = replicas[0].text(&daily).unwrap().unwrap();
    for (i, r) in replicas.iter().enumerate() {
        assert_eq!(
            r.text(&daily).unwrap().unwrap(),
            reference,
            "device {i} diverged"
        );
    }
    for line in &expected_lines {
        assert!(
            reference.contains(line.trim_end()),
            "lost {line:?} after 30 days"
        );
    }
}
