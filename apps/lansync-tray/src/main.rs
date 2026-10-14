// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;
mod tray;

use std::path::PathBuf;
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;
use tracing::info;

use lansync_core::config::Config;
use lansync_core::engine::{SyncEngine, SyncEvent};
use state::{ActivityItem, AppState, PendingPairingInfo};

fn main() {
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let runtime = tokio::runtime::Runtime::new().expect("Failed to initialize tokio runtime");

    // Determine data directory and config path
    let data_dir = Config::default_data_dir();
    let config_path = data_dir.join("config.json");
    let default_sync_dir = directories::UserDirs::new()
        .and_then(|u| u.document_dir().map(|d| d.join("LanSync")))
        .unwrap_or_else(|| PathBuf::from("./LanSync"));

    let config = Config::load_or_default(&config_path, default_sync_dir.clone())
        .unwrap_or_else(|_| Config::new(default_sync_dir.clone(), 42424, "LanSync Device".into()));

    let sync_dir = config.sync_dir.clone();
    let device_name = config.device_name.clone();
    let listen_port = config.listen_port;

    // Start SyncEngine
    let engine = runtime
        .block_on(async { SyncEngine::start(config, data_dir.clone()).await })
        .expect("Failed to start LanSync Core Engine");

    let app_state = Arc::new(AppState {
        engine: engine.clone(),
        data_dir,
        config_path,
        sync_dir,
        device_name,
        listen_port,
        activities: Arc::new(Mutex::new(Vec::new())),
        pending_pairing: Arc::new(Mutex::new(None)),
    });

    let app_state_for_events = app_state.clone();
    let mut rx_events = engine.subscribe_events();

    tauri::Builder::default()
        .manage(app_state.clone())
        .setup(move |app| {
            let app_handle = app.handle().clone();

            // Initialize System Tray
            tray::setup_tray(&app_handle, app_state.clone())?;

            // When user clicks the window close button (X), hide to tray instead of quitting
            if let Some(window) = app.get_webview_window("main") {
                let win_clone = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win_clone.hide();
                    }
                });
                let _ = window.show();
                let _ = window.set_focus();
            }

            // Event forwarding task: engine -> webview frontend
            let app_handle_events = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                while let Ok(event) = rx_events.recv().await {
                    match &event {
                        SyncEvent::PairingRequested { peer_id, device_name, code } => {
                            *app_state_for_events.pending_pairing.lock().await = Some(PendingPairingInfo {
                                peer_id: peer_id.clone(),
                                device_name: device_name.clone(),
                                code: code.clone(),
                            });
                            // Automatically show and focus the window when a pairing request arrives!
                            if let Some(w) = app_handle_events.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                        SyncEvent::PairingAccepted { .. } | SyncEvent::PairingRejected { .. } => {
                            *app_state_for_events.pending_pairing.lock().await = None;
                        }
                        _ => {}
                    }

                    let activity_opt = match &event {
                        SyncEvent::PeerDiscovered(p) => Some(ActivityItem::new(
                            "Device Discovered",
                            format!("{} ({})", p.device_name, p.device_id),
                            "peer",
                        )),
                        SyncEvent::PairingRequested { device_name, code, .. } => Some(ActivityItem::new(
                            "Pairing Requested",
                            format!("Device: {}, Code: {}", device_name, code),
                            "peer",
                        )),
                        SyncEvent::PairingAccepted { peer_id } => Some(ActivityItem::new(
                            "Pairing Succeeded",
                            format!("ID: {}", peer_id),
                            "peer",
                        )),
                        SyncEvent::PeerConnected { device_name, .. } => Some(ActivityItem::new(
                            "Connected",
                            device_name.clone(),
                            "peer",
                        )),
                        SyncEvent::PeerDisconnected { peer_id } => Some(ActivityItem::new(
                            "Disconnected",
                            format!("ID: {}", peer_id),
                            "peer",
                        )),
                        SyncEvent::FileDownloading { rel_path, size } => Some(ActivityItem::new(
                            "Downloading File",
                            format!("{} ({} B)", rel_path, size),
                            "file",
                        )),
                        SyncEvent::FileDownloaded { rel_path } => Some(ActivityItem::new(
                            "Download Complete",
                            rel_path.clone(),
                            "file",
                        )),
                        SyncEvent::ConflictCreated { original, conflict } => Some(ActivityItem::new(
                            "File Conflict",
                            format!("Original: {} -> Copy: {}", original, conflict),
                            "conflict",
                        )),
                        SyncEvent::SyncCompleted { .. } => Some(ActivityItem::new(
                            "Sync Completed",
                            "Data is up to date",
                            "sync",
                        )),
                        SyncEvent::FileTrashed { rel_path, .. } => Some(ActivityItem::new(
                            "Moved to Trash",
                            rel_path.clone(),
                            "file",
                        )),
                        SyncEvent::PendingDeletion { peer_id, rel_path } => Some(ActivityItem::new(
                            "Pending Deletion",
                            format!("Peer {} requested deletion of {}", peer_id, rel_path),
                            "file",
                        )),
                        SyncEvent::Error(err) => Some(ActivityItem::new(
                            "System Error",
                            err.clone(),
                            "error",
                        )),
                        _ => None,
                    };

                    if let Some(item) = activity_opt {
                        app_state_for_events.add_activity(item).await;
                    }

                    // Forward event to UI
                    let _ = app_handle_events.emit("sync_event", serde_json::to_value(&event).unwrap_or_default());
                }
            });

            info!("LanSync Tray application is ready.");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::open_sync_folder,
            commands::pair_device,
            commands::approve_pairing,
            commands::reject_pairing,
            commands::pick_sync_folder,
            commands::list_peers,
            commands::list_discovered_peers,
            commands::rescan_peers,
            commands::unpair_device,
            commands::list_conflicts,
            commands::resolve_conflict,
            commands::get_delete_mode,
            commands::set_delete_mode,
            commands::set_peer_delete_mode,
            commands::list_pending_deletions,
            commands::resolve_pending_deletion,
            commands::list_versioned_files,
            commands::list_file_versions,
            commands::restore_file_version,
            commands::is_sync_paused,
            commands::set_sync_paused,
            commands::get_bandwidth_limit,
            commands::set_bandwidth_limit,
            commands::trigger_manual_sync,
        ])
        .run(tauri::generate_context!())
        .expect("error running tauri application");
}
