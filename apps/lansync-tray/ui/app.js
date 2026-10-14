// LanSync UI Controller
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// Local state
let currentStatus = null;
let currentPairingPeerId = null;
let knownPeers = new Map();

// Elements
const elStatusDot = document.getElementById('status-dot');
const elStatusText = document.getElementById('status-text');
const elDeviceId = document.getElementById('device-id-badge');
const elDeviceName = document.getElementById('device-name');
const elFolderPath = document.getElementById('folder-path');
const elBtnOpenFolder = document.getElementById('btn-open-folder');

const elProgressBox = document.getElementById('sync-progress-box');
const elProgressFile = document.getElementById('progress-filename');
const elProgressStat = document.getElementById('progress-percent');

const elPairingCard = document.getElementById('pairing-card');
const elPairingPeerName = document.getElementById('pairing-peer-name');
const elPairingCodeDisplay = document.getElementById('pairing-code-display');
const elBtnApprovePairing = document.getElementById('btn-approve-pairing');
const elBtnRejectPairing = document.getElementById('btn-reject-pairing');

const elPeersList = document.getElementById('peers-list');
const elPeersCount = document.getElementById('peers-count');
const elActivitiesList = document.getElementById('activities-list');

const elBtnToggleSettings = document.getElementById('btn-toggle-settings');
const elBtnCloseSettings = document.getElementById('btn-close-settings');
const elSettingsView = document.getElementById('settings-view');
const elSettingSyncDir = document.getElementById('setting-sync-dir');
const elBtnBrowseFolder = document.getElementById('btn-browse-folder');
const elManualIpInput = document.getElementById('manual-ip-input');
const elBtnManualPair = document.getElementById('btn-manual-pair');

// Initialize application
async function init() {
  setupEventListeners();
  await refreshStatus();
  setupSyncEventSubscriber();
}

function setupEventListeners() {
  // Open sync folder button
  elBtnOpenFolder.addEventListener('click', async () => {
    try {
      await invoke('open_sync_folder');
    } catch (err) {
      console.error('Error opening folder:', err);
    }
  });

  // Copy Device ID
  elDeviceId.addEventListener('click', () => {
    if (currentStatus?.device_id) {
      navigator.clipboard.writeText(currentStatus.device_id);
      const originalText = elDeviceId.innerText;
      elDeviceId.innerText = 'Copied!';
      setTimeout(() => {
        elDeviceId.innerText = originalText;
      }, 1500);
    }
  });

  // Pairing - Approve
  elBtnApprovePairing.addEventListener('click', async () => {
    if (!currentPairingPeerId) return;
    try {
      await invoke('approve_pairing', { peerId: currentPairingPeerId });
      hidePairingCard();
      await refreshStatus();
    } catch (err) {
      alert('Error: ' + err);
    }
  });

  // Pairing - Reject
  elBtnRejectPairing.addEventListener('click', async () => {
    if (!currentPairingPeerId) return;
    try {
      await invoke('reject_pairing', { peerId: currentPairingPeerId });
      hidePairingCard();
    } catch (err) {
      console.error('Error:', err);
    }
  });

  // Settings overlay
  elBtnToggleSettings.addEventListener('click', () => {
    elSettingsView.classList.remove('hidden');
  });

  elBtnCloseSettings.addEventListener('click', () => {
    elSettingsView.classList.add('hidden');
  });

  // Manual connection via IP:Port
  elBtnManualPair.addEventListener('click', async () => {
    const addr = elManualIpInput.value.trim();
    if (!addr) return;
    try {
      elBtnManualPair.disabled = true;
      elBtnManualPair.innerText = 'Connecting...';
      await invoke('pair_device', { addr });
      elManualIpInput.value = '';
      elSettingsView.classList.add('hidden');
    } catch (err) {
      alert('Failed to connect: ' + err);
    } finally {
      elBtnManualPair.disabled = false;
      elBtnManualPair.innerText = 'Connect';
    }
  });

  // Select sync folder
  elBtnBrowseFolder.addEventListener('click', async () => {
    try {
      const selected = await invoke('pick_sync_folder');
      if (selected) {
        elSettingSyncDir.value = selected;
        elFolderPath.innerText = selected;
      }
    } catch (err) {
      console.error('Error selecting folder:', err);
    }
  });
}

// Refresh status from Rust backend
async function refreshStatus() {
  try {
    const status = await invoke('get_status');
    currentStatus = status;

    elDeviceName.innerText = status.device_name;
    elDeviceId.innerText = `ID: ${status.device_id.substring(0, 8)}...`;
    elFolderPath.innerText = status.sync_dir;
    elSettingSyncDir.value = status.sync_dir;

    renderActivities(status.activities || []);
  } catch (err) {
    console.error('Failed to get status:', err);
  }
}

