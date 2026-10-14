use anyhow::Result;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

use lansync_core::config::Config;
use lansync_core::engine::{SyncEngine, SyncEvent};

#[tokio::main]
async fn main() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let args: Vec<String> = env::args().collect();
    let mut name = "DevMachine".to_string();
    let mut sync_dir = PathBuf::from("./LanSync_Folder");
    let mut data_dir = PathBuf::from("./.lansync_data");
    let mut port: u16 = 42424;
    let mut auto_accept = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => {
                if i + 1 < args.len() {
                    name = args[i + 1].clone();
                    i += 1;
                }
            }
            "--dir" => {
                if i + 1 < args.len() {
                    sync_dir = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--data" => {
                if i + 1 < args.len() {
                    data_dir = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--port" => {
                if i + 1 < args.len() {
                    port = args[i + 1].parse().unwrap_or(42424);
                    i += 1;
                }
            }
            "--auto-accept" => {
                auto_accept = true;
            }
            _ => {}
        }
        i += 1;
    }

    println!("============================================================");
    println!(" LanSync Core Dev Tool");
    println!("  Device name  : {}", name);
    println!("  Sync dir     : {:?}", sync_dir);
    println!("  Data dir     : {:?}", data_dir);
    println!("  Listen port  : {}", port);
    println!("  Auto-accept  : {}", auto_accept);
    println!("============================================================");
    println!("Interactive commands:");
    println!("  p <ip:port>   : Manually pair with IP:Port");
    println!("  y <device_id> : Accept pairing request");
    println!("  n <device_id> : Reject pairing request");
    println!("  q             : Quit application");
    println!("============================================================\n");

    let mut config = Config::new(sync_dir, port, name);
    config.auto_accept_pairing = auto_accept;

    let engine = SyncEngine::start(config, data_dir).await?;
    println!("LanSync started successfully.");
    println!("Local device ID: {}\n", engine.local_device_id());

    let mut events = engine.subscribe_events();
    let engine_events = engine.clone();

    // Event printer task
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            match event {
                SyncEvent::PeerDiscovered(peer) => {
                    println!("\n[DISCOVERY] Discovered peer: {} (ID: {}) at {:?}", peer.device_name, peer.device_id, peer.addresses);
                }
                SyncEvent::PairingRequested { peer_id, device_name, code } => {
                    println!("\n[PAIRING REQUEST] Device '{}' ({}) requested pairing.", device_name, peer_id);
                    println!(">>> CONFIRMATION CODE: [{}] <<< (Verify both devices display this code)", code);
                    println!("Enter 'y {}' to accept or 'n {}' to reject.", peer_id, peer_id);
                }
                SyncEvent::PairingAccepted { peer_id } => {
                    println!("\n[PAIRING] Successfully paired with device: {}", peer_id);
                }
                SyncEvent::PairingRejected { peer_id } => {
                    println!("\n[PAIRING] Rejected pairing with: {}", peer_id);
                }
                SyncEvent::PeerConnected { peer_id, device_name } => {
                    println!("\n[CONNECTED] Connected to '{}' ({})", device_name, peer_id);
                }
                SyncEvent::PeerDisconnected { peer_id } => {
                    println!("\n[DISCONNECTED] Disconnected from: {}", peer_id);
                }
                SyncEvent::SyncStarted { peer_id } => {
                    println!("\n[SYNC START] Started syncing with {}", peer_id);
                }
                SyncEvent::FileDownloading { rel_path, size } => {
                    println!("[DOWNLOADING] {} ({} bytes)...", rel_path, size);
                }
                SyncEvent::FileDownloaded { rel_path } => {
                    println!("[DOWNLOADED] Completed downloading: {}", rel_path);
                }
                SyncEvent::ConflictCreated { original, conflict } => {
                    println!("[CONFLICT] Conflict detected! Both versions preserved:");
                    println!("  Original : {}", original);
                    println!("  Copy     : {}", conflict);
                }
                SyncEvent::SyncCompleted { peer_id } => {
                    println!("[SYNC DONE] Sync completed with {}\n", peer_id);
                }
                SyncEvent::Error(err) => {
                    eprintln!("[ERROR] {}", err);
                }
            }
        }
    });

    // Stdin reading loop
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();

    while let Ok(Some(line)) = reader.next_line().await {
        let trimmed = line.trim();
        if trimmed == "q" {
            println!("Shutting down LanSync...");
            break;
        } else if let Some(addr_str) = trimmed.strip_prefix("p ") {
            if let Ok(addr) = addr_str.trim().parse::<SocketAddr>() {
                println!("Connecting to {}...", addr);
                if let Err(e) = engine_events.pair_with(addr).await {
                    eprintln!("Connection error: {:?}", e);
                }
            } else {
                eprintln!("Invalid address! Example: p 192.168.1.10:42424");
            }
        } else if let Some(peer_id) = trimmed.strip_prefix("y ") {
            if let Err(e) = engine_events.approve_pairing(peer_id.trim()).await {
                eprintln!("Approval error: {:?}", e);
            }
        } else if let Some(peer_id) = trimmed.strip_prefix("n ") {
            if let Err(e) = engine_events.reject_pairing(peer_id.trim()).await {
                eprintln!("Rejection error: {:?}", e);
            }
        }
    }

    engine.shutdown().await?;
    println!("LanSync stopped.");
    Ok(())
}
