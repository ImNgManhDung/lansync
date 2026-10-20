use anyhow::{Context, Result};
use chrono::Local;
use rand::Rng;
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
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
use crate::pairing::{pairing_code, PairingLimiter, Peer, PeerStore};
use crate::paths::to_fs_path;
use crate::planner::{plan, Action, PlanInput};
use crate::protocol::{
    index_messages, read_msg, write_msg, Entry, Hash, Message, CHUNK_SIZE, PROTO_VERSION,
};
use crate::transfer::{FinishError, Incoming};
use crate::transport::{accept, client_config, connect, server_config, SecureConn};
use crate::version_vector::VersionVector;
use crate::watcher::FolderWatcher;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerStatus {
    Discovered,
    Connecting,
    Connected,
    Backoff,
    Offline,
}

impl std::fmt::Display for PeerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PeerStatus::Discovered => write!(f, "discovered"),
            PeerStatus::Connecting => write!(f, "connecting"),
            PeerStatus::Connected => write!(f, "connected"),
            PeerStatus::Backoff => write!(f, "backoff"),
            PeerStatus::Offline => write!(f, "offline"),
        }
    }
}

/// Anti-dual-connection rule: Only the peer with smaller device_id dials actively.
pub fn is_active_dialer(my_id: &str, peer_id: &str) -> bool {
    my_id < peer_id
}

