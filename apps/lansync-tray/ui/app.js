// LanSync Desktop Tray Controller
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// Application State
let currentStatus = null;
let currentPairingPeerId = null;
const knownPeers = new Map();

// Elements
const elStatusDot = document.getElementById('status-dot');
const elStatusText = document.getElementById('status-text');
const elBtnSyncNow = document.getElementById('btn-sync-now');
const elBtnPauseSync = document.getElementById('btn-pause-sync');
const elDeviceName = document.getElementById('device-name');
const elDevicePort = document.getElementById('device-port');
const elDeviceIdBadge = document.getElementById('device-id-badge');
const elDeviceIdShort = document.getElementById('device-id-short');
const elFolderPath = document.getElementById('folder-path');
const elSettingSyncDirDisplay = document.getElementById('setting-sync-dir-display');

const elBtnQuickFolder = document.getElementById('btn-quick-folder');
const elBtnOpenFolder = document.getElementById('btn-open-folder');
const elBtnBrowseFolderQuick = document.getElementById('btn-browse-folder-quick');
const elBtnBrowseFolder = document.getElementById('btn-browse-folder');

const elSyncProgressBox = document.getElementById('sync-progress-box');
const elProgressFilename = document.getElementById('progress-filename');
const elProgressPercent = document.getElementById('progress-percent');
const elProgressBarFill = document.getElementById('progress-bar-fill');

const elPairingCard = document.getElementById('pairing-card');
const elPairingPeerName = document.getElementById('pairing-peer-name');
const elPairingCodeDisplay = document.getElementById('pairing-code-display');
const elBtnApprovePairing = document.getElementById('btn-approve-pairing');
const elBtnRejectPairing = document.getElementById('btn-reject-pairing');

const elConflictsCard = document.getElementById('conflicts-card');
const elConflictsCountTitle = document.getElementById('conflicts-count-title');
const elConflictsList = document.getElementById('conflicts-list');

const elPendingDeletionsCard = document.getElementById('pending-deletions-card');
const elPendingDeletionsCountTitle = document.getElementById('pending-deletions-title');
const elPendingDeletionsList = document.getElementById('pending-deletions-list');
const elSettingDeleteMode = document.getElementById('setting-delete-mode');
const elSettingBandwidthLimit = document.getElementById('setting-bandwidth-limit');

const elVersionsFileList = document.getElementById('versions-file-list');
const elVersionDetailsCard = document.getElementById('version-details-card');
const elVersionSelectedFile = document.getElementById('version-selected-file');
const elBtnCloseVersionDetails = document.getElementById('btn-close-version-details');
const elVersionSnapshotsList = document.getElementById('version-snapshots-list');

const elPeersList = document.getElementById('peers-list');
const elPeersSummary = document.getElementById('peers-summary');
const elTabDevicesBadge = document.getElementById('tab-devices-badge');
const elManualIpInput = document.getElementById('manual-ip-input');
const elBtnManualPair = document.getElementById('btn-manual-pair');

const elActivitiesList = document.getElementById('activities-list');
const elFooterSummary = document.getElementById('footer-summary');
const elBtnCopyId = document.getElementById('btn-copy-id');

let isPaused = false;
let selectedVersionFile = null;

// Tabs
const tabButtons = document.querySelectorAll('.tab-item');
const tabPanels = document.querySelectorAll('.tab-panel');

async function init() {
  setupTabs();
  setupEventListeners();
  await initDeleteMode();
  await initBandwidthLimit();
  await initPauseStatus();
  await refreshStatus();
  await refreshVersions();
  await setupSyncEventSubscriber();

  // Lightweight fallback polling (UI is driven by real-time events)
  setInterval(refreshStatus, 30000);
  setInterval(refreshVersions, 10000);
}

// Tab Switching
function setupTabs() {
  tabButtons.forEach(btn => {
    btn.addEventListener('click', () => {
      const targetId = btn.getAttribute('data-tab');

      tabButtons.forEach(b => b.classList.remove('active'));
      tabPanels.forEach(p => p.classList.remove('active'));

      btn.classList.add('active');
      const targetPanel = document.getElementById(targetId);
      if (targetPanel) {
        targetPanel.classList.add('active');
      }
    });
  });
}

