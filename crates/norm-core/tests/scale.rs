//! Measurements on a vault the size of a real one.
//!
//! These are `#[ignore]`d: they print numbers rather than assert them, because
//! a wall-clock assertion on shared CI hardware fails for reasons that have
//! nothing to do with the code. Run them deliberately, in release:
//!
//! ```text
//! cargo test --release --test scale -- --ignored --nocapture
//! ```
//!
//! The point is to find out which of the known inefficiencies actually hurts
//! before spending time on any of them.

use norm_core::doc::DocId;
use norm_core::oplog::{store::ChunkStore, DeviceId, VaultKey};
use norm_core::workspace::Workspace;
use std::path::Path;
use std::time::Instant;

const LAPTOP: DeviceId = DeviceId([0xa1; 16]);
const PHONE: DeviceId = DeviceId([0xb2; 16]);
const NOTES: usize = 5_000;

fn key() -> VaultKey {
    VaultKey::new([3u8; 32])
}

fn body(i: usize) -> String {
    format!(
        "---\ntags: [note]\nindex: {i}\n---\n\n# Note {i}\n\n\
         Some ordinary paragraph of text, about as long as a real note's \
         opening line tends to be.\n\n- a list item\n- another one\n"
    )
}

/// Lays down a vault of plain Markdown files, as if the user already had one.
fn seed_vault(root: &Path, n: usize) {
    for i in 0..n {
        let dir = root.join(format!("folder-{}", i % 25));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("note-{i}.md")), body(i)).unwrap();
    }
}

fn dir_size(path: &Path) -> u64 {
    walkdir_size(path)
}

fn walkdir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                total += walkdir_size(&p);
            } else if let Ok(m) = p.metadata() {
                total += m.len();
            }
        }
    }
    total
}

fn ms(start: Instant) -> u128 {
    start.elapsed().as_millis()
}

#[test]
#[ignore = "measurement, not an assertion; run with --ignored --nocapture"]
fn a_real_sized_vault_end_to_end() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());

    println!("\n=== vault of {NOTES} notes ===\n");

    let t = Instant::now();
    seed_vault(a.path(), NOTES);
    println!("seed vault on disk                     {:>7} ms", ms(t));

    // 1. First run: the whole vault is new and must be folded in.
    let t = Instant::now();
    let mut laptop = Workspace::open(LAPTOP, key(), a.path()).unwrap();
    let scan = laptop.pull_from_disk().unwrap();
    println!(
        "first import ({} notes)             {:>7} ms",
        scan.new_notes.len(),
        ms(t)
    );

    // 2. Steady state: nothing changed. This is what `watch` does every few
    //    seconds, so it is the number that matters most.
    let t = Instant::now();
    let scan = laptop.pull_from_disk().unwrap();
    println!("idle pull_from_disk                    {:>7} ms", ms(t));
    assert!(scan.is_empty());

    let t = Instant::now();
    let written = laptop.push_to_disk().unwrap();
    println!(
        "idle push_to_disk ({written} written)          {:>7} ms",
        ms(t)
    );

    // 3. Publishing to the target.
    let t = Instant::now();
    let out = laptop.cycle(&cloud).unwrap();
    println!(
        "first push to target ({} chunks)         {:>7} ms",
        out.sync.pushed,
        ms(t)
    );

    // 4. A second device catching up from nothing, broken down so the cost
    //    lands on the right stage instead of being guessed at.
    let t_all = Instant::now();
    let mut phone = Workspace::open(PHONE, key(), b.path()).unwrap();

    let t = Instant::now();
    let sync = phone.replica_mut().sync_through(&cloud).unwrap();
    println!(
        "  catch-up: replicate + absorb ({} chunks) {:>5} ms",
        sync.pulled,
        ms(t)
    );

    let t = Instant::now();
    let written = phone.push_to_disk().unwrap();
    println!(
        "  catch-up: write {written} files             {:>7} ms",
        ms(t)
    );

    println!("second device full catch-up            {:>7} ms", ms(t_all));
    assert_eq!(phone.replica().notes().len(), NOTES);

    // 5. Restart cost: `applied` is not persisted, so the whole log is re-read.
    let t = Instant::now();
    let reopened = Workspace::open(LAPTOP, key(), a.path()).unwrap();
    println!("restart (re-reads the whole log)       {:>7} ms", ms(t));
    assert_eq!(reopened.replica().notes().len(), NOTES);

    // 6. One small edit in a large vault — the most common action there is.
    let t = Instant::now();
    std::fs::write(
        a.path().join("folder-3").join("note-3.md"),
        format!("{}\nan edit\n", body(3)),
    )
    .unwrap();
    let out = laptop.cycle(&cloud).unwrap();
    println!(
        "one edit, full cycle ({} in / {} out)     {:>7} ms",
        out.sync.pulled,
        out.sync.pushed,
        ms(t)
    );

    println!();
    println!(
        "notes on disk                          {:>7} KB",
        dir_size(a.path()) / 1024
    );
    println!(
        "oplog in .norm/sync                    {:>7} KB",
        dir_size(&a.path().join(".norm").join("sync")) / 1024
    );
    println!(
        "target folder                          {:>7} KB",
        dir_size(c.path()) / 1024
    );
    println!();
}

#[test]
#[ignore = "measurement, not an assertion; run with --ignored --nocapture"]
fn many_small_edits_grow_the_log() {
    // Every save appends. Without compaction the log only grows, and this
    // measures how fast.
    let a = tempfile::tempdir().unwrap();
    let mut ws = Workspace::open(LAPTOP, key(), a.path()).unwrap();
    let id = DocId::from_relative_path(Path::new("journal.md"));

    println!("\n=== 2,000 saves to one note ===\n");
    let mut text = String::new();
    let t = Instant::now();
    for i in 0..2_000 {
        text.push_str(&format!("line {i}\n"));
        std::fs::write(a.path().join("journal.md"), &text).unwrap();
        ws.pull_from_disk().unwrap();
    }
    println!("2,000 saves                            {:>7} ms", ms(t));

    let report = |label: &str, root: &Path| {
        let sync = dir_size(&root.join(".norm").join("sync")) / 1024;
        let note = std::fs::metadata(root.join("journal.md")).unwrap().len() / 1024;
        println!(
            "{label:<38} {sync:>7} KB  ({}x the note)",
            sync / note.max(1)
        );
    };
    report("oplog, uncompacted", a.path());

    let t = Instant::now();
    let reopened = Workspace::open(LAPTOP, key(), a.path()).unwrap();
    println!("restart, uncompacted                   {:>7} ms", ms(t));
    assert!(reopened.replica().text(&id).unwrap().is_some());

    // Now collapse it and measure the same two things again.
    let c = tempfile::tempdir().unwrap();
    let cloud = ChunkStore::new(c.path());
    ws.replica_mut().sync_through(&cloud).unwrap();
    let t = Instant::now();
    let done = ws
        .replica_mut()
        .compact_if_needed(50, Some(&cloud))
        .unwrap();
    println!(
        "compaction ({} chunks pruned)         {:>7} ms",
        done.pruned,
        ms(t)
    );

    report("oplog, compacted", a.path());

    let t = Instant::now();
    let reopened = Workspace::open(LAPTOP, key(), a.path()).unwrap();
    println!("restart, compacted                     {:>7} ms", ms(t));
    assert_eq!(
        reopened.replica().text(&id).unwrap(),
        ws.replica().text(&id).unwrap(),
        "compaction changed what the note says"
    );
    println!();
}
