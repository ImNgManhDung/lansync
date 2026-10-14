use anyhow::{Context, Result};
use chrono::Local;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, Notify, RwLock};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::discovery::{DiscoveredPeer, DiscoveryService};
use crate::fsutil::hash_file;
use crate::identity::{device_id_from_cert, fingerprint, Identity};
use crate::index::Index;
use crate::pairing::{pairing_code, Peer, PeerStore};
use crate::paths::to_fs_path;
use crate::planner::{plan, Action, PlanInput};
use crate::protocol::{
    index_messages, read_msg, write_msg, Entry, Hash, Message, CHUNK_SIZE, PROTO_VERSION,
};
use crate::transfer::{FinishError, Incoming};
use crate::transport::{accept, client_config, connect, server_config, SecureConn};
use crate::watcher::FolderWatcher;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SyncEvent {
    PeerDiscovered(DiscoveredPeer),
    PeerConnected { peer_id: String, device_name: String },
    PeerDisconnected { peer_id: String },
    PairingRequested { peer_id: String, device_name: String, code: String },
    PairingAccepted { peer_id: String },
    PairingRejected { peer_id: String },
    SyncStarted { peer_id: String },
    FileDownloading { rel_path: String, size: u64 },
    FileDownloaded { rel_path: String },
    ConflictCreated { original: String, conflict: String },
    SyncCompleted { peer_id: String },
    Error(String),
}

type PendingPairingMap = HashMap<String, (String, oneshot::Sender<bool>)>;

pub struct SyncEngine {
    config: Arc<RwLock<Config>>,
    identity: Arc<Identity>,
    peer_store: Arc<Mutex<PeerStore>>,
    index_db: Arc<Mutex<Index>>,
    events_tx: broadcast::Sender<SyncEvent>,
    pending_pairings: Arc<Mutex<PendingPairingMap>>,
    connected_peers: Arc<Mutex<HashSet<String>>>,
    active_peer_senders: Arc<Mutex<HashMap<String, mpsc::Sender<Message>>>>,
    shutdown_notify: Arc<Notify>,
    local_addr: SocketAddr,
    _watcher: Arc<Mutex<Option<FolderWatcher>>>,
    discovery: Arc<Mutex<Option<DiscoveryService>>>,
}