function setupEventListeners() {
  // Open folder triggers
  const handleOpenFolder = async () => {
    try {
      await invoke('open_sync_folder');
    } catch (err) {
      console.error('Không thể mở thư mục:', err);
    }
  };

  elBtnQuickFolder?.addEventListener('click', handleOpenFolder);
  elBtnOpenFolder?.addEventListener('click', handleOpenFolder);

  // Pick folder triggers
  const handlePickFolder = async () => {
    try {
      const selected = await invoke('pick_sync_folder');
      if (selected) {
        if (elFolderPath) elFolderPath.innerText = selected;
        if (elSettingSyncDirDisplay) elSettingSyncDirDisplay.innerText = selected;
        await refreshStatus();
      }
    } catch (err) {
      console.error('Lỗi chọn thư mục:', err);
    }
  };

  elBtnBrowseFolderQuick?.addEventListener('click', handlePickFolder);
  elBtnBrowseFolder?.addEventListener('click', handlePickFolder);

  // Copy Device ID
  const copyDeviceId = () => {
    if (currentStatus?.device_id) {
      navigator.clipboard.writeText(currentStatus.device_id);
      const originalText = elDeviceIdShort.innerText;
      elDeviceIdShort.innerText = 'Đã sao chép';
      setTimeout(() => {
        elDeviceIdShort.innerText = originalText;
      }, 1400);
    }
  };

  elDeviceIdBadge?.addEventListener('click', copyDeviceId);
  elBtnCopyId?.addEventListener('click', copyDeviceId);

  // Pairing actions
  elBtnApprovePairing?.addEventListener('click', async () => {
    try {
      await invoke('approve_pairing', { peerId: currentPairingPeerId || null });
      hidePairingCard();
      await refreshStatus();
    } catch (err) {
      alert('Không thể xác nhận ghép đôi: ' + err);
    }
  });

  elBtnRejectPairing?.addEventListener('click', async () => {
    try {
      await invoke('reject_pairing', { peerId: currentPairingPeerId || null });
      hidePairingCard();
      await refreshStatus();
    } catch (err) {
      console.error('Lỗi từ chối ghép đôi:', err);
    }
  });

  // Manual IP Connect
  elBtnManualPair?.addEventListener('click', async () => {
    const addr = elManualIpInput.value.trim();
    if (!addr) return;

    try {
      elBtnManualPair.disabled = true;
      elBtnManualPair.innerText = 'Đang gọi...';
      await invoke('pair_device', { addr });
      elManualIpInput.value = '';
      await refreshStatus();
    } catch (err) {
      alert('Không thể kết nối tới ' + addr + ': ' + err);
    } finally {
      elBtnManualPair.disabled = false;
      elBtnManualPair.innerText = 'Kết nối';
    }
  });

  // Delete mode setting
  elSettingDeleteMode?.addEventListener('change', async (e) => {
    try {
      await invoke('set_delete_mode', { mode: e.target.value });
    } catch (err) {
      alert('Không thể lưu cài đặt chế độ xoá: ' + err);
    }
  });

  // Manual Sync trigger
  elBtnSyncNow?.addEventListener('click', async () => {
    try {
      elBtnSyncNow.disabled = true;
      await invoke('trigger_manual_sync');
      await refreshStatus();
    } catch (err) {
      console.error('Trigger sync error:', err);
    } finally {
      setTimeout(() => { if (elBtnSyncNow) elBtnSyncNow.disabled = false; }, 800);
    }
  });

  // Pause / Resume Sync
  elBtnPauseSync?.addEventListener('click', async () => {
    try {
      const nextPaused = !isPaused;
      await invoke('set_sync_paused', { paused: nextPaused });
      isPaused = nextPaused;
      updatePauseUI(isPaused);
      await refreshStatus();
    } catch (err) {
      console.error('Toggle pause error:', err);
    }
  });

  // Bandwidth Limit setting
  elSettingBandwidthLimit?.addEventListener('change', async (e) => {
    try {
      const limitKbps = parseInt(e.target.value, 10) || 0;
      await invoke('set_bandwidth_limit', { limitKbps });
    } catch (err) {
      console.error('Set bandwidth limit error:', err);
    }
  });

  // Close version details
  elBtnCloseVersionDetails?.addEventListener('click', () => {
    elVersionDetailsCard?.classList.add('hidden');
    selectedVersionFile = null;
  });
}

