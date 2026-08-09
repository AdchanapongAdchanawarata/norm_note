//! Years of use by people who behave differently.
//!
//! The scripted tests elsewhere check situations someone thought of. This one
//! checks the ones nobody did: several people with different habits, different
//! numbers of devices, different levels of diligence about syncing, editing
//! through the app and behind its back, crashing at unhelpful moments — for as
//! many simulated days as you care to run.
//!
//! # How a failure is detected
//!
//! Modelling what a merge *should* produce would mean reimplementing the merge,
//! and the reimplementation would have the same bugs. Instead every edit leaves
//! a marker that appears nowhere else:
//!
//! ```text
//! <!--m:0417-->
//! ```
//!
//! The property is then simple and merge-agnostic: **once everyone has synced,
//! every marker ever written to a note is present in that note on every
//! device.** A lost edit is a missing marker, wherever it went missing.
//!
//! # Reproducibility
//!
//! Randomness comes from a seeded generator written out in full below, so a
//! failing run can be replayed exactly from its seed. Nothing here reads the
//! clock to make a decision.

use norm_core::oplog::{store::ChunkStore, DeviceId, VaultKey};
use norm_core::workspace::Workspace;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Deterministic randomness
// ---------------------------------------------------------------------------

/// xorshift64*. Small enough to read, good enough to shuffle behaviour, and
/// entirely reproducible from its seed.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    /// True `percent` times out of a hundred.
    fn chance(&mut self, percent: u32) -> bool {
        self.below(100) < percent as usize
    }
}

// ---------------------------------------------------------------------------
// The people
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    /// One long note, appended to most days. Barely uses a second device.
    Journaller,
    /// Constantly creating small, separate notes.
    Zettelkasten,
    /// Captures on a phone, tidies up on a laptop, syncs when they remember.
    MobileCapturer,
    /// A few large documents, edited hard but rarely.
    Researcher,
    /// Lives in a text editor and git. Edits files with the daemon stopped,
    /// restarts it at random, pokes at things.
    Tinkerer,
}

struct Persona {
    name: &'static str,
    style: Style,
    /// Indexes into `World::devices`.
    devices: Vec<usize>,
    /// Chance per day of touching their notes at all.
    active: u32,
    /// Chance of running a sync after editing. Low means long divergence.
    syncs: u32,
    /// Chance the process is killed rather than closed politely.
    crashes: u32,
}

fn cast() -> Vec<Persona> {
    vec![
        Persona {
            name: "nok, keeps a daily journal",
            style: Style::Journaller,
            devices: vec![0],
            active: 85,
            syncs: 70,
            crashes: 2,
        },
        Persona {
            name: "arun, zettelkasten, three devices",
            style: Style::Zettelkasten,
            devices: vec![1, 2, 3],
            active: 60,
            syncs: 55,
            crashes: 5,
        },
        Persona {
            name: "mai, phone first, syncs rarely",
            style: Style::MobileCapturer,
            devices: vec![4, 5],
            active: 70,
            // The iOS case: sync happens when the app is opened, which is not
            // often, so this device is routinely months behind.
            syncs: 20,
            crashes: 8,
        },
        Persona {
            name: "chai, long research documents",
            style: Style::Researcher,
            devices: vec![6, 7],
            active: 25,
            syncs: 80,
            crashes: 1,
        },
        Persona {
            name: "somsak, vim and git, daemon often off",
            style: Style::Tinkerer,
            devices: vec![8],
            active: 75,
            syncs: 45,
            crashes: 20,
        },
    ]
}

// ---------------------------------------------------------------------------
// The world
// ---------------------------------------------------------------------------

struct Device {
    root: PathBuf,
    id: DeviceId,
    /// `None` while the process is "not running".
    ws: Option<Workspace>,
}

struct World {
    _dirs: Vec<tempfile::TempDir>,
    devices: Vec<Device>,
    /// Every marker written, and the note it went into.
    written: BTreeMap<String, BTreeSet<String>>,
    /// Who wrote each marker, and on which device. A failure message that says
    /// "mai lost this, writing from her phone on day 812" is worth chasing; one
    /// that says "device 4" is not.
    author: BTreeMap<String, String>,
    next_marker: usize,
}

fn key() -> VaultKey {
    VaultKey::new([21u8; 32])
}

impl World {
    fn new(device_count: usize) -> Self {
        let dirs: Vec<_> = (0..device_count + 1)
            .map(|_| tempfile::tempdir().unwrap())
            .collect();
        let devices = (0..device_count)
            .map(|i| Device {
                root: dirs[i].path().to_path_buf(),
                id: DeviceId([i as u8 + 1; 16]),
                ws: None,
            })
            .collect();
        World {
            _dirs: dirs,
            devices,
            written: BTreeMap::new(),
            author: BTreeMap::new(),
            next_marker: 0,
        }
    }

