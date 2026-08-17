use norm_core::doc::DocId;
use norm_core::oplog::{DeviceId, VaultKey};
use norm_core::workspace::Workspace;
use rayon::prelude::*;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

#[test]
fn test_mass_simulation() {
    let num_personas = 500;
    let _key = VaultKey::new([42u8; 32]);
    let hub_dir = TempDir::new().unwrap();
    let hub_root = hub_dir.path().to_path_buf();

    // Create a central hub workspace
    let hub_device = DeviceId::from_hex("00000000000000000000000000000000").unwrap();
    let hub_ws = Arc::new(Mutex::new(
        Workspace::open(hub_device, VaultKey::new([42u8; 32]), &hub_root).unwrap(),
    ));

    println!("Spawning {} personas...", num_personas);

    // Spawn 500 personas concurrently
    (1..=num_personas).into_par_iter().for_each(|i| {
        let hex = format!("{:032x}", i);
        let device = DeviceId::from_hex(&hex).unwrap();

        let dir = TempDir::new().unwrap();
        let mut ws = Workspace::open(device, VaultKey::new([42u8; 32]), dir.path()).unwrap();

        // Persona does some actions
        let doc1 = DocId::from_relative_path(Path::new(&format!("note_a_{}.md", i)));
        let doc2 = DocId::from_relative_path(Path::new(&format!("folder/note_b_{}.md", i)));

        // Create notes
        ws.replica_mut()
            .write(&doc1, &format!("Hello from persona {} A", i))
            .unwrap();
        ws.replica_mut()
            .write(&doc2, &format!("Hello from persona {} B", i))
            .unwrap();
        ws.push_to_disk().unwrap();

        // Edit note
        ws.replica_mut()
            .write(&doc1, &format!("Hello from persona {} A (edited)", i))
            .unwrap();

        // Merge (simulate: append content and delete old)
        let text1 = ws.replica().text(&doc1).unwrap().unwrap();
        let text2 = ws.replica().text(&doc2).unwrap().unwrap();
        ws.replica_mut()
            .write(&doc2, &format!("{}\n{}", text2, text1))
            .unwrap();
        ws.replica_mut().delete(&doc1).unwrap();

        ws.push_to_disk().unwrap();

        // Sync with hub
        let mut hub = hub_ws.lock().unwrap();
        ws.cycle(hub.replica().store()).unwrap();
        hub.cycle(ws.replica().store()).unwrap();
    });

    // Verification
    println!("Verifying Hub CRDT State...");
    let mut hub = hub_ws.lock().unwrap();
    hub.pull_from_disk().unwrap();

    let live_notes = hub.replica().live_notes().unwrap();
    assert_eq!(
        live_notes.len(),
        500,
        "There should be exactly 500 notes remaining after merges"
    );

    for note in live_notes {
        let text = hub.replica().text(&note).unwrap().unwrap();
        assert!(
            text.contains("(edited)"),
            "Text should contain the merged edit"
        );
    }

    println!(
        "Mass simulation passed successfully for {} personas!",
        num_personas
    );
}