// Refresh Status from Rust Backend
async function refreshStatus() {
  try {
    const status = await invoke('get_status');
    currentStatus = status;

    if (elDeviceName) elDeviceName.innerText = status.device_name;
    if (elDevicePort) elDevicePort.innerText = status.listen_port;

    const fullAddr = (status.local_ip ? status.local_ip : '127.0.0.1') + ':' + status.listen_port;
    const elDeviceLanAddr = document.getElementById('device-lan-addr');
    if (elDeviceLanAddr) elDeviceLanAddr.innerText = fullAddr;

    const elManualSelfAddr = document.getElementById('manual-self-addr');
    if (elManualSelfAddr) elManualSelfAddr.innerText = fullAddr;

    if (status.device_id) {
      const shortId = status.device_id.length > 8 ? status.device_id.substring(0, 8) : status.device_id;
      if (elDeviceIdShort && elDeviceIdShort.innerText !== 'Đã sao chép') {
        elDeviceIdShort.innerText = shortId;
      }
    }

    if (elFolderPath) elFolderPath.innerText = status.sync_dir;
    if (elSettingSyncDirDisplay) elSettingSyncDirDisplay.innerText = status.sync_dir;

    // Pending pairing notification
    if (status.pending_pairing) {
      showPairingCard(
        status.pending_pairing.peer_id,
        status.pending_pairing.device_name,
        status.pending_pairing.code
      );
    } else if (currentPairingPeerId) {
      hidePairingCard();
    }

    renderActivities(status.activities || []);
    await refreshPeers();
    await refreshConflicts();
    await refreshPendingDeletions();
  } catch (err) {
    console.error('Error refreshing status:', err);
  }
}

// Real-time events from Core Engine
async function setupSyncEventSubscriber() {
  await listen('sync_event', (event) => {
    const data = event.payload;
    handleSyncEvent(data);
  });

  await listen('peer_status', (event) => {
    const payload = event.payload || {};
    const peerId = payload.peer_id;
    const status = payload.status;
    if (!peerId) return;

    if (pairedPeersMap.has(peerId)) {
      const p = pairedPeersMap.get(peerId);
      p.status = status;
      p.is_connected = (status === 'connected');
    }
    if (knownPeers.has(peerId)) {
      const p = knownPeers.get(peerId);
      p.status = status;
      p.connected = (status === 'connected');
    }
    renderPeers();
  });
}

function handleSyncEvent(event) {
  if (!event) return;

  if (event.PeerDiscovered) {
    const peer = event.PeerDiscovered;
    knownPeers.set(peer.device_id, {
      ...peer,
      connected: false,
    });
    renderPeers();
  } else if (event.PairingRequested) {
    const { peer_id, device_name, code } = event.PairingRequested;
    showPairingCard(peer_id, device_name, code);
  } else if (event.PairingAccepted) {
    hidePairingCard();
    refreshStatus();
  } else if (event.PeerConnected) {
    const { peer_id, device_name } = event.PeerConnected;
    const existing = knownPeers.get(peer_id) || { device_id: peer_id, device_name, addresses: [] };
    existing.connected = true;
    existing.status = 'connected';
    knownPeers.set(peer_id, existing);
    if (pairedPeersMap.has(peer_id)) {
      const p = pairedPeersMap.get(peer_id);
      p.is_connected = true;
      p.status = 'connected';
    }
    renderPeers();
    setSyncState('idle');
  } else if (event.PeerDisconnected) {
    const { peer_id } = event.PeerDisconnected;
    const existing = knownPeers.get(peer_id);
    if (existing) {
      existing.connected = false;
      existing.status = 'backoff';
    }
    if (pairedPeersMap.has(peer_id)) {
      const p = pairedPeersMap.get(peer_id);
      p.is_connected = false;
      p.status = 'backoff';
    }
    renderPeers();
    setSyncState('idle');
  } else if (event.PeerStatusChanged) {
    const { peer_id, status } = event.PeerStatusChanged;
    if (pairedPeersMap.has(peer_id)) {
      const p = pairedPeersMap.get(peer_id);
      p.status = status;
      p.is_connected = (status === 'connected');
    }
    if (knownPeers.has(peer_id)) {
      const p = knownPeers.get(peer_id);
      p.status = status;
      p.connected = (status === 'connected');
    }
    renderPeers();
  } else if (event.FileDownloading) {
    const { rel_path, size } = event.FileDownloading;
    setSyncState('syncing');
    showProgress(rel_path, formatBytes(size));
  } else if (event.FileDownloaded || event.SyncCompleted) {
    hideProgress();
    setSyncState('idle');
    refreshStatus();
  }
}