    /// Starts the daemon on a device if it is not already running.
    fn running(&mut self, d: usize) -> &mut Workspace {
        if self.devices[d].ws.is_none() {
            let ws = Workspace::open(self.devices[d].id, key(), self.devices[d].root.clone())
                .expect("vault should always reopen");
            self.devices[d].ws = Some(ws);
        }
        self.devices[d].ws.as_mut().expect("just started")
    }

    /// Kills the process. Everything held only in memory is gone; the next
    /// call to `running` starts from whatever is on disk.
    fn crash(&mut self, d: usize) {
        self.devices[d].ws = None;
    }

    fn path(&self, d: usize, note: &str) -> PathBuf {
        let mut p = self.devices[d].root.clone();
        for part in note.split('/') {
            p.push(part);
        }
        p
    }

    fn read(&self, d: usize, note: &str) -> String {
        std::fs::read_to_string(self.path(d, note)).unwrap_or_default()
    }

    /// Appends a uniquely identifiable line, the way a person adds a thought to
    /// a note. Writes the file directly — which is what actually happens, since
    /// people use their own editor.
    fn append(&mut self, d: usize, note: &str, who: &str, day: usize) {
        let marker = format!("<!--m:{:05}-->", self.next_marker);
        self.next_marker += 1;
        self.author
            .insert(marker.clone(), format!("{who}, on device {d}, day {day}"));

        let mut text = self.read(d, note);
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!("a thought {marker}\n"));

        let path = self.path(d, note);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();

        self.written
            .entry(note.to_owned())
            .or_default()
            .insert(marker);
    }

    fn sync(&mut self, d: usize) {
        // Take the store out of `self` so the borrow checker is happy about
        // handing a `&mut Workspace` and a `&ChunkStore` to the same call.
        let cloud_root = self.cloud_root();
        let store = ChunkStore::new(cloud_root);
        let ws = self.running(d);
        ws.cycle(&store).expect("a sync pass should not fail");
    }

    fn compact(&mut self, d: usize) {
        let store = ChunkStore::new(self.cloud_root());
        let ws = self.running(d);
        ws.replica_mut()
            .compact_if_needed(150, Some(&store))
            .expect("compaction should not fail");
    }

    fn cloud_root(&self) -> PathBuf {
        self._dirs.last().unwrap().path().to_path_buf()
    }

    /// Everyone comes online and syncs until nothing more moves.
    fn settle(&mut self) {
        for _ in 0..4 {
            for d in 0..self.devices.len() {
                self.sync(d);
            }
        }
    }

    /// The property the whole simulation exists to check.
    fn assert_nothing_was_lost(&mut self) {
        let expectations: Vec<(String, Vec<String>)> = self
            .written
            .iter()
            .map(|(note, markers)| (note.clone(), markers.iter().cloned().collect()))
            .collect();

        for d in 0..self.devices.len() {
            for (note, markers) in &expectations {
                let on_disk = self.read(d, note);
                for marker in markers {
                    assert!(
                        on_disk.contains(marker),
                        "device {d} is missing {marker} from {note}\n\
                         written by {}\n\
                         (the file there is {} bytes)",
                        self.author
                            .get(marker)
                            .map(String::as_str)
                            .unwrap_or("someone"),
                        on_disk.len()
                    );
                }
            }
        }
    }

    fn assert_no_conflict_files(&self) {
        for d in 0..self.devices.len() {
            for entry in walkdir(&self.devices[d].root) {
                let name = entry.file_name().unwrap_or_default().to_string_lossy();
                assert!(
                    !name.contains("conflict") && !name.contains("(1)"),
                    "device {d} produced {}",
                    entry.display()
                );
            }
        }
    }

    fn chunk_count(&mut self) -> usize {
        let mut total = 0;
        for d in 0..self.devices.len() {
            let ws = self.running(d);
            let store = ws.replica().store();
            for dev in store.devices().unwrap() {
                total += store.list(dev).unwrap().len();
            }
        }
        total
    }
}