// Subscribe to real-time events from Rust
async function setupSyncEventSubscriber() {
  await listen('sync_event', (event) => {
    const data = event.payload;
    handleSyncEvent(data);
  });
}

function handleSyncEvent(event) {
  if (!event) return;

  // 1. Peer discovered via mDNS
  if (event.PeerDiscovered) {
    const peer = event.PeerDiscovered;
    knownPeers.set(peer.device_id, {
      ...peer,
      connected: false,
    });
    renderPeers();
  }

  // 2. Pairing requested
  else if (event.PairingRequested) {
    const { peer_id, device_name, code } = event.PairingRequested;
    showPairingCard(peer_id, device_name, code);
  }

  // 3. Pairing accepted
  else if (event.PairingAccepted) {
    hidePairingCard();
    refreshStatus();
  }

  // 4. Peer connected
  else if (event.PeerConnected) {
    const { peer_id, device_name } = event.PeerConnected;
    const existing = knownPeers.get(peer_id) || { device_id, device_name, addresses: [] };
    existing.connected = true;
    knownPeers.set(peer_id, existing);
    renderPeers();
    setSyncStatus('idle');
  }

  // 5. Peer disconnected
  else if (event.PeerDisconnected) {
    const { peer_id } = event.PeerDisconnected;
    const existing = knownPeers.get(peer_id);
    if (existing) {
      existing.connected = false;
      renderPeers();
    }
    setSyncStatus('idle');
  }

  // 6. File downloading
  else if (event.FileDownloading) {
    const { rel_path, size } = event.FileDownloading;
    setSyncStatus('syncing');
    showProgress(rel_path, `${formatBytes(size)}`);
  }

  // 7. File downloaded
  else if (event.FileDownloaded) {
    hideProgress();
    setSyncStatus('idle');
    refreshStatus();
  }

  // 8. Full sync completed
  else if (event.SyncCompleted) {
    hideProgress();
    setSyncStatus('idle');
    refreshStatus();
  }
}

function setSyncStatus(state) {
  if (state === 'syncing') {
    elStatusDot.className = 'dot dot-syncing';
    elStatusText.innerText = 'Syncing...';
  } else {
    elStatusDot.className = 'dot dot-online';
    elStatusText.innerText = 'Active';
  }
}

function showProgress(fileName, statText) {
  elProgressBox.classList.remove('hidden');
  elProgressFile.innerText = fileName;
  elProgressStat.innerText = statText;
}

function hideProgress() {
  elProgressBox.classList.add('hidden');
}

function showPairingCard(peerId, name, code) {
  currentPairingPeerId = peerId;
  elPairingPeerName.innerText = name;
  elPairingCodeDisplay.innerText = code;
  elPairingCard.classList.remove('hidden');
}

function hidePairingCard() {
  currentPairingPeerId = null;
  elPairingCard.classList.add('hidden');
}

// Render peer list
function renderPeers() {
  const peers = Array.from(knownPeers.values());
  elPeersCount.innerText = peers.length;

  if (peers.length === 0) {
    elPeersList.innerHTML = '<div class="empty-state">Searching for devices on LAN...</div>';
    return;
  }

  elPeersList.innerHTML = peers.map(p => {
    const statusText = p.connected ? 'Connected' : 'Discovered';
    const statusClass = p.connected ? 'connected' : 'unpaired';
    const firstAddr = p.addresses && p.addresses.length > 0 ? p.addresses[0] : '';

    return `
      <div class="peer-item">
        <div class="peer-info">
          <svg class="peer-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <rect x="2" y="3" width="20" height="14" rx="2" ry="2"></rect>
            <line x1="8" y1="21" x2="16" y2="21"></line>
            <line x1="12" y1="17" x2="12" y2="21"></line>
          </svg>
          <div>
            <div class="peer-name">${escapeHtml(p.device_name)}</div>
            <div class="peer-addr">${escapeHtml(firstAddr)}</div>
          </div>
        </div>
        <span class="peer-status ${statusClass}">${statusText}</span>
      </div>
    `;
  }).join('');
}

// Render recent activities
function renderActivities(activities) {
  if (!activities || activities.length === 0) {
    elActivitiesList.innerHTML = '<div class="empty-state">No recent activity</div>';
    return;
  }

  elActivitiesList.innerHTML = activities.slice(0, 10).map(act => `
    <div class="activity-item">
      <div class="activity-main">
        <span class="activity-title">${escapeHtml(act.title)}</span>
        <span class="activity-details" title="${escapeHtml(act.details)}">${escapeHtml(act.details)}</span>
      </div>
      <span class="activity-time">${escapeHtml(act.timestamp)}</span>
    </div>
  `).join('');
}

function formatBytes(bytes) {
  if (bytes === 0) return '0 B';
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

document.addEventListener('DOMContentLoaded', init);