function setSyncState(state) {
  if (state === 'syncing') {
    if (elStatusDot) elStatusDot.className = 'dot dot-syncing';
    if (elStatusText) elStatusText.innerText = 'Đang đồng bộ...';
    if (elFooterSummary) elFooterSummary.innerText = 'Đang truyền dữ liệu';
  } else {
    if (elStatusDot) elStatusDot.className = 'dot dot-idle';
    if (elStatusText) elStatusText.innerText = 'Đã đồng bộ';
    if (elFooterSummary) elFooterSummary.innerText = 'Sẵn sàng';
  }
}

function showProgress(filename, sizeText) {
  if (elSyncProgressBox) elSyncProgressBox.classList.remove('hidden');
  if (elProgressFilename) elProgressFilename.innerText = filename;
  if (elProgressPercent) elProgressPercent.innerText = sizeText;
}

function hideProgress() {
  if (elSyncProgressBox) elSyncProgressBox.classList.add('hidden');
}

function showPairingCard(peerId, name, code) {
  currentPairingPeerId = peerId;
  if (elPairingPeerName) elPairingPeerName.innerText = name;
  if (elPairingCodeDisplay) elPairingCodeDisplay.innerText = code;
  if (elPairingCard) elPairingCard.classList.remove('hidden');
}

function hidePairingCard() {
  currentPairingPeerId = null;
  if (elPairingCard) elPairingCard.classList.add('hidden');
}

// Peer and Conflict State
const pairedPeersMap = new Map();

async function refreshPeers() {
  try {
    const peers = await invoke('list_peers');
    pairedPeersMap.clear();
    for (const p of (peers || [])) {
      pairedPeersMap.set(p.device_id, p);
    }

    // Fetch discovered LAN peers from engine cache
    try {
      const discovered = await invoke('list_discovered_peers');
      for (const dp of (discovered || [])) {
        if (!knownPeers.has(dp.device_id)) {
          knownPeers.set(dp.device_id, {
            ...dp,
            connected: false,
          });
        } else {
          const existing = knownPeers.get(dp.device_id);
          if (dp.addresses && dp.addresses.length) existing.addresses = dp.addresses;
          if (dp.device_name) existing.device_name = dp.device_name;
        }
      }
    } catch (e) {
      console.warn('Failed to fetch discovered peers:', e);
    }

    renderPeers();
  } catch (err) {
    console.error('Error fetching peers:', err);
  }
}

async function refreshConflicts() {
  if (!elConflictsCard || !elConflictsList) return;
  try {
    const conflicts = await invoke('list_conflicts');
    if (!conflicts || conflicts.length === 0) {
      elConflictsCard.classList.add('hidden');
      return;
    }

    elConflictsCard.classList.remove('hidden');
    if (elConflictsCountTitle) {
      elConflictsCountTitle.innerText = `Xung đột file (${conflicts.length})`;
    }

    elConflictsList.innerHTML = conflicts.map(c => `
      <div class="conflict-item">
        <div class="conflict-file-row">
          <span class="conflict-path-name" title="${escapeHtml(c.conflict_path)}">${escapeHtml(c.original_path)}</span>
          <span class="conflict-meta">${formatBytes(c.size)}</span>
        </div>
        <div class="conflict-meta">Sửa đổi lúc: ${escapeHtml(c.modified_at)}</div>
        <div class="conflict-actions-row">
          <button class="btn btn-primary btn-xs" onclick="window.resolveConflict('${escapeAttr(c.conflict_path)}', 'keep_mine')">Giữ bản máy này</button>
          <button class="btn btn-outline btn-xs" onclick="window.resolveConflict('${escapeAttr(c.conflict_path)}', 'keep_theirs')">Giữ bản máy kia</button>
          <button class="btn btn-subtle btn-xs" onclick="window.resolveConflict('${escapeAttr(c.conflict_path)}', 'keep_both')">Giữ cả hai</button>
        </div>
      </div>
    `).join('');
  } catch (err) {
    console.error('Error fetching conflicts:', err);
  }
}

