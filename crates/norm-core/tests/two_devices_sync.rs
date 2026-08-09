//! The claim this project is built on, as an executable test:
//!
//! > Put your vault in Dropbox and never get a `(conflicted copy)` again.
//!
//! Two devices, one shared folder, no server and no coordination. The folder
//! is "dumb" — it moves files and nothing else, exactly like Dropbox, iCloud
//! Drive or a NAS mount. Everything here goes through the same public API a
//! real sync loop would use.

use norm_core::oplog::{store::ChunkStore, ChunkId, ChunkKind, DeviceId, VaultKey};

fn key() -> VaultKey {
    VaultKey::new([9u8; 32])
}

const LAPTOP: DeviceId = DeviceId([0xa1; 16]);
const PHONE: DeviceId = DeviceId([0xb2; 16]);

/// A vault, plus the shared folder it syncs through.
struct Fixture {
    _dirs: Vec<tempfile::TempDir>,
    laptop: ChunkStore,
    phone: ChunkStore,
    /// Stands in for a Dropbox folder: it holds ciphertext and never sees a key.
    cloud: ChunkStore,
}

fn fixture() -> Fixture {
    let dirs: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
    Fixture {
        laptop: ChunkStore::new(dirs[0].path()),
        phone: ChunkStore::new(dirs[1].path()),
        cloud: ChunkStore::new(dirs[2].path()),
        _dirs: dirs,
    }
}

fn payloads(store: &ChunkStore, ids: &[ChunkId]) -> Vec<Vec<u8>> {
    ids.iter()
        .map(|id| store.read(&key(), *id).unwrap())
        .collect()
}

fn everything(store: &ChunkStore) -> Vec<ChunkId> {
    let mut all: Vec<_> = store
        .devices()
        .unwrap()
        .into_iter()
        .flat_map(|d| store.list(d).unwrap())
        .collect();
    all.sort();
    all
}

#[test]
fn an_edit_on_one_device_reaches_the_other_through_a_dumb_folder() {
    let f = fixture();

    let id = f
        .laptop
        .append(&key(), LAPTOP, ChunkKind::Op, b"written on the laptop")
        .unwrap();

    // Push, then pull. Neither device ever talks to the other.
    f.cloud.replicate_from(&f.laptop).unwrap();
    f.phone.replicate_from(&f.cloud).unwrap();

    assert_eq!(f.phone.read(&key(), id).unwrap(), b"written on the laptop");
}

#[test]
fn the_relay_never_needs_the_key() {
    let f = fixture();
    f.laptop
        .append(&key(), LAPTOP, ChunkKind::Op, b"private")
        .unwrap();
    f.cloud.replicate_from(&f.laptop).unwrap();

    let id = everything(&f.cloud)[0];
    let on_disk = std::fs::read(f.cloud.chunk_path(id)).unwrap();

    assert!(
        !on_disk.windows(7).any(|w| w == b"private"),
        "plaintext reached the sync target"
    );
    assert!(f.cloud.read(&VaultKey::new([0u8; 32]), id).is_err());
}

#[test]
fn concurrent_edits_on_both_devices_both_survive() {
    // The scenario that produces `(conflicted copy)` files in Obsidian: both
    // devices write while neither can see the other.
    let f = fixture();

    let a = f
        .laptop
        .append(&key(), LAPTOP, ChunkKind::Op, b"laptop edit")
        .unwrap();
    let b = f
        .phone
        .append(&key(), PHONE, ChunkKind::Op, b"phone edit")
        .unwrap();

    // Both push to the same folder, then both pull. Order is irrelevant.
    f.cloud.replicate_from(&f.laptop).unwrap();
    f.cloud.replicate_from(&f.phone).unwrap();
    f.laptop.replicate_from(&f.cloud).unwrap();
    f.phone.replicate_from(&f.cloud).unwrap();

    for (name, store) in [
        ("laptop", &f.laptop),
        ("phone", &f.phone),
        ("cloud", &f.cloud),
    ] {
        assert_eq!(everything(store), vec![a, b], "{name} is missing a chunk");
    }

    assert_eq!(
        payloads(&f.laptop, &[a, b]),
        vec![b"laptop edit".to_vec(), b"phone edit".to_vec()]
    );
    assert_eq!(
        payloads(&f.phone, &[a, b]),
        vec![b"laptop edit".to_vec(), b"phone edit".to_vec()]
    );
}