fn walkdir(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// A day in each person's life
// ---------------------------------------------------------------------------

fn notes_for(style: Style, day: usize, rng: &mut Rng) -> Vec<String> {
    match style {
        // The same note, for years.
        Style::Journaller => vec!["journal.md".to_owned()],

        // A new note most days, plus a revisit of an older one.
        Style::Zettelkasten => {
            let mut v = vec![format!("zk/{:04}.md", day)];
            if day > 5 {
                v.push(format!("zk/{:04}.md", rng.below(day)));
            }
            v
        }

        // Quick captures, tidied into a smaller set later.
        Style::MobileCapturer => {
            if rng.chance(60) {
                vec![format!("inbox/{:04}.md", day)]
            } else {
                vec![format!("projects/{}.md", rng.below(6))]
            }
        }

        // A handful of long documents.
        Style::Researcher => vec![format!("research/chapter-{}.md", rng.below(4))],

        // Anything, including files in odd places.
        Style::Tinkerer => {
            let n = rng.below(3);
            vec![format!("notes/{n}.md")]
        }
    }
}

fn live_a_day(world: &mut World, people: &mut [Persona], day: usize, rng: &mut Rng) {
    for p in people.iter_mut() {
        if !rng.chance(p.active) {
            continue;
        }

        let d = p.devices[rng.below(p.devices.len())];

        // Someone who has not synced in a while may be working from an old
        // copy on this device. That is the normal state of affairs, not an
        // error, and the merge has to cope with it.
        let edits = match p.style {
            Style::Researcher => 3 + rng.below(6),
            Style::Zettelkasten => 1 + rng.below(3),
            _ => 1 + rng.below(2),
        };

        for note in notes_for(p.style, day, rng) {
            for _ in 0..edits {
                world.append(d, &note, p.name, day);
            }
        }

        if rng.chance(p.crashes) {
            // Killed mid-edit, before anything was folded in.
            world.crash(d);
            continue;
        }

        if rng.chance(p.syncs) {
            world.sync(d);

            // Occasionally another of their devices is on at the same time.
            if p.devices.len() > 1 && rng.chance(35) {
                let other = p.devices[rng.below(p.devices.len())];
                world.sync(other);
            }
        }

        if day % 40 == 0 {
            world.compact(d);
        }
    }
}

fn run_simulation(days: usize, seed: u64) -> World {
    let mut people = cast();
    let device_count = people.iter().flat_map(|p| p.devices.iter()).max().unwrap() + 1;
    let mut world = World::new(device_count);
    let mut rng = Rng::new(seed);

    for day in 0..days {
        live_a_day(&mut world, &mut people, day, &mut rng);
    }

    world.settle();
    world
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

#[test]
fn three_months_of_five_different_people() {
    // Kept to a quarter so it stays inside a normal test run. `five_years`
    // covers the long haul; this is here so a regression shows up on every
    // commit rather than whenever someone remembers to run the slow one.
    let mut world = run_simulation(90, 0x5EED_0001);

    world.assert_nothing_was_lost();
    world.assert_no_conflict_files();
    assert!(
        world.next_marker > 200,
        "the simulation barely did anything: {} edits",
        world.next_marker
    );
}

#[test]
fn the_same_six_months_twice_gives_the_same_result() {
    // A simulation that is not reproducible cannot be used to chase a bug.
    let a = run_simulation(60, 0x5EED_0002);
    let b = run_simulation(60, 0x5EED_0002);
    assert_eq!(a.written, b.written);
    assert_eq!(a.next_marker, b.next_marker);
}

#[test]
fn different_seeds_explore_different_behaviour() {
    let a = run_simulation(60, 0x5EED_0003);
    let b = run_simulation(60, 0x5EED_0099);
    assert_ne!(
        a.written, b.written,
        "the seed is not actually changing anything"
    );
}

/// Reads a `NORM_SIM_*` override, accepting `0x`-prefixed hex.
///
/// One seed explores one path through the possible orderings. A fault that
/// only appears under a different interleaving is invisible until something
/// tries a different seed, so CI runs several.
fn env_u64(name: &str, default: u64) -> u64 {
    let Ok(raw) = std::env::var(name) else {
        return default;
    };
    let raw = raw.trim();
    let parsed = match raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => raw.parse(),
    };
    parsed.unwrap_or_else(|_| panic!("{name}={raw:?} is not a number"))
}

#[test]
#[ignore = "five simulated years; run with --release --ignored --nocapture"]
fn five_years() {
    let days = env_u64("NORM_SIM_DAYS", 365 * 5) as usize;
    let seed = env_u64("NORM_SIM_SEED", 0x5EED_2030);
    println!("\nseed {seed:#x}, {days} days");

    let mut world = run_simulation(days, seed);

    let edits = world.next_marker;
    let notes = world.written.len();
    let chunks = world.chunk_count();

    println!("\n=== {days} days ===");
    println!("edits written      {edits}");
    println!("notes touched      {notes}");
    println!("chunks across all devices {chunks}");

    world.assert_nothing_was_lost();
    world.assert_no_conflict_files();

    // Compaction has to actually keep up. Before every holder of a snapshot
    // was allowed to prune, five years left 25,479 chunks: each device deleted
    // its own and the others pushed them back. The number of chunks must stay
    // related to the number of notes, not to the number of edits ever made.
    assert!(
        chunks < edits / 2,
        "{chunks} chunks for {edits} edits — compaction is not keeping up"
    );

    println!(
        "every one of the {edits} edits is present on all {} devices\n",
        world.devices.len()
    );
}