window.resolveConflict = async (conflictPath, resolution) => {
  try {
    await invoke('resolve_conflict', { conflictPath, resolution });
    await refreshConflicts();
    await refreshStatus();
  } catch (err) {
    alert('Không thể giải quyết xung đột: ' + err);
  }
};

async function initDeleteMode() {
  if (!elSettingDeleteMode) return;
  try {
    const mode = await invoke('get_delete_mode');
    if (mode) {
      elSettingDeleteMode.value = mode;
    }
  } catch (err) {
    console.error('Error fetching delete mode:', err);
  }
}

async function refreshPendingDeletions() {
  if (!elPendingDeletionsCard || !elPendingDeletionsList) return;
  try {
    const items = await invoke('list_pending_deletions');
    if (!items || items.length === 0) {
      elPendingDeletionsCard.classList.add('hidden');
      return;
    }

    elPendingDeletionsCard.classList.remove('hidden');
    if (elPendingDeletionsCountTitle) {
      elPendingDeletionsCountTitle.innerText = `Yêu cầu xoá từ thiết bị (${items.length})`;
    }

    elPendingDeletionsList.innerHTML = items.map(item => `
      <div class="conflict-item">
        <div class="conflict-file-row">
          <span class="conflict-path-name" title="${escapeHtml(item.path)}">${escapeHtml(item.path)}</span>
          <span class="conflict-meta">Từ máy: ${escapeHtml(item.peer_id.substring(0, 8))}...</span>
        </div>
        <div class="conflict-actions-row">
          <button class="btn btn-danger-subtle btn-xs" onclick="window.resolvePendingDeletion('${escapeAttr(item.path)}', true)">Chuyển vào Thùng rác</button>
          <button class="btn btn-outline btn-xs" onclick="window.resolvePendingDeletion('${escapeAttr(item.path)}', false)">Giữ lại file</button>
        </div>
      </div>
    `).join('');
  } catch (err) {
    console.error('Error fetching pending deletions:', err);
  }
}

window.resolvePendingDeletion = async (path, accept) => {
  try {
    await invoke('resolve_pending_deletion', { path, accept });
    await refreshPendingDeletions();
    await refreshStatus();
  } catch (err) {
    alert('Không thể xử lý yêu cầu xoá: ' + err);
  }
};

window.setPeerDeleteMode = async (peerId, mode) => {
  try {
    await invoke('set_peer_delete_mode', { peerId, mode });
    await refreshPeers();
  } catch (err) {
    alert('Không thể cập nhật chế độ xoá của thiết bị: ' + err);
  }
};