impl SyncEngine {
    pub async fn start(config: Config, data_dir: PathBuf) -> Result<Arc<Self>> {
        tokio::fs::create_dir_all(&data_dir).await?;
        tokio::fs::create_dir_all(&config.sync_dir).await?;

        let identity = Arc::new(Identity::load_or_create(&data_dir)?);
        let peer_store = Arc::new(Mutex::new(PeerStore::load(&data_dir.join("peers.json"))?));
        let index_db = Arc::new(Mutex::new(Index::open(
            &data_dir.join("index.db"),
            &config.sync_dir,
        )?));

        // Initial scan of the sync directory
        {
            let mut db = index_db.lock().await;
            let report = db.scan()?;
            info!(
                "Initial scan complete: {} changed, {} skipped",
                report.changed.len(), report.skipped.len()
            );
        }

        let (events_tx, _) = broadcast::channel(128);
        let shutdown_notify = Arc::new(Notify::new());

        // Start TCP Listener with TLS
        let bind_addr = SocketAddr::from(([0, 0, 0, 0], config.listen_port));
        let listener = TcpListener::bind(bind_addr)
            .await
            .with_context(|| format!("Failed to bind to port {}", config.listen_port))?;
        let local_addr = listener.local_addr()?;

        let srv_cfg = server_config(&identity)?;
        let cli_cfg = client_config(&identity)?;

        let engine = Arc::new(Self {
            config: Arc::new(RwLock::new(config.clone())),
            identity: identity.clone(),
            peer_store: peer_store.clone(),
            index_db: index_db.clone(),
            events_tx: events_tx.clone(),
            pending_pairings: Arc::new(Mutex::new(HashMap::new())),
            connected_peers: Arc::new(Mutex::new(HashSet::new())),
            active_peer_senders: Arc::new(Mutex::new(HashMap::new())),
            shutdown_notify: shutdown_notify.clone(),
            local_addr,
            _watcher: Arc::new(Mutex::new(None)),
            discovery: Arc::new(Mutex::new(None)),
        });

        // Start file watcher
        let (watcher_tx, mut watcher_rx) = mpsc::channel(16);
        let watcher = FolderWatcher::start(
            &config.sync_dir,
            Duration::from_millis(500),
            watcher_tx,
        )?;
        *engine._watcher.lock().await = Some(watcher);

        // Task to handle file watcher events
        let engine_watcher = engine.clone();
        tokio::spawn(async move {
            while watcher_rx.recv().await.is_some() {
                debug!("Watcher detected changes, rescanning index...");
                let changed_entries = {
                    let mut db = engine_watcher.index_db.lock().await;
                    match db.scan() {
                        Ok(report) => report.changed,
                        Err(e) => {
                            warn!("Index scan error after watcher event: {:?}", e);
                            Vec::new()
                        }
                    }
                };

                if !changed_entries.is_empty() {
                    let senders = engine_watcher.active_peer_senders.lock().await.clone();
                    for (_peer, sender) in senders {
                        let _ = sender
                            .send(Message::IndexUpdate {
                                entries: changed_entries.clone(),
                            })
                            .await;
                    }
                }
            }
        });

        // Start mDNS discovery service
        if let Ok(mut disc) = DiscoveryService::new(&identity.device_id) {
            let _ = disc.register(&config.device_name, local_addr.port());
            if let Ok(mut rx_peer) = disc.browse() {
                let engine_disc = engine.clone();
                tokio::spawn(async move {
                    while let Ok(discovered) = rx_peer.recv().await {
                        let _ = engine_disc
                            .events_tx
                            .send(SyncEvent::PeerDiscovered(discovered.clone()));

                        // If this peer is already paired and not connected, connect automatically
                        let is_paired = {
                            let store = engine_disc.peer_store.lock().await;
                            store.list().iter().any(|p| p.device_id == discovered.device_id)
                        };
                        let is_connected = {
                            let conns = engine_disc.connected_peers.lock().await;
                            conns.contains(&discovered.device_id)
                        };

                        if is_paired && !is_connected {
                            for addr in discovered.addresses {
                                if let Ok(conn) = connect(addr, cli_cfg.clone()).await {
                                    let eng = engine_disc.clone();
                                    tokio::spawn(async move {
                                        eng.handle_connection(conn, false).await;
                                    });
                                    break;
                                }
                            }
                        }
                    }
                });
            }
            *engine.discovery.lock().await = Some(disc);
        }

        // Loop to accept incoming TCP connections
        let engine_srv = engine.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = engine_srv.shutdown_notify.notified() => {
                        break;
                    }
                    accepted = listener.accept() => {
                        match accepted {
                            Ok((tcp, _addr)) => {
                                let scfg = srv_cfg.clone();
                                let eng = engine_srv.clone();
                                tokio::spawn(async move {
                                    match accept(tcp, scfg).await {
                                        Ok(conn) => {
                                            eng.handle_connection(conn, true).await;
                                        }
                                        Err(e) => {
                                            debug!("Incoming TLS handshake error: {:?}", e);
                                        }
                                    }
                                });
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        });

        Ok(engine)
    }

    pub fn local_device_id(&self) -> &str {
        &self.identity.device_id
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<SyncEvent> {
        self.events_tx.subscribe()
    }

    pub async fn approve_pairing(&self, peer_id: &str) -> Result<()> {
        let mut map = self.pending_pairings.lock().await;
        if let Some((_code, tx)) = map.remove(peer_id) {
            let _ = tx.send(true);
            let _ = self
                .events_tx
                .send(SyncEvent::PairingAccepted { peer_id: peer_id.to_string() });
            Ok(())
        } else {
            anyhow::bail!("No pending pairing request found for device {}", peer_id)
        }
    }

    pub async fn reject_pairing(&self, peer_id: &str) -> Result<()> {
        let mut map = self.pending_pairings.lock().await;
        if let Some((_code, tx)) = map.remove(peer_id) {
            let _ = tx.send(false);
            let _ = self
                .events_tx
                .send(SyncEvent::PairingRejected { peer_id: peer_id.to_string() });
            Ok(())
        } else {
            anyhow::bail!("No pending pairing request found for device {}", peer_id)
        }
    }

    pub async fn pair_with(self: &Arc<Self>, addr: SocketAddr) -> Result<()> {
        let cli_cfg = client_config(&self.identity)?;
        let conn = connect(addr, cli_cfg).await?;
        let eng = self.clone();
        tokio::spawn(async move {
            eng.handle_connection(conn, false).await;
        });
        Ok(())
    }

    pub async fn handle_connection(self: Arc<Self>, conn: SecureConn, _is_incoming: bool) {
        let peer_cert = conn.peer_cert;
        let peer_id = device_id_from_cert(&peer_cert);
        let peer_fp = fingerprint(&peer_cert);

        if peer_id == self.identity.device_id {
            return;
        }

        // Split stream into Read + Write halves
        let (mut r, mut w) = tokio::io::split(conn.stream);
        let (tx_msg, mut rx_msg) = mpsc::channel::<Message>(64);

        // Writer task
        let writer_task = tokio::spawn(async move {
            while let Some(msg) = rx_msg.recv().await {
                if let Err(e) = write_msg(&mut w, &msg).await {
                    debug!("Error writing message to peer: {:?}", e);
                    break;
                }
            }
        });

        // Check if peer is already paired
        let is_paired = {
            let store = self.peer_store.lock().await;
            store.find_by_fingerprint(&peer_fp).is_some()
        };

        let cfg = self.config.read().await.clone();

        // Step 1: Exchange Hello messages
        let hello_out = Message::Hello {
            device_id: self.identity.device_id.clone(),
            name: cfg.device_name.clone(),
            proto: PROTO_VERSION,
            pairing: !is_paired,
            listen_port: self.local_addr.port(),
        };
        if let Err(e) = tx_msg.send(hello_out).await {
            warn!("Failed to send Hello message: {:?}", e);
            return;
        }

        let remote_hello = match read_msg(&mut r).await {
            Ok(Message::Hello { device_id, name, proto, pairing, .. }) => {
                if proto != PROTO_VERSION {
                    warn!("Incompatible protocol version: {}", proto);
                    return;
                }
                (device_id, name, pairing)
            }
            other => {
                warn!("Received invalid initial message: {:?}", other);
                return;
            }
        };

        let remote_name = remote_hello.1;
        let pairing_mode = !is_paired || remote_hello.2;

        if pairing_mode {
            let code = pairing_code(&self.identity.cert_der, &peer_cert);

            let accepted = if cfg.auto_accept_pairing {
                true
            } else {
                let (decision_tx, decision_rx) = oneshot::channel();
                {
                    let mut pend = self.pending_pairings.lock().await;
                    pend.insert(peer_id.clone(), (code.clone(), decision_tx));
                }

                let _ = self.events_tx.send(SyncEvent::PairingRequested {
                    peer_id: peer_id.clone(),
                    device_name: remote_name.clone(),
                    code: code.clone(),
                });

                // Await decision or 60-second timeout
                match tokio::time::timeout(Duration::from_secs(60), decision_rx).await {
                    Ok(Ok(decision)) => decision,
                    _ => false,
                }
            };

            let _ = tx_msg.send(Message::PairDecision { accept: accepted }).await;

            // Read remote peer decision
            let peer_decision = match read_msg(&mut r).await {
                Ok(Message::PairDecision { accept }) => accept,
                _ => false,
            };

            if !accepted || !peer_decision {
                info!("Pairing failed or rejected with peer {}", peer_id);
                return;
            }

            // Save peer in PeerStore
            {
                let mut store = self.peer_store.lock().await;
                store.add(Peer {
                    device_id: peer_id.clone(),
                    name: remote_name.clone(),
                    cert_fingerprint: peer_fp.clone(),
                    last_addr: Some(conn.peer_addr.to_string()),
                });
                let _ = store.save();
            }
            let _ = self
                .events_tx
                .send(SyncEvent::PairingAccepted { peer_id: peer_id.clone() });
        }

        // Successfully paired and authenticated
        {
            let mut conns = self.connected_peers.lock().await;
            if !conns.insert(peer_id.clone()) {
                // Connection already established with this peer
                return;
            }
        }
        {
            let mut senders = self.active_peer_senders.lock().await;
            senders.insert(peer_id.clone(), tx_msg.clone());
        }

        let _ = self.events_tx.send(SyncEvent::PeerConnected {
            peer_id: peer_id.clone(),
            device_name: remote_name.clone(),
        });
        let _ = self
            .events_tx
            .send(SyncEvent::SyncStarted { peer_id: peer_id.clone() });

        // Send local index to remote peer
        let local_entries = {
            let db = self.index_db.lock().await;
            db.entries().unwrap_or_default()
        };
        for msg in index_messages(local_entries, true) {
            let _ = tx_msg.send(msg).await;
        }

        let mut remote_entries: HashMap<String, Entry> = HashMap::new();
        let mut expected_expected_local: HashMap<String, Option<Hash>> = HashMap::new();
        let mut current_incoming: Option<Incoming> = None;
        let mut current_incoming_path: Option<String> = None;

        // Loop to read incoming messages from peer
        loop {
            let msg = match read_msg(&mut r).await {
                Ok(m) => m,
                Err(_) => break,
            };

            match msg {
                Message::Index { entries } => {
                    remote_entries.clear();
                    for e in entries {
                        remote_entries.insert(e.path.clone(), e);
                    }
                    self.execute_plan(
                        &peer_id,
                        &remote_entries,
                        &tx_msg,
                        &mut expected_expected_local,
                    )
                    .await;
                }
                Message::IndexUpdate { entries } => {
                    for e in entries {
                        remote_entries.insert(e.path.clone(), e);
                    }
                    self.execute_plan(
                        &peer_id,
                        &remote_entries,
                        &tx_msg,
                        &mut expected_expected_local,
                    )
                    .await;
                }
                Message::GetFile { path, hash } => {
                    let sync_dir = self.config.read().await.sync_dir.clone();
                    let tx_chunk = tx_msg.clone();
                    tokio::spawn(async move {
                        let full = to_fs_path(&sync_dir, &path);
                        if let Ok(meta) = tokio::fs::metadata(&full).await {
                            let mtime = crate::fsutil::mtime_ns(&meta);
                            let size = meta.len();
                            let _ = tx_chunk
                                .send(Message::FileHeader {
                                    path: path.clone(),
                                    size,
                                    hash,
                                    mtime_ns: mtime,
                                })
                                .await;

                            if let Ok(mut f) = tokio::fs::File::open(&full).await {
                                use tokio::io::AsyncReadExt;
                                let mut buf = vec![0u8; CHUNK_SIZE];
                                loop {
                                    match f.read(&mut buf).await {
                                        Ok(0) => break,
                                        Ok(n) => {
                                            let _ = tx_chunk
                                                .send(Message::FileChunk {
                                                    data: buf[..n].to_vec(),
                                                })
                                                .await;
                                        }
                                        Err(_) => break,
                                    }
                                }
                            }
                            let _ = tx_chunk.send(Message::FileEnd).await;
                        } else {
                            let _ = tx_chunk.send(Message::FileUnavailable { path }).await;
                        }
                    });
                }
                Message::FileHeader { path, size, hash, mtime_ns } => {
                    let sync_dir = self.config.read().await.sync_dir.clone();
                    let _ = self.events_tx.send(SyncEvent::FileDownloading {
                        rel_path: path.clone(),
                        size,
                    });
                    current_incoming_path = Some(path.clone());
                    current_incoming = Some(Incoming::begin(&sync_dir, &path, size, hash, mtime_ns).await);
                }
                Message::FileChunk { data } => {
                    if let Some(ref mut inc) = current_incoming {
                        inc.chunk(&data).await;
                    }
                }
                Message::FileEnd => {
                    if let (Some(inc), Some(rel)) = (current_incoming.take(), current_incoming_path.take()) {
                        let exp = expected_expected_local.remove(&rel).flatten();
                        match inc.finish(exp).await {
                            Ok(entry) => {
                                let mut db = self.index_db.lock().await;
                                let _ = db.record_file(&entry);
                                if let Some(h) = entry.hash {
                                    let _ = db.set_base(&peer_id, &entry.path, &h);
                                }
                                let _ = self.events_tx.send(SyncEvent::FileDownloaded {
                                    rel_path: entry.path,
                                });
                            }
                            Err(FinishError::HashMismatch) => {
                                warn!("Downloaded file {} failed hash verification, discarded", rel);
                            }
                            Err(FinishError::LocalChanged) => {
                                info!("Local file {} modified during download, preserving local copy", rel);
                            }
                            Err(FinishError::Io(e)) => {
                                warn!("IO error writing file {}: {}", rel, e);
                            }
                        }
                    }
                }
                Message::FileUnavailable { path } => {
                    debug!("Peer reported file {} unavailable", path);
                }
                Message::Ping => {
                    let _ = tx_msg.send(Message::Pong).await;
                }
                Message::Pong => {}
                _ => {}
            }
        }

        // Cleanup upon disconnection
        {
            let mut conns = self.connected_peers.lock().await;
            conns.remove(&peer_id);
        }
        {
            let mut senders = self.active_peer_senders.lock().await;
            senders.remove(&peer_id);
        }
        let _ = self.events_tx.send(SyncEvent::PeerDisconnected {
            peer_id: peer_id.clone(),
        });
        drop(tx_msg);
        let _ = writer_task.await;
    }

    async fn execute_plan(
        &self,
        peer_id: &str,
        remote_entries: &HashMap<String, Entry>,
        tx_msg: &mpsc::Sender<Message>,
        expected_local_map: &mut HashMap<String, Option<Hash>>,
    ) {
        let local_entries = {
            let db = self.index_db.lock().await;
            db.entries().unwrap_or_default()
        };
        let bases = {
            let db = self.index_db.lock().await;
            db.bases(peer_id).unwrap_or_default()
        };

        let my_id = self.identity.device_id.clone();
        let i_win = my_id.as_str() < peer_id;
        let my_name = self.config.read().await.device_name.clone();
        let stamp = Local::now().format("%Y-%m-%d %H%M").to_string();

        let remote_vec: Vec<Entry> = remote_entries.values().cloned().collect();
        let actions = plan(&PlanInput {
            local: &local_entries,
            remote: &remote_vec,
            base: &bases,
            i_win_conflicts: i_win,
            my_name: &my_name,
            stamp: &stamp,
        });

        let sync_dir = self.config.read().await.sync_dir.clone();

        for action in actions {
            match action {
                Action::Download { local_path, remote_path, hash, expected_local } => {
                    expected_local_map.insert(local_path.clone(), expected_local);
                    let _ = tx_msg
                        .send(Message::GetFile {
                            path: remote_path,
                            hash,
                        })
                        .await;
                }
                Action::UpdateBase { path, hash } => {
                    let mut db = self.index_db.lock().await;
                    let _ = db.set_base(peer_id, &path, &hash);
                }
                Action::Conflict { local_path, conflict_path, remote_path, hash } => {
                    // Rename local file to conflict filename
                    let src = to_fs_path(&sync_dir, &local_path);
                    let dst = to_fs_path(&sync_dir, &conflict_path);
                    if let Ok(()) = tokio::fs::rename(&src, &dst).await {
                        // Update index for the freshly created conflict file
                        if let Ok(meta) = tokio::fs::metadata(&dst).await {
                            if let Ok(h) = hash_file(&dst) {
                                let entry = Entry {
                                    path: conflict_path.clone(),
                                    size: meta.len(),
                                    mtime_ns: crate::fsutil::mtime_ns(&meta),
                                    hash: Some(h),
                                };
                                let mut db = self.index_db.lock().await;
                                let _ = db.record_file(&entry);
                            }
                        }
                        let _ = self.events_tx.send(SyncEvent::ConflictCreated {
                            original: local_path.clone(),
                            conflict: conflict_path,
                        });
                    }

                    // Download remote copy to original filename
                    expected_local_map.insert(local_path, None);
                    let _ = tx_msg
                        .send(Message::GetFile {
                            path: remote_path,
                            hash,
                        })
                        .await;
                }
            }
        }

        let _ = self
            .events_tx
            .send(SyncEvent::SyncCompleted { peer_id: peer_id.to_string() });
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.shutdown_notify.notify_waiters();
        if let Some(disc) = self.discovery.lock().await.take() {
            let _ = disc.shutdown();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_engine_start_and_shutdown() {
        let dir = tempdir().unwrap();
        let sync_dir = dir.path().join("sync");
        let data_dir = dir.path().join("data");
        let cfg = Config::new(sync_dir, 0, "TestDev".into());
        let engine = SyncEngine::start(cfg, data_dir).await.unwrap();
        assert!(!engine.local_device_id().is_empty());
        engine.shutdown().await.unwrap();
    }
}
