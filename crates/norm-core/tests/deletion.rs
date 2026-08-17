use norm_core::doc::DocId;
use norm_core::oplog::{DeviceId, VaultKey};
use norm_core::workspace::Workspace;
use std::path::Path;

fn setup(name: &str) -> (tempfile::TempDir, Workspace) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(name);
    std::fs::create_dir_all(&root).unwrap();

    let key = VaultKey::new([42u8; 32]);
    let device = DeviceId::from_hex("00000000000000000000000000000001").unwrap();
    let ws = Workspace::open(device, key, &root).unwrap();
    (dir, ws)
}

#[test]
fn test_deletion_and_trash() {
    let (_dir, mut ws) = setup("test_deletion");
    let doc_id = DocId::from_relative_path(Path::new("test.md"));

    // 1. Create a note
    let text = "Hello world";
    ws.vault().write(&doc_id, text).unwrap();

    // 2. Sync loop to absorb it
    ws.pull_from_disk().unwrap();
    ws.push_to_disk().unwrap();

    // Verify it exists in CRDT
    assert_eq!(ws.replica().text(&doc_id).unwrap().unwrap(), text);

    // 3. Simulate external deletion by user
    std::fs::remove_file(ws.vault().path_of(&doc_id)).unwrap();

    // 4. Next sync should detect deletion and move to trash
    let scan = ws.pull_from_disk().unwrap();
    assert!(scan.deleted.contains(&doc_id));

    // 5. Verify it's in trash
    let trash_entries = norm_core::trash::list(ws.vault().root()).unwrap();
    assert_eq!(trash_entries.len(), 1);
    assert_eq!(trash_entries[0].doc_id, doc_id.as_str());

    // 6. Verify tombstone in CRDT
    assert!(ws.replica().is_deleted(&doc_id).unwrap());

    // 7. Test restore
    let hash = &trash_entries[0].hash;
    let (restored_id, restored_text) = norm_core::trash::restore(ws.vault().root(), hash)
        .unwrap()
        .unwrap();
    assert_eq!(restored_id, doc_id);
    assert_eq!(restored_text, text);

    // 8. Test purge
    let removed = norm_core::trash::purge(ws.vault().root(), 0).unwrap();
    assert_eq!(removed, 1);
}