// Render Peers
function renderPeers() {
  const allMap = new Map();
  for (const [id, p] of pairedPeersMap) {
    allMap.set(id, {
      device_id: p.device_id,
      device_name: p.name,
      is_paired: true,
      connected: p.is_connected,
      status: p.status || (p.is_connected ? 'connected' : 'offline'),
      addresses: p.last_addr ? [p.last_addr] : [],
      delete_mode: p.delete_mode || null,
    });
  }
  for (const [id, p] of knownPeers) {
    if (!allMap.has(id)) {
      allMap.set(id, {
        device_id: p.device_id,
        device_name: p.device_name,
        is_paired: false,
        connected: !!p.connected,
        status: p.status || (p.connected ? 'connected' : 'discovered'),
        addresses: p.addresses || [],
        delete_mode: null,
      });
    } else {
      const existing = allMap.get(id);
      if (p.connected) existing.connected = true;
      if (p.status) existing.status = p.status;
      if (p.addresses && p.addresses.length) existing.addresses = p.addresses;
    }
  }

  const peers = Array.from(allMap.values());
  const totalCount = peers.length;

  if (elTabDevicesBadge) {
    if (totalCount > 0) {
      elTabDevicesBadge.innerText = totalCount;
      elTabDevicesBadge.classList.remove('hidden');
    } else {
      elTabDevicesBadge.classList.add('hidden');
    }
  }

  if (elPeersSummary) {
    elPeersSummary.innerText = totalCount > 0 ? `Tìm thấy ${totalCount} thiết bị` : 'Tự động tìm kiếm qua mDNS';
  }

  if (totalCount === 0) {
    elPeersList.innerHTML = `
      <div class="empty-state">
        <div class="empty-icon-box">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
            <circle cx="12" cy="12" r="10"/>
            <line x1="2" y1="12" x2="22" y2="12"/>
            <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>
          </svg>
        </div>
        <p class="empty-title">Đang dò quét thiết bị...</p>
        <p class="empty-desc">Đảm bảo các máy cùng kết nối chung một mạng Wi-Fi hoặc cáp LAN.</p>
      </div>
    `;
    return;
  }

  elPeersList.innerHTML = peers.map(p => {
    const isConn = !!p.connected;
    const isPaired = !!p.is_paired;
    let badgeText = 'Ghép đôi';
    let badgeClass = 'discovered';

    if (isConn || p.status === 'connected') {
      badgeText = 'Đã kết nối';
      badgeClass = 'connected';
    } else if (p.status === 'connecting') {
      badgeText = 'Đang nối...';
      badgeClass = 'connecting';
    } else if (p.status === 'backoff') {
      badgeText = 'Chờ thử lại...';
      badgeClass = 'backoff';
    } else if (isPaired) {
      if (p.status === 'offline') {
        badgeText = 'Ngoại tuyến';
        badgeClass = 'offline';
      } else {
        badgeText = 'Đã ghép';
        badgeClass = 'paired';
      }
    }
    const firstAddr = p.addresses && p.addresses.length > 0 ? p.addresses[0] : '';

    return `
      <div class="peer-card">
        <div class="peer-card-left">
          <div class="peer-avatar">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <rect x="2" y="3" width="20" height="14" rx="2" ry="2"/>
              <line x1="8" y1="21" x2="16" y2="21"/>
              <line x1="12" y1="17" x2="12" y2="21"/>
            </svg>
          </div>
          <div class="peer-text-box">
            <div class="peer-title">${escapeHtml(p.device_name)}</div>
            ${firstAddr ? `<div class="peer-ip">${escapeHtml(firstAddr)}</div>` : ''}
          </div>
        </div>
        <div style="display:flex;align-items:center;gap:6px;">
          ${isPaired ? `
            <select class="native-select native-select-xs" title="Chế độ xoá từ thiết bị này" onchange="window.setPeerDeleteMode('${escapeAttr(p.device_id)}', this.value)">
              <option value="default" ${!p.delete_mode ? 'selected' : ''}>Chung</option>
              <option value="never" ${p.delete_mode === 'never' ? 'selected' : ''}>Không xoá</option>
              <option value="ask" ${p.delete_mode === 'ask' ? 'selected' : ''}>Hỏi trước</option>
              <option value="mirror_trash" ${p.delete_mode === 'mirror_trash' ? 'selected' : ''}>Thùng rác</option>
            </select>
          ` : ''}
          <button class="peer-badge ${badgeClass}" ${!isPaired && p.addresses?.length ? `onclick="window.connectPeer('${escapeAttr(p.addresses[0])}')"` : ''}>
            ${badgeText}
          </button>
          ${isPaired ? `
            <button class="btn btn-danger-subtle btn-xs" title="Huỷ ghép nối" onclick="window.unpairDevice('${escapeAttr(p.device_id)}', this)">
              Huỷ
            </button>
          ` : ''}
        </div>
      </div>
    `;
  }).join('');
}