/// Calculate next exponential backoff delay with ±20% jitter.
pub fn next_delay(attempt: u32) -> Duration {
    let base = (1u64 << attempt.min(6)).min(60); // 1, 2, 4, 8, 16, 32, 60s
    let jitter = rand::thread_rng().gen_range(0.8..1.2);
    Duration::from_secs_f64(base as f64 * jitter)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PendingDeletion {
    pub peer_id: String,
    pub path: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SyncEvent {
    PeerDiscovered(DiscoveredPeer),
    PeerConnected { peer_id: String, device_name: String },
    PeerDisconnected { peer_id: String },
    PeerStatusChanged { peer_id: String, status: PeerStatus },
    PairingRequested { peer_id: String, device_name: String, code: String },
    PairingAccepted { peer_id: String },
    PairingRejected { peer_id: String },
    SyncStarted { peer_id: String },
    FileDownloading { rel_path: String, size: u64 },
    FileDownloaded { rel_path: String },
    FileTrashed { rel_path: String, trash_path: String },
    PendingDeletion { peer_id: String, rel_path: String },
    ConflictCreated { original: String, conflict: String },
    SyncCompleted { peer_id: String },
    Error(String),
}

type PendingPairingMap = HashMap<String, (String, oneshot::Sender<bool>)>;

pub struct SyncEngine {
    data_dir: PathBuf,
    config: Arc<RwLock<Config>>,
    identity: Arc<Identity>,
    peer_store: Arc<Mutex<PeerStore>>,
    index_db: Arc<Mutex<Index>>,
    events_tx: broadcast::Sender<SyncEvent>,
    pending_pairings: Arc<Mutex<PendingPairingMap>>,
    pending_deletions: Arc<Mutex<Vec<PendingDeletion>>>,
    pairing_limiter: Arc<Mutex<PairingLimiter>>,
    connected_peers: Arc<Mutex<HashSet<String>>>,
    active_peer_senders: Arc<Mutex<HashMap<String, mpsc::Sender<Message>>>>,
    peer_cancellations: Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>,
    shutdown_notify: Arc<Notify>,
    reconnect_notify: Arc<Notify>,
    resync_tx: broadcast::Sender<()>,
    peer_statuses: Arc<Mutex<HashMap<String, PeerStatus>>>,
    local_addr: SocketAddr,
    _watcher: Arc<Mutex<Option<FolderWatcher>>>,
    watcher_tx: mpsc::Sender<()>,
    discovery: Arc<Mutex<Option<DiscoveryService>>>,
    discovered_peers: Arc<Mutex<HashMap<String, DiscoveredPeer>>>,
    is_paused: Arc<std::sync::atomic::AtomicBool>,
    bandwidth_limit_kbps: Arc<std::sync::atomic::AtomicU32>,
}

#[derive(Debug, Clone)]
struct QueuedDownload {
    local_path: String,
    remote_path: String,
    hash: Hash,
    expected_local: Option<Hash>,
}

async fn trigger_next_download(
    sync_dir: &Path,
    queue: &mut VecDeque<QueuedDownload>,
    expected_map: &mut HashMap<String, Option<Hash>>,
    remote_entries: &HashMap<String, Entry>,
    tx: &mpsc::Sender<Message>,
) {
    if let Some(item) = queue.pop_front() {
        expected_map.insert(item.local_path.clone(), item.expected_local);
        let expected_size = remote_entries.get(&item.remote_path).map(|e| e.size).unwrap_or(0);
        let resume_offset = crate::transfer::check_resume_offset(sync_dir, &item.local_path, &item.hash, expected_size).await;
        if resume_offset > 0 {
            info!("Resuming download for {} at offset {}", item.local_path, resume_offset);
            let _ = tx
                .send(Message::GetFileRange {
                    path: item.remote_path,
                    hash: item.hash,
                    offset: resume_offset,
                })
                .await;
        } else {
            info!("Requesting download for {}", item.local_path);
            let _ = tx
                .send(Message::GetFile {
                    path: item.remote_path,
                    hash: item.hash,
                })
                .await;
        }
    }
}

impl SyncEngine {
    pub async fn start(config: Config, data_dir: PathBuf) -> Result<Arc<Self>> {
        tokio::fs::create_dir_all(&data_dir).await?;
        tokio::fs::create_dir_all(&config.sync_dir).await?;

        // 2.5 Clean shutdown verification: detect unexpected crash or abrupt power off
        let clean_flag = data_dir.join(".clean_shutdown");
        let was_clean = clean_flag.exists();
        if !was_clean {
            info!("Previous shutdown was ungraceful. Running thorough initial scan.");
        }
        let _ = tokio::fs::remove_file(&clean_flag).await;

        let identity = Arc::new(Identity::load_or_create(&data_dir)?);
        let peer_store = Arc::new(Mutex::new(PeerStore::load(&data_dir.join("peers.json"))?));
        let index_db = Arc::new(Mutex::new(Index::open(
            &data_dir.join("index.db"),
            &config.sync_dir,
            &identity.device_id,
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

        // Cleanup orphaned temporary files older than 24 hours
        let _ = crate::fsutil::cleanup_orphaned_temps(&config.sync_dir, Duration::from_secs(24 * 3600));

        let sync_dir_cleanup = config.sync_dir.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(6 * 3600));
            interval.tick().await;
            loop {
                interval.tick().await;
                let _ = crate::fsutil::cleanup_orphaned_temps(&sync_dir_cleanup, Duration::from_secs(24 * 3600));
            }
        });

        let (events_tx, _) = broadcast::channel(128);
        let (resync_tx, _) = broadcast::channel(32);
        let shutdown_notify = Arc::new(Notify::new());
        let reconnect_notify = Arc::new(Notify::new());
        let (watcher_tx, mut watcher_rx) = mpsc::channel(16);

        // Start TCP Listener with TLS
        let bind_addr = SocketAddr::from(([0, 0, 0, 0], config.listen_port));
        let listener = TcpListener::bind(bind_addr)
            .await
            .with_context(|| format!("Failed to bind to port {}", config.listen_port))?;
        let local_addr = listener.local_addr()?;

        let srv_cfg = server_config(&identity)?;

        let engine = Arc::new(Self {
            data_dir: data_dir.clone(),
            config: Arc::new(RwLock::new(config.clone())),
            identity: identity.clone(),
            peer_store: peer_store.clone(),
            index_db: index_db.clone(),
            events_tx: events_tx.clone(),
            pending_pairings: Arc::new(Mutex::new(HashMap::new())),
            pending_deletions: Arc::new(Mutex::new(Vec::new())),
            pairing_limiter: Arc::new(Mutex::new(PairingLimiter::default_policy())),
            connected_peers: Arc::new(Mutex::new(HashSet::new())),
            active_peer_senders: Arc::new(Mutex::new(HashMap::new())),
            peer_cancellations: Arc::new(Mutex::new(HashMap::new())),
            shutdown_notify: shutdown_notify.clone(),
            reconnect_notify: reconnect_notify.clone(),
            resync_tx,
            peer_statuses: Arc::new(Mutex::new(HashMap::new())),
            local_addr,
            _watcher: Arc::new(Mutex::new(None)),
            watcher_tx: watcher_tx.clone(),
            discovery: Arc::new(Mutex::new(None)),
            discovered_peers: Arc::new(Mutex::new(HashMap::new())),
            is_paused: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            bandwidth_limit_kbps: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        });

        // Start file watcher
        let watcher = FolderWatcher::start(
            &config.sync_dir,
            Duration::from_millis(500),
            watcher_tx,
        )?;
        *engine._watcher.lock().await = Some(watcher);

        // Task to handle file watcher events and periodic rescans
        let engine_watcher = engine.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(10));
            ticker.tick().await; // consume first immediate tick
            loop {
                let should_scan = tokio::select! {
                    _ = engine_watcher.shutdown_notify.notified() => break,
                    res = watcher_rx.recv() => res.is_some(),
                    _ = ticker.tick() => {
                        let has_peers = !engine_watcher.active_peer_senders.lock().await.is_empty();
                        has_peers
                    }
                };

                if !should_scan || engine_watcher.is_paused() {
                    continue;
                }

                debug!("Rescanning local index...");
                let changed_entries = {
                    let mut db = engine_watcher.index_db.lock().await;
                    match db.scan() {
                        Ok(report) => report.changed,
                        Err(e) => {
                            warn!("Index scan error after watcher/periodic event: {:?}", e);
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
                        {
                            let mut disc_map = engine_disc.discovered_peers.lock().await;
                            if let Some(existing) = disc_map.get_mut(&discovered.device_id) {
                                for addr in discovered.addresses.clone() {
                                    if !existing.addresses.contains(&addr) {
                                        existing.addresses.push(addr);
                                    }
                                }
                                existing.device_name = discovered.device_name.clone();
                                existing.port = discovered.port;
                            } else {
                                disc_map.insert(discovered.device_id.clone(), discovered.clone());
                            }
                        }

                        let _ = engine_disc
                            .events_tx
                            .send(SyncEvent::PeerDiscovered(discovered.clone()));

                        let is_paired = {
                            let store = engine_disc.peer_store.lock().await;
                            store.list().iter().any(|p| p.device_id == discovered.device_id)
                        };
                        let is_connected = {
                            let conns = engine_disc.connected_peers.lock().await;
                            conns.contains(&discovered.device_id)
                        };

                        if !is_connected {
                            if is_paired {
                                engine_disc.reconnect_notify.notify_one();
                            } else {
                                engine_disc
                                    .set_peer_status(&discovered.device_id, PeerStatus::Discovered)
                                    .await;
                            }
                        }
                    }
                });
            }
            *engine.discovery.lock().await = Some(disc);
        }

        // Periodic background subnet probe (fallback when mDNS multicast is dropped by router/firewall)
        let engine_scan = engine.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            engine_scan.scan_subnet_peers().await;

            let mut interval = tokio::time::interval(Duration::from_secs(20));
            loop {
                tokio::select! {
                    _ = engine_scan.shutdown_notify.notified() => {
                        break;
                    }
                    _ = interval.tick() => {
                        engine_scan.scan_subnet_peers().await;
                    }
                }
            }
        });

        // Background event-driven auto-reconnect task with exponential backoff & jitter
        let engine_reconnect = engine.clone();
        tokio::spawn(async move {
            let mut attempts: HashMap<String, u32> = HashMap::new();
            let mut next_retry: HashMap<String, tokio::time::Instant> = HashMap::new();
            let dials_in_flight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

            let mut first_run = true;
            loop {
                if first_run {
                    first_run = false;
                } else {
                    let now = tokio::time::Instant::now();
                    let earliest_retry = next_retry.values().min().copied();
                    let sleep_duration = match earliest_retry {
                        Some(t) if t > now => t - now,
                        Some(_) => Duration::from_millis(50),
                        None => Duration::from_secs(60),
                    };

                    tokio::select! {
                        _ = engine_reconnect.shutdown_notify.notified() => {
                            break;
                        }
                        _ = engine_reconnect.reconnect_notify.notified() => {
                            // Event-driven: peer discovered, network changed, or pairing approved!
                            // Reset backoff delay for peers so we attempt connection immediately
                            for t in next_retry.values_mut() {
                                *t = tokio::time::Instant::now();
                            }
                        }
                        _ = tokio::time::sleep(sleep_duration) => {}
                    }
                }

                let paired_peers = engine_reconnect.peer_store.lock().await.list().to_vec();
                let connected = engine_reconnect.connected_peers.lock().await.clone();
                let local_id = engine_reconnect.identity.device_id.clone();
                let now = tokio::time::Instant::now();

                for peer in paired_peers {
                    if connected.contains(&peer.device_id) {
                        attempts.remove(&peer.device_id);
                        next_retry.remove(&peer.device_id);
                        continue;
                    }

                    // Anti-dual-connection: Only the peer with smaller device_id dials actively
                    if !is_active_dialer(&local_id, &peer.device_id) {
                        let is_discovered = engine_reconnect.discovered_peers.lock().await.contains_key(&peer.device_id);
                        let status = if is_discovered {
                            PeerStatus::Discovered
                        } else {
                            PeerStatus::Offline
                        };
                        engine_reconnect.set_peer_status(&peer.device_id, status).await;
                        continue;
                    }

                    // Avoid duplicate dial attempts if already dialing or connecting to this peer
                    {
                        let in_flight = dials_in_flight.lock().await;
                        if in_flight.contains(&peer.device_id) {
                            continue;
                        }
                    }

                    // Check if it's time to retry this peer
                    if next_retry.get(&peer.device_id).is_some_and(|&retry_time| now < retry_time) {
                        continue;
                    }

                    let addr_opt = {
                        let disc = engine_reconnect.discovered_peers.try_lock().ok();
                        disc.and_then(|d| d.get(&peer.device_id).and_then(|p| {
                            p.addresses.iter().find(|a| a.is_ipv4()).copied()
                        }))
                    }
                    .or_else(|| peer.last_addr.as_ref().and_then(|a| {
                        let mut sa: SocketAddr = a.parse().ok()?;
                        // If stored port is an ephemeral client port (>49151), fallback to default sync port
                        if sa.port() > 49151 {
                            sa.set_port(42424);
                        }
                        Some(sa)
                    }))
                    .or_else(|| {
                        let disc = engine_reconnect.discovered_peers.try_lock().ok();
                        disc.and_then(|d| d.get(&peer.device_id).and_then(|p| p.addresses.first().copied()))
                    });

                    let Some(addr) = addr_opt else {
                        engine_reconnect.set_peer_status(&peer.device_id, PeerStatus::Offline).await;
                        continue;
                    };

                    let Ok(cli_cfg) = client_config(&engine_reconnect.identity) else { continue };

                    engine_reconnect.set_peer_status(&peer.device_id, PeerStatus::Connecting).await;
                    dials_in_flight.lock().await.insert(peer.device_id.clone());

                    let flight_tracker = dials_in_flight.clone();
                    let peer_id_for_flight = peer.device_id.clone();

                    match connect(addr, cli_cfg).await {
                        Ok(conn) => {
                            attempts.remove(&peer.device_id);
                            next_retry.remove(&peer.device_id);
                            let eng = engine_reconnect.clone();
                            tokio::spawn(async move {
                                eng.handle_connection(conn, false).await;
                                flight_tracker.lock().await.remove(&peer_id_for_flight);
                            });
                        }
                        Err(e) => {
                            flight_tracker.lock().await.remove(&peer_id_for_flight);
                            let attempt = attempts.entry(peer.device_id.clone()).or_insert(0);
                            *attempt = attempt.saturating_add(1);
                            let delay = next_delay(*attempt);
                            next_retry.insert(peer.device_id.clone(), tokio::time::Instant::now() + delay);
                            engine_reconnect.set_peer_status(&peer.device_id, PeerStatus::Backoff).await;
                            debug!(
                                "Reconnect failed to {} ({:?}): {}. Next attempt in {:?}",
                                peer.device_id, addr, e, delay
                            );
                        }
                    }
                }
            }
        });

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

    pub async fn get_peer_status(&self, peer_id: &str) -> PeerStatus {
        let statuses = self.peer_statuses.lock().await;
        if let Some(&s) = statuses.get(peer_id) {
            return s;
        }
        let connected = self.connected_peers.lock().await;
        if connected.contains(peer_id) {
            return PeerStatus::Connected;
        }
        let discovered = self.discovered_peers.lock().await;
        if discovered.contains_key(peer_id) {
            PeerStatus::Discovered
        } else {
            PeerStatus::Offline
        }
    }

    pub async fn set_peer_status(&self, peer_id: &str, status: PeerStatus) {
        let mut statuses = self.peer_statuses.lock().await;
        let prev = statuses.insert(peer_id.to_string(), status);
        if prev != Some(status) {
            let _ = self.events_tx.send(SyncEvent::PeerStatusChanged {
                peer_id: peer_id.to_string(),
                status,
            });
        }
    }

    pub fn trigger_reconnect(&self) {
        self.reconnect_notify.notify_one();
    }

    pub async fn approve_pairing(&self, peer_id: &str) -> Result<()> {
        let mut map = self.pending_pairings.lock().await;
        if let Some((_code, tx)) = map.remove(peer_id) {
            let _ = tx.send(true);
            let _ = self
                .events_tx
                .send(SyncEvent::PairingAccepted { peer_id: peer_id.to_string() });
            self.reconnect_notify.notify_one();
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

    /// Unpairs a device, removes it from peer store and disconnects active connections.
    pub async fn unpair_peer(&self, peer_id: &str) -> Result<bool> {
        let (removed, peer_info) = {
            let mut store = self.peer_store.lock().await;
            let info = store.get(peer_id).cloned();
            let ok = store.remove(peer_id);
            if ok {
                let _ = store.save();
            }
            (ok, info)
        };
        {
            let mut conns = self.connected_peers.lock().await;
            conns.remove(peer_id);
        }
        if let Some(tx) = {
            let mut senders = self.active_peer_senders.lock().await;
            senders.remove(peer_id)
        } {
            let _ = tx.send(Message::Unpair).await;
        }
        if let Some(cancel_tx) = {
            let mut cancels = self.peer_cancellations.lock().await;
            cancels.remove(peer_id)
        } {
            let _ = cancel_tx.send(());
        }
        let _ = self.events_tx.send(SyncEvent::PeerDisconnected { peer_id: peer_id.to_string() });

        if let Some(p) = peer_info {
            let mut disc = self.discovered_peers.lock().await;
            if !disc.contains_key(peer_id) {
                let addresses = p.last_addr.as_ref().and_then(|a| a.parse().ok()).into_iter().collect();
                disc.insert(
                    peer_id.to_string(),
                    DiscoveredPeer {
                        device_id: p.device_id.clone(),
                        device_name: p.name.clone(),
                        port: self.local_addr.port(),
                        addresses,
                    },
                );
            }
        }
        let is_discovered = self.discovered_peers.lock().await.contains_key(peer_id);
        let status = if is_discovered {
            PeerStatus::Discovered
        } else {
            PeerStatus::Offline
        };
        self.set_peer_status(peer_id, status).await;
        self.reconnect_notify.notify_one();

        if let Some(dp) = self.discovered_peers.lock().await.get(peer_id).cloned() {
            let _ = self.events_tx.send(SyncEvent::PeerDiscovered(dp));
        }

        Ok(removed)
    }

    pub async fn list_discovered_peers(&self) -> Vec<DiscoveredPeer> {
        let disc = self.discovered_peers.lock().await;
        disc.values().cloned().collect()
    }

    pub async fn rescan_peers(self: &Arc<Self>) {
        if let Some(ref mut disc) = *self.discovery.lock().await {
            let device_name = self.config.read().await.device_name.clone();
            let _ = disc.register(&device_name, self.local_addr.port());
        }
        self.scan_subnet_peers().await;
        self.reconnect_notify.notify_one();
        let peers = self.list_discovered_peers().await;
        for peer in peers {
            let _ = self.events_tx.send(SyncEvent::PeerDiscovered(peer));
        }
    }

    /// Actively scans local /24 subnets for peers listening on the sync port.
    /// This acts as a reliable fallback when mDNS multicast is dropped by Wi-Fi routers or firewalls.
    pub async fn scan_subnet_peers(self: &Arc<Self>) {
        let local_ips: Vec<std::net::Ipv4Addr> = if_addrs::get_if_addrs()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|iface| {
                if iface.is_loopback() {
                    return None;
                }
                match iface.addr.ip() {
                    std::net::IpAddr::V4(ipv4)
                        if crate::discovery::is_lan_routable_address(&std::net::IpAddr::V4(ipv4)) =>
                    {
                        Some(ipv4)
                    }
                    _ => None,
                }
            })
            .collect();

        let target_port = self.local_addr.port();
        let cli_cfg = match client_config(&self.identity) {
            Ok(c) => c,
            Err(_) => return,
        };

        for local_ip in local_ips {
            let [a, b, c, host_byte] = local_ip.octets();
            let mut set = tokio::task::JoinSet::new();

            for host in 1..=254 {
                if host == host_byte {
                    continue;
                }
                let target_ip = std::net::Ipv4Addr::new(a, b, c, host);
                let target_addr = SocketAddr::new(std::net::IpAddr::V4(target_ip), target_port);

                // Skip probing if a peer with this IP is already connected
                let is_already_connected = {
                    let disc = self.discovered_peers.lock().await;
                    let conns = self.connected_peers.lock().await;
                    disc.values().any(|p| {
                        p.addresses.iter().any(|addr| addr.ip() == std::net::IpAddr::V4(target_ip))
                            && conns.contains(&p.device_id)
                    })
                };
                if is_already_connected {
                    continue;
                }

                let cli_cfg_clone = cli_cfg.clone();
                let engine = self.clone();

                set.spawn(async move {
                    // Step 1: Fast TCP pre-check (250ms timeout)
                    let precheck = tokio::time::timeout(
                        Duration::from_millis(250),
                        tokio::net::TcpStream::connect(target_addr),
                    )
                    .await;

                    if precheck.is_err() || precheck.unwrap().is_err() {
                        return;
                    }

                    // Step 2: Full TLS handshake & Hello exchange
                    if let Ok(conn) = tokio::time::timeout(
                        Duration::from_millis(1500),
                        connect(target_addr, cli_cfg_clone.clone()),
                    )
                    .await
                    .unwrap_or(Err(anyhow::anyhow!("timeout")))
                    {
                        let (mut r, mut w) = tokio::io::split(conn.stream);
                        let hello = Message::Hello {
                            device_id: engine.identity.device_id.clone(),
                            name: engine.config.read().await.device_name.clone(),
                            proto: PROTO_VERSION,
                            pairing: false,
                            listen_port: engine.local_addr.port(),
                        };

                        if write_msg(&mut w, &hello).await.is_err() {
                            return;
                        }

                        if let Ok(Ok(Message::Hello { device_id, name, listen_port, .. })) =
                            tokio::time::timeout(Duration::from_millis(1500), read_msg(&mut r)).await
                        {
                            if device_id == engine.identity.device_id {
                                return;
                            }

                            let peer = DiscoveredPeer {
                                device_id: device_id.clone(),
                                device_name: name,
                                port: listen_port,
                                addresses: vec![SocketAddr::new(target_addr.ip(), listen_port)],
                            };
                            {
                                let mut disc = engine.discovered_peers.lock().await;
                                if let Some(existing) = disc.get_mut(&device_id) {
                                    for addr in peer.addresses.clone() {
                                        if !existing.addresses.contains(&addr) {
                                            existing.addresses.insert(0, addr);
                                        }
                                    }
                                    existing.device_name = peer.device_name.clone();
                                    existing.port = peer.port;
                                } else {
                                    disc.insert(device_id.clone(), peer.clone());
                                }
                            }
                            info!("Subnet scan discovered peer: {} ({}) at {}", peer.device_name, peer.device_id, target_addr);
                            let _ = engine.events_tx.send(SyncEvent::PeerDiscovered(peer.clone()));

                            // If this peer is already paired and not connected, notify reconnect loop
                            let is_paired = {
                                let store = engine.peer_store.lock().await;
                                store.list().iter().any(|p| p.device_id == peer.device_id)
                            };
                            let is_connected = {
                                let conns = engine.connected_peers.lock().await;
                                conns.contains(&peer.device_id)
                            };

                            if !is_connected {
                                if is_paired {
                                    engine.reconnect_notify.notify_one();
                                } else {
                                    engine.set_peer_status(&peer.device_id, PeerStatus::Discovered).await;
                                }
                            }
                        }
                    }
                });
            }

            while let Some(res) = set.join_next().await {
                let _ = res;
            }
        }
    }

    pub async fn list_peers(&self) -> Vec<Peer> {
        let store = self.peer_store.lock().await;
        store.list().to_vec()
    }

    pub async fn connected_peer_ids(&self) -> HashSet<String> {
        self.connected_peers.lock().await.clone()
    }

    /// Removes tombstone for a file, allowing manual re-sync/restore from peer.
    pub async fn restore_tombstone(&self, path: &str) -> Result<bool> {
        let mut db = self.index_db.lock().await;
        db.remove_tombstone(path)
    }

    pub async fn list_pending_deletions(&self) -> Vec<PendingDeletion> {
        self.pending_deletions.lock().await.clone()
    }

    pub async fn resolve_pending_deletion(&self, path: &str, accept: bool) -> Result<()> {
        let mut list = self.pending_deletions.lock().await;
        list.retain(|item| item.path != path);

        if accept {
            let sync_dir = self.config.read().await.sync_dir.clone();
            let src = crate::paths::to_fs_path(&sync_dir, path);
            if src.exists() {
                let trash_dir = sync_dir.join(".lansync-trash");
                let _ = tokio::fs::create_dir_all(&trash_dir).await;
                let ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
                let fname = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("file");
                let trash_dst = trash_dir.join(format!("{ts}_{fname}"));
                let _ = tokio::fs::rename(&src, &trash_dst).await;
            }

            let mut db = self.index_db.lock().await;
            let del_entry = Entry {
                path: path.to_string(),
                size: 0,
                mtime_ns: 0,
                hash: None,
                vv: VersionVector::default(),
            };
            let _ = db.record_file(&del_entry);
        }
        Ok(())
    }

    pub async fn get_delete_mode(&self) -> crate::config::DeleteMode {
        self.config.read().await.delete_mode
    }

    pub async fn set_delete_mode(&self, mode: crate::config::DeleteMode) -> Result<()> {
        let mut cfg = self.config.write().await;
        cfg.delete_mode = mode;
        cfg.save(&self.data_dir.join("config.json"))?;
        Ok(())
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub async fn set_paused(&self, paused: bool) {
        self.is_paused.store(paused, std::sync::atomic::Ordering::Relaxed);
        if !paused {
            self.trigger_sync_all().await;
        }
    }

    pub fn get_bandwidth_limit_kbps(&self) -> u32 {
        self.bandwidth_limit_kbps.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_bandwidth_limit_kbps(&self, kbps: u32) {
        self.bandwidth_limit_kbps.store(kbps, std::sync::atomic::Ordering::Relaxed);
    }

    pub async fn set_sync_dir(&self, new_dir: PathBuf) -> Result<()> {
        tokio::fs::create_dir_all(&new_dir).await?;
        {
            let mut cfg = self.config.write().await;
            cfg.sync_dir = new_dir.clone();
            cfg.save(&self.data_dir.join("config.json"))?;
        }
        {
            let mut db = self.index_db.lock().await;
            *db = Index::open(&self.data_dir.join("index.db"), &new_dir, &self.identity.device_id)?;
            let _ = db.clear_missing_files_for_resync();
            let _ = db.scan();
        }
        if let Ok(watcher) = FolderWatcher::start(
            &new_dir,
            Duration::from_millis(500),
            self.watcher_tx.clone(),
        ) {
            *self._watcher.lock().await = Some(watcher);
        }
        self.trigger_sync_all().await;
        Ok(())
    }

    pub async fn trigger_sync_all(&self) {
        self.is_paused.store(false, std::sync::atomic::Ordering::Relaxed);
        let local_entries = {
            let mut db = self.index_db.lock().await;
            let _ = db.clear_missing_files_for_resync();
            let _ = db.scan();
            db.entries().unwrap_or_default()
        };
        let senders: Vec<mpsc::Sender<Message>> = {
            let map = self.active_peer_senders.lock().await;
            map.values().cloned().collect()
        };
        for tx in senders {
            for msg in index_messages(local_entries.clone(), true) {
                let _ = tx.send(msg).await;
            }
        }
        let _ = self.resync_tx.send(());
    }

    pub async fn set_peer_delete_mode(&self, peer_id: &str, mode: Option<crate::config::DeleteMode>) -> Result<()> {
        let mut pstore = self.peer_store.lock().await;
        if pstore.set_delete_mode(peer_id, mode) {
            pstore.save()?;
        }
        Ok(())
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

    pub async fn handle_connection(self: Arc<Self>, conn: SecureConn, is_incoming: bool) {
        let peer_cert = conn.peer_cert;
        let peer_id = device_id_from_cert(&peer_cert);
        let peer_fp = fingerprint(&peer_cert);

        if peer_id == self.identity.device_id {
            return;
        }

        info!(
            "Connection established with peer {} at {} (incoming: {})",
            peer_id, conn.peer_addr, is_incoming
        );

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
            store.find_by_fingerprint(&peer_fp).is_some() || store.get(&peer_id).is_some()
        };

        let cfg = self.config.read().await.clone();

        // Step 1: Exchange Hello messages
        let hello_out = Message::Hello {
            device_id: self.identity.device_id.clone(),
            name: cfg.device_name.clone(),
            proto: PROTO_VERSION,
            pairing: !is_paired && !is_incoming,
            listen_port: self.local_addr.port(),
        };
        if let Err(e) = tx_msg.send(hello_out).await {
            warn!("Failed to send Hello message: {:?}", e);
            return;
        }

        let (remote_id, remote_name, pairing_requested, remote_listen_port) = match read_msg(&mut r).await {
            Ok(Message::Hello { device_id, name, proto, pairing, listen_port }) => {
                if proto != PROTO_VERSION {
                    warn!("Incompatible protocol version: {}", proto);
                    return;
                }
                (device_id, name, pairing, listen_port)
            }
            other => {
                warn!("Received invalid initial message: {:?}", other);
                return;
            }
        };

        // Learn remote peer and cache into discovered_peers
        let incoming_discovered = DiscoveredPeer {
            device_id: remote_id.clone(),
            device_name: remote_name.clone(),
            port: remote_listen_port,
            addresses: vec![SocketAddr::new(conn.peer_addr.ip(), remote_listen_port)],
        };
        {
            let mut disc = self.discovered_peers.lock().await;
            disc.insert(remote_id.clone(), incoming_discovered.clone());
        }
        let _ = self.events_tx.send(SyncEvent::PeerDiscovered(incoming_discovered));

        // Pairing mode:
        // 1) Active dial for pairing: local peer initiated connection and is unpaired.
        // 2) Incoming pairing request: remote peer requested pairing (even if local has old entry).
        let pairing_mode = (!is_paired && !is_incoming) || (is_incoming && pairing_requested);

        if pairing_mode {
            let peer_ip = conn.peer_addr.ip().to_string();
            // Check rate limiter only on incoming pairing attempts
            if is_incoming {
                let limiter = self.pairing_limiter.lock().await;
                if let Err(err) = limiter.can_attempt(&peer_ip, std::time::Instant::now()) {
                    warn!("Pairing attempt rejected for peer {}: {}", peer_ip, err);
                    let _ = tx_msg.send(Message::PairDecision { accept: false }).await;
                    return;
                }
            }

            let code = pairing_code(&self.identity.cert_der, &peer_cert);
            info!(
                "Pairing handshake with peer {} ({}) - Confirmation Code: [{}] (incoming: {})",
                peer_id, remote_name, code, is_incoming
            );

            let (accepted, was_explicit_rejection) = if cfg.auto_accept_pairing {
                (true, false)
            } else if !is_incoming {
                // Initiator explicitly clicked 'Pair' in UI, automatically accepted locally!
                (true, false)
            } else {
                let (decision_tx, decision_rx) = oneshot::channel();
                {
                    let mut pend = self.pending_pairings.lock().await;
                    if pend.contains_key(&peer_id) {
                        info!("Pairing handshake already pending for peer {}, ignoring duplicate incoming attempt", peer_id);
                        return;
                    }
                    pend.insert(peer_id.clone(), (code.clone(), decision_tx));
                }

                let _ = self.events_tx.send(SyncEvent::PairingRequested {
                    peer_id: peer_id.clone(),
                    device_name: remote_name.clone(),
                    code: code.clone(),
                });

                // Await decision or 90-second timeout (code TTL)
                let res = match tokio::time::timeout(Duration::from_secs(90), decision_rx).await {
                    Ok(Ok(decision)) => (decision, !decision),
                    _ => (false, false),
                };
                self.pending_pairings.lock().await.remove(&peer_id);
                res
            };

            let _ = tx_msg.send(Message::PairDecision { accept: accepted }).await;

            // Read remote peer decision
            let peer_decision = match read_msg(&mut r).await {
                Ok(Message::PairDecision { accept }) => accept,
                _ => false,
            };

            info!(
                "Pairing handshake for {}: local_accepted={}, remote_accepted={}",
                peer_id, accepted, peer_decision
            );

            if !accepted || !peer_decision {
                info!("Pairing failed or rejected with peer {}", peer_id);
                if was_explicit_rejection {
                    let mut limiter = self.pairing_limiter.lock().await;
                    limiter.record_failure(&peer_ip, std::time::Instant::now());
                }
                return;
            }

            // Success: clear failures from limiter
            {
                let mut limiter = self.pairing_limiter.lock().await;
                limiter.record_success(&peer_ip);
            }

            // Save peer in PeerStore using listening address, NOT ephemeral port
            let remote_listen_addr = SocketAddr::new(conn.peer_addr.ip(), remote_listen_port);
            {
                let mut store = self.peer_store.lock().await;
                store.add(Peer {
                    device_id: peer_id.clone(),
                    name: remote_name.clone(),
                    cert_fingerprint: peer_fp.clone(),
                    last_addr: Some(remote_listen_addr.to_string()),
                    delete_mode: None,
                });
                let _ = store.save();
            }
            let _ = self
                .events_tx
                .send(SyncEvent::PairingAccepted { peer_id: peer_id.clone() });
        } else if !is_paired {
            // Neither side requested pairing and peer is not paired.
            debug!("Unpaired connection with peer {} completed discovery; closing session.", peer_id);
            return;
        }

        // Update last_addr in PeerStore with peer's listening address
        let remote_listen_addr = SocketAddr::new(conn.peer_addr.ip(), remote_listen_port);
        {
            let mut store = self.peer_store.lock().await;
            store.set_addr(&peer_id, &remote_listen_addr.to_string());
            let _ = store.save();
        }

        info!("Peer {} fully authenticated and connected.", peer_id);

        // Anti-dual-connection tie-breaker:
        // Deterministically keep the connection initiated by the active dialer (smaller device_id).
        let my_id = &self.identity.device_id;
        let am_dialer = is_active_dialer(my_id, &peer_id);

        let has_existing = {
            let conns = self.connected_peers.lock().await;
            conns.contains(&peer_id)
        };

        if has_existing {
            if am_dialer && is_incoming {
                info!(
                    "Tie-breaker: Dropping incoming connection from {} because local node is active dialer",
                    peer_id
                );
                return;
            }
            if !am_dialer && !is_incoming {
                info!(
                    "Tie-breaker: Dropping outgoing connection to {} because remote peer is active dialer",
                    peer_id
                );
                return;
            }
            info!("Tie-breaker: Replacing existing connection for peer {}", peer_id);
        }

        // Register connection and cancel previous one if any
        let (cancel_tx, mut cancel_rx) = oneshot::channel();
        {
            let mut cancels = self.peer_cancellations.lock().await;
            if let Some(old_cancel) = cancels.insert(peer_id.clone(), cancel_tx) {
                let _ = old_cancel.send(());
            }
        }
        {
            let mut conns = self.connected_peers.lock().await;
            conns.insert(peer_id.clone());
        }
        {
            let mut senders = self.active_peer_senders.lock().await;
            senders.insert(peer_id.clone(), tx_msg.clone());
        }

        self.set_peer_status(&peer_id, PeerStatus::Connected).await;

        let _ = self.events_tx.send(SyncEvent::PeerConnected {
            peer_id: peer_id.clone(),
            device_name: remote_name.clone(),
        });
        let _ = self
            .events_tx
            .send(SyncEvent::SyncStarted { peer_id: peer_id.clone() });

        // Send local index to remote peer (fresh disk scan ensures any new files are synced immediately)
        let local_entries = {
            let mut db = self.index_db.lock().await;
            let _ = db.scan();
            db.entries().unwrap_or_default()
        };
        for msg in index_messages(local_entries, true) {
            let _ = tx_msg.send(msg).await;
        }

        let mut remote_entries: HashMap<String, Entry> = HashMap::new();
        let mut download_queue: VecDeque<QueuedDownload> = VecDeque::new();
        let mut expected_expected_local: HashMap<String, Option<Hash>> = HashMap::new();
        let mut current_incoming: Option<Incoming> = None;
        let mut current_incoming_path: Option<String> = None;

        let mut last_activity = tokio::time::Instant::now();
        let mut consecutive_missed_pings: u32 = 0;
        let mut heartbeat_ticker = tokio::time::interval(Duration::from_secs(5));
        heartbeat_ticker.tick().await;
        let mut resync_rx = self.resync_tx.subscribe();

        // Spawn a dedicated reader task so read_msg(&mut r) is never cancelled mid-frame by select! branches
        let (tx_in, mut rx_in) = mpsc::channel::<Result<Message>>(64);
        let reader_task = tokio::spawn(async move {
            loop {
                let res = read_msg(&mut r).await;
                let is_err = res.is_err();
                if tx_in.send(res).await.is_err() || is_err {
                    break;
                }
            }
        });

        // Loop to read incoming messages from peer
        loop {
            let msg = tokio::select! {
                biased;
                Ok(()) = &mut cancel_rx => {
                    info!("Connection cancelled locally for peer {}", peer_id);
                    break;
                }
                Ok(()) = resync_rx.recv() => {
                    if !remote_entries.is_empty() {
                        self.execute_plan(
                            &peer_id,
                            &remote_entries,
                            &tx_msg,
                            &mut expected_expected_local,
                            &mut download_queue,
                            current_incoming.is_some(),
                        )
                        .await;
                    }
                    continue;
                }
                _ = heartbeat_ticker.tick() => {
                    let elapsed = last_activity.elapsed();
                    if elapsed >= Duration::from_secs(15) {
                        if consecutive_missed_pings >= 2 {
                            warn!("Peer {} heartbeat timeout (2 missed pings). Disconnecting.", peer_id);
                            break;
                        }
                        if let Err(e) = tx_msg.send(Message::Ping).await {
                            debug!("Peer {} ping send error: {:?}", peer_id, e);
                            break;
                        }
                        consecutive_missed_pings += 1;
                    }
                    continue;
                }
                res = rx_in.recv() => {
                    match res {
                        Some(Ok(m)) => {
                            last_activity = tokio::time::Instant::now();
                            consecutive_missed_pings = 0;
                            m
                        }
                        Some(Err(e)) => {
                            info!("Peer {} disconnected / read error: {:?}", peer_id, e);
                            break;
                        }
                        None => {
                            info!("Peer {} reader stream closed", peer_id);
                            break;
                        }
                    }
                }
            };

            match msg {
                Message::Unpair => {
                    info!("Remote peer {} requested unpair. Removing peer and disconnecting.", peer_id);
                    let removed_peer = {
                        let mut store = self.peer_store.lock().await;
                        let p = store.get(&peer_id).cloned();
                        store.remove(&peer_id);
                        let _ = store.save();
                        p
                    };
                    if let Some(p) = removed_peer {
                        let mut disc = self.discovered_peers.lock().await;
                        if !disc.contains_key(&peer_id) {
                            let addresses = p.last_addr.as_ref().and_then(|a| a.parse().ok()).into_iter().collect();
                            disc.insert(
                                peer_id.clone(),
                                DiscoveredPeer {
                                    device_id: p.device_id.clone(),
                                    device_name: p.name.clone(),
                                    port: self.local_addr.port(),
                                    addresses,
                                },
                            );
                        }
                    }
                    if let Some(dp) = self.discovered_peers.lock().await.get(&peer_id).cloned() {
                        let _ = self.events_tx.send(SyncEvent::PeerDiscovered(dp));
                    }
                    break;
                }
                Message::Index { entries } => {
                    info!("Received full Index from peer {} with {} entries", peer_id, entries.len());
                    remote_entries.clear();
                    for e in entries {
                        remote_entries.insert(e.path.clone(), e);
                    }
                    self.execute_plan(
                        &peer_id,
                        &remote_entries,
                        &tx_msg,
                        &mut expected_expected_local,
                        &mut download_queue,
                        current_incoming.is_some(),
                    )
                    .await;
                }
                Message::IndexUpdate { entries } => {
                    info!("Received IndexUpdate from peer {} with {} entries", peer_id, entries.len());
                    for e in entries {
                        remote_entries.insert(e.path.clone(), e);
                    }
                    self.execute_plan(
                        &peer_id,
                        &remote_entries,
                        &tx_msg,
                        &mut expected_expected_local,
                        &mut download_queue,
                        current_incoming.is_some(),
                    )
                    .await;
                }
                Message::GetFile { path, hash } => {
                    let sync_dir = self.config.read().await.sync_dir.clone();
                    let tx_chunk = tx_msg.clone();
                    let limit_kbps = self.get_bandwidth_limit_kbps();
                    tokio::spawn(async move {
                        send_file_to_peer(sync_dir, path, hash, 0, tx_chunk, limit_kbps).await;
                    });
                }
                Message::GetFileRange { path, hash, offset } => {
                    let sync_dir = self.config.read().await.sync_dir.clone();
                    let tx_chunk = tx_msg.clone();
                    let limit_kbps = self.get_bandwidth_limit_kbps();
                    tokio::spawn(async move {
                        send_file_to_peer(sync_dir, path, hash, offset, tx_chunk, limit_kbps).await;
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
                Message::FileHeaderRange { path, size, hash, mtime_ns, offset } => {
                    let sync_dir = self.config.read().await.sync_dir.clone();
                    let _ = self.events_tx.send(SyncEvent::FileDownloading {
                        rel_path: path.clone(),
                        size,
                    });
                    current_incoming_path = Some(path.clone());
                    current_incoming = Some(Incoming::begin_resumable(&sync_dir, &path, size, hash, mtime_ns, offset).await);
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
                            Ok(mut entry) => {
                                if let Some(re) = remote_entries.get(&rel) {
                                    entry.vv = re.vv.clone();
                                }
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
                    let sync_dir = self.config.read().await.sync_dir.clone();
                    trigger_next_download(
                        &sync_dir,
                        &mut download_queue,
                        &mut expected_expected_local,
                        &remote_entries,
                        &tx_msg,
                    )
                    .await;
                }
                Message::FileUnavailable { path } => {
                    debug!("Peer reported file {} unavailable", path);
                    let sync_dir = self.config.read().await.sync_dir.clone();
                    trigger_next_download(
                        &sync_dir,
                        &mut download_queue,
                        &mut expected_expected_local,
                        &remote_entries,
                        &tx_msg,
                    )
                    .await;
                }
                Message::Ping => {
                    let _ = tx_msg.send(Message::Pong).await;
                }
                Message::Pong => {}
                _ => {}
            }
        }

        reader_task.abort();

        // Cleanup upon disconnection only if this connection's sender is still active
        let was_active = {
            let mut senders = self.active_peer_senders.lock().await;
            if senders.get(&peer_id).is_some_and(|s| s.same_channel(&tx_msg)) {
                senders.remove(&peer_id);
                let mut conns = self.connected_peers.lock().await;
                conns.remove(&peer_id);
                let mut cancels = self.peer_cancellations.lock().await;
                cancels.remove(&peer_id);
                true
            } else {
                false
            }
        };

        if was_active {
            let is_paired = {
                let store = self.peer_store.lock().await;
                store.get(&peer_id).is_some()
            };
            let status = if is_paired {
                PeerStatus::Backoff
            } else {
                PeerStatus::Offline
            };
            self.set_peer_status(&peer_id, status).await;

            let _ = self.events_tx.send(SyncEvent::PeerDisconnected {
                peer_id: peer_id.clone(),
            });

            self.reconnect_notify.notify_one();
        }
        drop(tx_msg);
        let _ = writer_task.await;
    }

    async fn execute_plan(
        &self,
        peer_id: &str,
        remote_entries: &HashMap<String, Entry>,
        tx_msg: &mpsc::Sender<Message>,
        expected_local_map: &mut HashMap<String, Option<Hash>>,
        download_queue: &mut VecDeque<QueuedDownload>,
        is_downloading: bool,
    ) {
        if self.is_paused() {
            return;
        }

        let sync_dir = self.config.read().await.sync_dir.clone();
        let (mut local_entries, mut bases, mut tombstones) = {
            let mut db = self.index_db.lock().await;
            let entries = db.entries().unwrap_or_default();
            // Detect if any indexed active file is missing on disk (e.g. folder cleared without .lansync-root)
            let any_active_missing_on_disk = entries
                .iter()
                .any(|e| e.hash.is_some() && !to_fs_path(&sync_dir, &e.path).exists());
            let marker_missing = sync_dir.is_dir() && !sync_dir.join(".lansync-root").exists();
            if any_active_missing_on_disk || marker_missing {
                let _ = db.clear_missing_files_for_resync();
                let _ = db.scan();
            }
            (
                db.entries().unwrap_or_default(),
                db.bases(peer_id).unwrap_or_default(),
                db.tombstones().unwrap_or_default(),
            )
        };

        // If the local sync folder has 0 active files on disk, allow all remote files to download cleanly
        let has_any_local_file = local_entries.iter().any(|e| e.hash.is_some());
        if !has_any_local_file && remote_entries.values().any(|e| e.hash.is_some()) {
            local_entries.clear();
            bases.clear();
            tombstones.clear();
        }

        // Filter out tombstones if an in-progress partial download (.lansync-part-*.tmp) exists on disk
        tombstones.retain(|path, hash| {
            if let Some(part) = crate::transfer::partial_download_path(&sync_dir, path, hash) {
                !part.exists()
            } else {
                true
            }
        });

        let my_id = self.identity.device_id.clone();
        let i_win = my_id.as_str() < peer_id;
        let my_name = self.config.read().await.device_name.clone();
        let stamp = Local::now().format("%Y-%m-%d %H%M").to_string();

        let remote_vec: Vec<Entry> = remote_entries.values().cloned().collect();
        let delete_mode = {
            let pstore = self.peer_store.lock().await;
            pstore
                .list()
                .iter()
                .find(|p| p.device_id == peer_id)
                .and_then(|p| p.delete_mode)
                .unwrap_or(self.config.read().await.delete_mode)
        };
        let actions = plan(&PlanInput {
            local: &local_entries,
            remote: &remote_vec,
            base: &bases,
            tombstones: &tombstones,
            i_win_conflicts: i_win,
            my_name: &my_name,
            stamp: &stamp,
            delete_mode,
        });
        info!("execute_plan with peer {}: planned {} actions: {:?}", peer_id, actions.len(), actions);

        let sync_dir = self.config.read().await.sync_dir.clone();

        // Verify sufficient disk space for all planned downloads
        let total_download_bytes: u64 = actions
            .iter()
            .filter_map(|a| match a {
                Action::Download { remote_path, .. } => {
                    remote_entries.get(remote_path).map(|e| e.size)
                }
                _ => None,
            })
            .sum();

        let available_space = if total_download_bytes > 0 {
            crate::fsutil::available_disk_space(&sync_dir).ok()
        } else {
            None
        };

        if let Some(available) = available_space {
            let required = total_download_bytes.saturating_add(50 * 1024 * 1024);
            if available < required {
                warn!(
                    "Insufficient disk space for sync: required {} bytes, available {} bytes. Skipping download batch.",
                    required, available
                );
                return;
            }
        }

        for action in actions {
            match action {
                Action::Download { local_path, remote_path, hash, expected_local } => {
                    if !download_queue.iter().any(|q| q.local_path == local_path) {
                        download_queue.push_back(QueuedDownload {
                            local_path,
                            remote_path,
                            hash,
                            expected_local,
                        });
                    }
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
                        if let (Ok(meta), Ok(h)) = (tokio::fs::metadata(&dst).await, hash_file(&dst)) {
                            let entry = Entry {
                                path: conflict_path.clone(),
                                size: meta.len(),
                                mtime_ns: crate::fsutil::mtime_ns(&meta),
                                hash: Some(h),
                                vv: VersionVector::default(),
                            };
                            let mut db = self.index_db.lock().await;
                            let _ = db.record_file(&entry);
                        }
                        let _ = self.events_tx.send(SyncEvent::ConflictCreated {
                            original: local_path.clone(),
                            conflict: conflict_path,
                        });
                    }

                    // Queue download remote copy to original filename
                    if !download_queue.iter().any(|q| q.local_path == local_path) {
                        download_queue.push_back(QueuedDownload {
                            local_path,
                            remote_path,
                            hash,
                            expected_local: None,
                        });
                    }
                }
                Action::Rename { from_path, to_path, hash } => {
                    let src = to_fs_path(&sync_dir, &from_path);
                    let dst = to_fs_path(&sync_dir, &to_path);
                    if let Some(parent) = dst.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    if let Ok(()) = tokio::fs::rename(&src, &dst).await {
                        tracing::info!("Renamed local file {} -> {} (zero network transfer)", from_path, to_path);
                        if let Ok(meta) = tokio::fs::metadata(&dst).await {
                            let mut db = self.index_db.lock().await;
                            let old_entry = Entry {
                                path: from_path.clone(),
                                size: 0,
                                mtime_ns: 0,
                                hash: None,
                                vv: VersionVector::default(),
                            };
                            let _ = db.record_file(&old_entry);
                            let new_entry = Entry {
                                path: to_path.clone(),
                                size: meta.len(),
                                mtime_ns: crate::fsutil::mtime_ns(&meta),
                                hash: Some(hash),
                                vv: match remote_entries.get(&to_path) {
                                    Some(re) => re.vv.clone(),
                                    None => VersionVector::default(),
                                },
                            };
                            let _ = db.record_file(&new_entry);
                            let _ = db.set_base(peer_id, &to_path, &hash);
                        }
                    } else {
                        // Fallback to normal download if rename fails
                        if !download_queue.iter().any(|q| q.local_path == to_path) {
                            download_queue.push_back(QueuedDownload {
                                local_path: to_path.clone(),
                                remote_path: to_path,
                                hash,
                                expected_local: None,
                            });
                        }
                    }
                }
                Action::Trash { path } => {
                    let src = to_fs_path(&sync_dir, &path);
                    if src.exists() {
                        let trash_dir = sync_dir.join(".lansync-trash");
                        let _ = tokio::fs::create_dir_all(&trash_dir).await;
                        let ts = Local::now().format("%Y%m%d_%H%M%S").to_string();
                        let fname = std::path::Path::new(&path).file_name().and_then(|n| n.to_str()).unwrap_or("file");
                        let trash_dst = trash_dir.join(format!("{ts}_{fname}"));
                        if let Ok(()) = tokio::fs::rename(&src, &trash_dst).await {
                            tracing::info!("Moved file to trash: {} -> {:?}", path, trash_dst);
                            let _ = self.events_tx.send(SyncEvent::FileTrashed {
                                rel_path: path.clone(),
                                trash_path: trash_dst.to_string_lossy().to_string(),
                            });
                        }
                    }
                    let mut db = self.index_db.lock().await;
                    let del_entry = Entry {
                        path: path.clone(),
                        size: 0,
                        mtime_ns: 0,
                        hash: None,
                        vv: match remote_entries.get(&path) {
                            Some(re) => re.vv.clone(),
                            None => VersionVector::default(),
                        },
                    };
                    let _ = db.record_file(&del_entry);
                }
                Action::AskDelete { path } => {
                    let mut pending = self.pending_deletions.lock().await;
                    if !pending.iter().any(|item| item.path == path) {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        pending.push(PendingDeletion {
                            peer_id: peer_id.to_string(),
                            path: path.clone(),
                            timestamp: now,
                        });
                        let _ = self.events_tx.send(SyncEvent::PendingDeletion {
                            peer_id: peer_id.to_string(),
                            rel_path: path,
                        });
                    }
                }
            }
        }

        if !is_downloading {
            trigger_next_download(
                &sync_dir,
                download_queue,
                expected_local_map,
                remote_entries,
                tx_msg,
            )
            .await;
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
        let _ = tokio::fs::write(&self.data_dir.join(".clean_shutdown"), b"clean\n").await;
        Ok(())
    }
}

async fn send_file_to_peer(
    sync_dir: PathBuf,
    path: String,
    hash: Hash,
    offset: u64,
    tx_chunk: mpsc::Sender<Message>,
    limit_kbps: u32,
) {
    let full = to_fs_path(&sync_dir, &path);
    if let Ok(meta) = tokio::fs::metadata(&full).await {
        let mtime = crate::fsutil::mtime_ns(&meta);
        let size = meta.len();
        if offset > 0 {
            let _ = tx_chunk
                .send(Message::FileHeaderRange {
                    path: path.clone(),
                    size,
                    hash,
                    mtime_ns: mtime,
                    offset,
                })
                .await;
        } else {
            let _ = tx_chunk
                .send(Message::FileHeader {
                    path: path.clone(),
                    size,
                    hash,
                    mtime_ns: mtime,
                })
                .await;
        }

        if let Ok(std_file) = crate::fsutil::open_with_retry(&full, Duration::from_secs(5)).await {
            let mut f = tokio::fs::File::from_std(std_file);
            use tokio::io::{AsyncReadExt, AsyncSeekExt};
            if offset > 0 {
                let _ = f.seek(std::io::SeekFrom::Start(offset)).await;
            }
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
                        if limit_kbps > 0 {
                            let delay_micros = (n as u64 * 1_000_000) / (limit_kbps as u64 * 1024);
                            tokio::time::sleep(Duration::from_micros(delay_micros)).await;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        let _ = tx_chunk.send(Message::FileEnd).await;
    } else {
        let _ = tx_chunk.send(Message::FileUnavailable { path }).await;
    }
}