#[test]
fn both_devices_using_the_same_sequence_number_is_not_a_collision() {
    // Both devices are at seq 0. In a path-per-note design this is the exact
    // moment the cloud has to pick a winner. Here the paths differ, so there
    // is nothing to pick.
    let f = fixture();

    let a = f
        .laptop
        .append(&key(), LAPTOP, ChunkKind::Op, b"from laptop")
        .unwrap();
    let b = f
        .phone
        .append(&key(), PHONE, ChunkKind::Op, b"from phone")
        .unwrap();
    assert_eq!((a.seq, b.seq), (0, 0));
    assert_ne!(a.relative_path(), b.relative_path());

    f.cloud.replicate_from(&f.laptop).unwrap();
    f.cloud.replicate_from(&f.phone).unwrap();

    assert_eq!(everything(&f.cloud).len(), 2);
}

#[test]
fn replication_is_idempotent_and_order_independent() {
    let f = fixture();
    f.laptop
        .append(&key(), LAPTOP, ChunkKind::Op, b"one")
        .unwrap();
    f.laptop
        .append(&key(), LAPTOP, ChunkKind::Op, b"two")
        .unwrap();

    assert_eq!(f.cloud.replicate_from(&f.laptop).unwrap().len(), 2);
    assert_eq!(
        f.cloud.replicate_from(&f.laptop).unwrap().len(),
        0,
        "a second pass must copy nothing"
    );

    // Pulling back into the source changes nothing either.
    assert_eq!(f.laptop.replicate_from(&f.cloud).unwrap().len(), 0);
    assert_eq!(everything(&f.laptop), everything(&f.cloud));
}

#[test]
fn a_half_finished_sync_resumes_without_loss() {
    // Interrupted mid-push: the folder has some chunks, not all. Nothing is
    // corrupt and the next attempt simply continues.
    let f = fixture();
    for i in 0..5u8 {
        f.laptop
            .append(&key(), LAPTOP, ChunkKind::Op, &[i])
            .unwrap();
    }

    let partial = f.cloud.replicate_from(&f.laptop).unwrap();
    assert_eq!(partial.len(), 5);

    // Simulate the interruption by deleting the tail from the target.
    for id in &partial[3..] {
        std::fs::remove_file(f.cloud.chunk_path(*id)).unwrap();
    }
    assert_eq!(everything(&f.cloud).len(), 3);

    assert_eq!(f.cloud.replicate_from(&f.laptop).unwrap().len(), 2);
    assert_eq!(everything(&f.cloud).len(), 5);
}

#[test]
fn replicating_does_not_advance_the_local_sequence_counter() {
    // A device allocates sequence numbers only for itself. If replication moved
    // the local counter, this device would start issuing numbers another device
    // has already used under the same key — nonce reuse by another route.
    let f = fixture();

    for _ in 0..4 {
        f.laptop
            .append(&key(), LAPTOP, ChunkKind::Op, b"x")
            .unwrap();
    }
    f.phone.replicate_from(&f.laptop).unwrap();

    let next = f
        .phone
        .append(&key(), PHONE, ChunkKind::Op, b"first phone note")
        .unwrap();
    assert_eq!(next.seq, 0, "the phone's own sequence must be untouched");
    assert_eq!(f.phone.list(LAPTOP).unwrap().len(), 4);
    assert_eq!(f.phone.list(PHONE).unwrap().len(), 1);
}

#[test]
fn a_third_device_joining_late_catches_up_completely() {
    let f = fixture();
    let desktop = tempfile::tempdir().unwrap();
    let desktop = ChunkStore::new(desktop.path());

    f.laptop
        .append(&key(), LAPTOP, ChunkKind::Op, b"old note")
        .unwrap();
    f.phone
        .append(&key(), PHONE, ChunkKind::Op, b"another old note")
        .unwrap();
    f.cloud.replicate_from(&f.laptop).unwrap();
    f.cloud.replicate_from(&f.phone).unwrap();

    desktop.replicate_from(&f.cloud).unwrap();

    assert_eq!(everything(&desktop), everything(&f.cloud));
    assert_eq!(desktop.devices().unwrap(), vec![LAPTOP, PHONE]);
}