window.unpairDevice = async (peerId, btnElement) => {
  if (btnElement && !btnElement.dataset.confirming) {
    btnElement.dataset.confirming = 'true';
    btnElement.innerText = 'Chắc chắn?';
    btnElement.classList.add('btn-danger');
    setTimeout(() => {
      if (btnElement && btnElement.dataset.confirming) {
        delete btnElement.dataset.confirming;
        btnElement.innerText = 'Huỷ';
        btnElement.classList.remove('btn-danger');
      }
    }, 4000);
    return;
  }

  try {
    if (btnElement) {
      btnElement.disabled = true;
      btnElement.innerText = 'Đang huỷ...';
    }
    const existingPaired = pairedPeersMap.get(peerId);

    await invoke('unpair_device', { peerId });
    pairedPeersMap.delete(peerId);

    // Retain peer in knownPeers so it remains visible as a discovered device to pair
    if (existingPaired) {
      knownPeers.set(peerId, {
        device_id: existingPaired.device_id,
        device_name: existingPaired.name,
        addresses: existingPaired.last_addr ? [existingPaired.last_addr] : [],
        connected: false,
      });
    } else if (knownPeers.has(peerId)) {
      const p = knownPeers.get(peerId);
      p.connected = false;
      knownPeers.set(peerId, p);
    }

    await refreshPeers();
    await refreshStatus();
  } catch (err) {
    console.error('Failed to unpair device:', err);
    if (btnElement) {
      btnElement.disabled = false;
      btnElement.innerText = 'Lỗi huỷ';
    }
  }
};

window.rescanPeers = async () => {
  const btn = document.getElementById('btn-rescan-peers');
  if (btn) {
    btn.disabled = true;
  }
  try {
    const discovered = await invoke('rescan_peers');
    for (const dp of (discovered || [])) {
      if (!knownPeers.has(dp.device_id)) {
        knownPeers.set(dp.device_id, {
          ...dp,
          connected: false,
        });
      } else {
        const existing = knownPeers.get(dp.device_id);
        if (dp.addresses && dp.addresses.length) existing.addresses = dp.addresses;
        if (dp.device_name) existing.device_name = dp.device_name;
      }
    }
    await refreshPeers();
  } catch (err) {
    console.error('Failed to rescan peers:', err);
  } finally {
    if (btn) {
      btn.disabled = false;
    }
  }
};

window.copyLanAddr = async () => {
  const el = document.getElementById('device-lan-addr') || document.getElementById('manual-self-addr');
  const text = el ? el.innerText.trim() : '';
  if (text && text !== '--') {
    try {
      await navigator.clipboard.writeText(text);
      const btn = document.getElementById('btn-copy-lan-addr');
      if (btn) {
        const orig = btn.innerText;
        btn.innerText = 'Đã chép!';
        setTimeout(() => { btn.innerText = orig; }, 2000);
      }
    } catch (_) {}
  }
};

window.connectPeer = async (addr) => {
  if (!addr) return;
  try {
    await invoke('pair_device', { addr });
    await refreshStatus();
  } catch (err) {
    alert('Không thể kết nối tới ' + addr + ': ' + err);
  }
};

// Render Activities
function renderActivities(activities) {
  if (!activities || activities.length === 0) {
    elActivitiesList.innerHTML = `
      <div class="empty-state">
        <p class="empty-title">Chưa có hoạt động</p>
        <p class="empty-desc">Các file được đồng bộ hoặc sự kiện kết nối sẽ xuất hiện tại đây.</p>
      </div>
    `;
    return;
  }

  elActivitiesList.innerHTML = activities.slice(0, 15).map(act => `
    <div class="activity-node">
      <div class="activity-node-main">
        <span class="activity-action-title">${escapeHtml(act.title)}</span>
        <span class="activity-action-detail" title="${escapeHtml(act.details)}">${escapeHtml(act.details)}</span>
      </div>
      <span class="activity-time-stamp">${escapeHtml(act.timestamp)}</span>
    </div>
  `).join('');
}

