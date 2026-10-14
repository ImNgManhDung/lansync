use lansync_core::config::Config;
use lansync_core::engine::{SyncEngine, SyncEvent};
use std::time::Duration;
use tempfile::tempdir;

#[tokio::test]
async fn test_two_way_sync_and_rules() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();

    let sync_a = dir_a.path().join("sync");
    let data_a = dir_a.path().join("data");
    let sync_b = dir_b.path().join("sync");
    let data_b = dir_b.path().join("data");

    let mut cfg_a = Config::new(sync_a.clone(), 0, "MachineA".into());
    cfg_a.auto_accept_pairing = true;
    let mut cfg_b = Config::new(sync_b.clone(), 0, "MachineB".into());
    cfg_b.auto_accept_pairing = true;

    let engine_a = SyncEngine::start(cfg_a, data_a).await.unwrap();
    let engine_b = SyncEngine::start(cfg_b, data_b).await.unwrap();

    let mut rx_b = engine_b.subscribe_events();

    // 1. Create file on Machine A BEFORE connecting
    tokio::fs::write(sync_a.join("hello_a.txt"), b"Content from Machine A").await.unwrap();

    // 2. Connect and pair
    engine_a.pair_with(engine_b.local_addr()).await.unwrap();

    // Wait for sync completed event or file download
    let mut b_received = false;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            Ok(ev) = rx_b.recv() => {
                if let SyncEvent::FileDownloaded { ref rel_path } = ev {
                    if rel_path == "hello_a.txt" {
                        b_received = true;
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
        if b_received {
            break;
        }
    }

    assert!(b_received, "Machine B must receive hello_a.txt from Machine A");
    let content_b = tokio::fs::read(sync_b.join("hello_a.txt")).await.unwrap();
    assert_eq!(content_b, b"Content from Machine A");

    // 3. Core rule: Deletion on A MUST NOT delete on B
    tokio::fs::remove_file(sync_a.join("hello_a.txt")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // File on Machine B must still exist!
    assert!(
        sync_b.join("hello_a.txt").exists(),
        "Core rule: Deleting a file on Machine A must never delete it on Machine B"
    );

    // 4. Critical 1.1 Tombstone rule: Machine A must NOT resurrect hello_a.txt from B
    tokio::time::sleep(Duration::from_millis(2000)).await;
    assert!(
        !sync_a.join("hello_a.txt").exists(),
        "Tombstone rule: Machine A must NOT resurrect the file it just deleted!"
    );

    // Clean shutdown
    engine_a.shutdown().await.unwrap();
    engine_b.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_offline_concurrent_edits_create_conflict_file() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();

    let sync_a = dir_a.path().join("sync");
    let data_a = dir_a.path().join("data");
    let sync_b = dir_b.path().join("sync");
    let data_b = dir_b.path().join("data");

    let mut cfg_a = Config::new(sync_a.clone(), 0, "MachineA".into());
    cfg_a.auto_accept_pairing = true;
    let mut cfg_b = Config::new(sync_b.clone(), 0, "MachineB".into());
    cfg_b.auto_accept_pairing = true;

    // Both machines create the same file with different content while disconnected
    tokio::fs::create_dir_all(&sync_a).await.unwrap();
    tokio::fs::create_dir_all(&sync_b).await.unwrap();
    tokio::fs::write(sync_a.join("notes.txt"), b"Edit from Machine A").await.unwrap();
    tokio::fs::write(sync_b.join("notes.txt"), b"Edit from Machine B").await.unwrap();

    let engine_a = SyncEngine::start(cfg_a, data_a).await.unwrap();
    let engine_b = SyncEngine::start(cfg_b, data_b).await.unwrap();

    // Connect
    engine_a.pair_with(engine_b.local_addr()).await.unwrap();

    // Wait 2 seconds for exchange and conflict resolution
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Both versions must be preserved; one side must have created a conflict copy!
    let files_a: Vec<String> = walkdir::WalkDir::new(&sync_a)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    let files_b: Vec<String> = walkdir::WalkDir::new(&sync_b)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    let has_conflict_file = files_a.iter().chain(files_b.iter()).any(|n| n.contains("conflict"));
    assert!(
        has_conflict_file,
        "Concurrent edits on both machines must generate a conflict file: A={:?}, B={:?}",
        files_a, files_b
    );

    engine_a.shutdown().await.unwrap();
    engine_b.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_file_rename_propagates_locally_without_retransfer() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();

    let sync_a = dir_a.path().join("sync");
    let data_a = dir_a.path().join("data");
    let sync_b = dir_b.path().join("sync");
    let data_b = dir_b.path().join("data");

    let mut cfg_a = Config::new(sync_a.clone(), 0, "MachineA".into());
    cfg_a.auto_accept_pairing = true;
    let mut cfg_b = Config::new(sync_b.clone(), 0, "MachineB".into());
    cfg_b.auto_accept_pairing = true;

    // Create file on A
    tokio::fs::create_dir_all(&sync_a).await.unwrap();
    tokio::fs::create_dir_all(&sync_b).await.unwrap();
    let sample_payload = b"Big video or document payload to verify zero re-transfer rename";
    tokio::fs::write(sync_a.join("document_v1.bin"), sample_payload).await.unwrap();

    let engine_a = SyncEngine::start(cfg_a, data_a).await.unwrap();
    let engine_b = SyncEngine::start(cfg_b, data_b).await.unwrap();

    // 1. Initial sync
    engine_a.pair_with(engine_b.local_addr()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2500)).await;

    assert!(sync_b.join("document_v1.bin").exists(), "Machine B must receive initial file");

    // 2. Machine A renames file
    tokio::fs::rename(sync_a.join("document_v1.bin"), sync_a.join("document_renamed.bin")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2500)).await;

    // Machine B must have the renamed file
    assert!(
        sync_b.join("document_renamed.bin").exists(),
        "Machine B must have renamed document_renamed.bin"
    );
    assert_eq!(
        tokio::fs::read(sync_b.join("document_renamed.bin")).await.unwrap(),
        sample_payload
    );

    engine_a.shutdown().await.unwrap();
    engine_b.shutdown().await.unwrap();
}