function formatBytes(bytes) {
  if (!bytes || bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
}

function escapeHtml(text) {
  if (!text) return '';
  return String(text)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function escapeAttr(str) {
  if (!str) return '';
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

// Pause State & UI
async function initPauseStatus() {
  try {
    isPaused = await invoke('is_sync_paused');
    updatePauseUI(isPaused);
  } catch (err) {
    console.error('Init pause error:', err);
  }
}

function updatePauseUI(paused) {
  if (paused) {
    elBtnPauseSync?.classList.add('active');
    elBtnPauseSync?.setAttribute('title', 'Tiếp tục đồng bộ');
    if (elStatusDot) {
      elStatusDot.className = 'dot dot-paused';
    }
    if (elStatusText) {
      elStatusText.innerText = 'Đang tạm dừng';
    }
  } else {
    elBtnPauseSync?.classList.remove('active');
    elBtnPauseSync?.setAttribute('title', 'Tạm dừng đồng bộ');
  }
}

// Bandwidth Limit Setting
async function initBandwidthLimit() {
  try {
    const limit = await invoke('get_bandwidth_limit');
    if (elSettingBandwidthLimit) {
      elSettingBandwidthLimit.value = String(limit);
    }
  } catch (err) {
    console.error('Init bandwidth limit error:', err);
  }
}

// Versions & File Snapshots
async function refreshVersions() {
  if (!elVersionsFileList) return;
  try {
    const files = await invoke('list_versioned_files');
    if (!files || files.length === 0) {
      elVersionsFileList.innerHTML = `
        <div class="empty-state">
          <div class="empty-icon-box">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
              <path d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"/>
            </svg>
          </div>
          <p class="empty-title">Chưa có phiên bản cũ</p>
          <p class="empty-desc">Khi file bị ghi đè bởi máy khác, bản trước đó sẽ tự động được lưu tại đây.</p>
        </div>
      `;
      return;
    }

    elVersionsFileList.innerHTML = '';
    files.forEach(file => {
      const row = document.createElement('div');
      row.className = 'version-file-item';
      row.innerHTML = `
        <div class="version-file-info">
          <span class="version-file-name" title="${escapeAttr(file)}">${escapeHtml(file)}</span>
          <span class="version-file-count">Nhấp để xem lịch sử bản lưu</span>
        </div>
        <button class="btn btn-subtle btn-xs">Xem</button>
      `;
      row.addEventListener('click', () => showVersionDetails(file));
      elVersionsFileList.appendChild(row);
    });

    if (selectedVersionFile) {
      await showVersionDetails(selectedVersionFile);
    }
  } catch (err) {
    console.error('List versioned files error:', err);
  }
}

async function showVersionDetails(filePath) {
  selectedVersionFile = filePath;
  if (!elVersionDetailsCard || !elVersionSnapshotsList) return;

  elVersionDetailsCard.classList.remove('hidden');
  if (elVersionSelectedFile) {
    elVersionSelectedFile.innerText = filePath;
  }

  try {
    const versions = await invoke('list_file_versions', { relPath: filePath });
    elVersionSnapshotsList.innerHTML = '';

    if (!versions || versions.length === 0) {
      elVersionSnapshotsList.innerHTML = '<p class="card-caption">Không tìm thấy bản lưu nào.</p>';
      return;
    }

    versions.forEach(v => {
      const row = document.createElement('div');
      row.className = 'snapshot-row';
      const d = new Date(v.timestamp_micros / 1000);
      const dateStr = d.toLocaleString('vi-VN', {
        year: 'numeric',
        month: '2-digit',
        day: '2-digit',
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
      });
      const sizeStr = formatBytes(v.size);

      row.innerHTML = `
        <div class="snapshot-meta">
          <span class="snapshot-date">${dateStr}</span>
          <span class="snapshot-size">${sizeStr}</span>
        </div>
        <button class="btn btn-outline btn-xs btn-restore-version">Khôi phục</button>
      `;

      row.querySelector('.btn-restore-version')?.addEventListener('click', async (e) => {
        e.stopPropagation();
        const btn = e.currentTarget;
        if (btn && !btn.dataset.confirming) {
          btn.dataset.confirming = 'true';
          btn.innerText = 'Xác nhận?';
          btn.classList.add('btn-primary');
          setTimeout(() => {
            if (btn && btn.dataset.confirming) {
              delete btn.dataset.confirming;
              btn.innerText = 'Khôi phục';
              btn.classList.remove('btn-primary');
            }
          }, 3500);
          return;
        }

        try {
          if (btn) {
            btn.disabled = true;
            btn.innerText = 'Đang khôi phục...';
          }
          await invoke('restore_file_version', {
            relPath: filePath,
            timestampMicros: v.timestamp_micros,
          });
          if (btn) {
            btn.innerText = 'Đã xong!';
          }
          await refreshStatus();
          await refreshVersions();
        } catch (err) {
          console.error('Failed to restore file version:', err);
          if (btn) {
            btn.disabled = false;
            btn.innerText = 'Lỗi!';
          }
        }
      });

      elVersionSnapshotsList.appendChild(row);
    });
  } catch (err) {
    console.error('Show version details error:', err);
  }
}

document.addEventListener('DOMContentLoaded', init);
