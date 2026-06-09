// ShadowGate Tauri UI — Frontend Application Logic

// ===== Tauri IPC Wrapper =====
const invoke = window.__TAURI__?.core?.invoke || window.__TAURI_INTERNALS__?.invoke;
const isTauriRuntime = Boolean(invoke);

// Fallback for development without Tauri
function tauriInvoke(cmd, args = {}) {
  if (invoke) {
    return invoke(cmd, args);
  }
  console.warn(`[ShadowGate] Tauri not available — mock: ${cmd}`, args);
  return Promise.resolve(mockResponse(cmd));
}

function mockResponse(cmd) {
  switch (cmd) {
    case 'get_status':
      return { state: 'SCANNING', rssi: -52.5, device_name: 'Pixel 7 Pro', uptime_seconds: 120, daemon_available: false };
    case 'get_devices':
      return [{ name: 'Pixel 7 Pro', hash: 'a1b2c3d4e5f6a7b8', paired_at: '2026-06-08T12:00:00Z', last_auth: '2026-06-08T12:05:00Z' }];
    case 'get_config':
      return { unlock_threshold: -60, lock_threshold: -80, scan_interval_ms: 1000, challenge_timeout_ms: 1500, lock_confirmation_ms: 5000 };
    case 'get_logs':
      return ['[12:00:01] ShadowGate daemon started', '[12:00:02] BLE adapter initialized (Intel AX210)', '[12:00:03] Loaded 1 trusted device(s)', '[12:00:05] BLE scan started (filter: 7f4d0001...)'];
    case 'toggle_daemon':
      return true;
    case 'pair_device':
      return 'Device paired: a1b2c3d4...';
    case 'unpair_device':
      return 'Device unpaired';
    case 'update_config':
      return 'Config updated';
    default:
      return null;
  }
}

// ===== State =====
let daemonActive = false;
let refreshInterval = null;

// ===== DOM Ready =====
document.addEventListener('DOMContentLoaded', () => {
  initTabs();
  loadStatus();
  loadDevices();
  loadConfig();
  startAutoRefresh();
});

// ===== Tab Navigation =====
function initTabs() {
  document.querySelectorAll('.tab').forEach(tab => {
    tab.addEventListener('click', () => {
      document.querySelectorAll('.tab').forEach(t => t.classList.remove('active'));
      document.querySelectorAll('.tab-panel').forEach(p => p.classList.remove('active'));

      tab.classList.add('active');
      const target = tab.dataset.tab;
      document.getElementById(`panel-${target}`).classList.add('active');

      // Refresh content when switching tabs
      if (target === 'devices') loadDevices();
      if (target === 'logs') refreshLogs();
    });
  });
}

// ===== Status Updates =====
async function loadStatus() {
  try {
    const status = await tauriInvoke('get_status');
    updateStatusUI(status);
  } catch (e) {
    console.error('Failed to load status:', e);
  }
}

function updateStatusUI(status) {
  const dot = document.getElementById('statusDot');
  const text = document.getElementById('statusText');
  const rssiVal = document.getElementById('rssiValue');
  const rssiFill = document.getElementById('rssiFill');
  const deviceName = document.getElementById('deviceName');
  const uptimeEl = document.getElementById('uptime');

  // Status dot
  dot.className = 'status-dot';
  if (status.state === 'UNLOCKED' || status.state === 'MONITORING') {
    dot.classList.add('active');
  } else if (status.state === 'AUTHENTICATING') {
    dot.classList.add('warning');
  } else if (status.state === 'IDLE') {
    dot.classList.add('danger');
  }

  text.textContent = status.state || 'IDLE';

  // RSSI
  if (status.rssi != null) {
    rssiVal.textContent = `${status.rssi.toFixed(1)} dBm`;
    const pct = Math.min(100, Math.max(0, ((status.rssi + 100) / 70) * 100));
    rssiFill.style.width = `${pct}%`;
  } else {
    rssiVal.textContent = '-- dBm';
    rssiFill.style.width = '0%';
  }

  // Device / backend status
  if (status.device_name) {
    deviceName.textContent = status.device_name;
  } else if (status.daemon_available === false && isTauriRuntime) {
    deviceName.textContent = 'Local configuration mode - daemon IPC pending';
  } else if (!isTauriRuntime) {
    deviceName.textContent = 'Browser preview mode';
  } else {
    deviceName.textContent = 'No device connected';
  }

  // Uptime
  const uptime = status.uptime_seconds || 0;
  const mins = Math.floor(uptime / 60);
  const secs = uptime % 60;
  uptimeEl.textContent = `up ${mins}m ${secs}s`;
}

// ===== Toggle Daemon =====
async function toggleDaemon() {
  try {
    daemonActive = await tauriInvoke('toggle_daemon');
    updateToggleButton();
    loadStatus();
    refreshLogs();
  } catch (e) {
    console.error('Failed to toggle daemon:', e);
    showToast('Failed to toggle daemon');
  }
}

function updateToggleButton() {
  const btn = document.getElementById('toggleBtn');
  const icon = btn.querySelector('.btn-icon');
  const text = btn.querySelector('span:last-child');

  if (daemonActive) {
    icon.innerHTML = '&#9632;'; // Stop square
    text.textContent = 'Stop Daemon';
    btn.style.background = 'linear-gradient(135deg, #ff6b6b, #e74c3c)';
  } else {
    icon.innerHTML = '&#9654;'; // Play triangle
    text.textContent = 'Start Daemon';
    btn.style.background = 'linear-gradient(135deg, var(--accent), #a29bfe)';
  }
}

// ===== Devices =====
async function loadDevices() {
  try {
    const devices = await tauriInvoke('get_devices');
    renderDevices(devices);
  } catch (e) {
    console.error('Failed to load devices:', e);
  }
}

function renderDevices(devices) {
  const container = document.getElementById('deviceList');

  if (!devices || devices.length === 0) {
    container.innerHTML = `
      <div class="empty-state">
        <p>No trusted devices</p>
        <p class="sub">Scan a QR code from your Android device to pair</p>
      </div>`;
    return;
  }

  container.innerHTML = devices.map(d => `
    <div class="device-item">
      <div class="device-item-info">
        <span class="device-item-name">${escapeHtml(d.name)}</span>
        <span class="device-item-hash">${escapeHtml(d.hash).substring(0, 16)}...</span>
        <span class="device-item-meta">Paired ${formatTimestamp(d.paired_at)}</span>
      </div>
      <button class="device-item-remove" onclick="unpairDevice('${escapeHtml(d.hash)}')">Remove</button>
    </div>
  `).join('');
}

async function pairDevice() {
  const qrContent = prompt('Enter QR code content or paste public key:');
  if (!qrContent) return;

  try {
    const result = await tauriInvoke('pair_device', { qrContent });
    console.log('Pair result:', result);
    showToast(result);
    loadDevices();
    refreshLogs();
  } catch (e) {
    console.error('Failed to pair device:', e);
    alert('Failed to pair device: ' + e);
  }
}

async function unpairDevice(hash) {
  if (!confirm(`Remove device ${hash.substring(0, 8)}...?`)) return;

  try {
    await tauriInvoke('unpair_device', { hash });
    showToast('Device removed');
    loadDevices();
    refreshLogs();
  } catch (e) {
    console.error('Failed to unpair device:', e);
    showToast('Failed to remove device');
  }
}

// ===== Config =====
async function loadConfig() {
  try {
    const config = await tauriInvoke('get_config');
    document.getElementById('cfgUnlock').value = config.unlock_threshold;
    document.getElementById('cfgLock').value = config.lock_threshold;
    document.getElementById('cfgScan').value = config.scan_interval_ms;
    document.getElementById('cfgTimeout').value = config.challenge_timeout_ms;
    document.getElementById('cfgConfirm').value = config.lock_confirmation_ms;

    updateSliderLabel('cfgUnlock', 'cfgUnlockVal', 'dBm');
    updateSliderLabel('cfgLock', 'cfgLockVal', 'dBm');
    updateSliderLabel('cfgScan', 'cfgScanVal', 'ms');
    updateSliderLabel('cfgTimeout', 'cfgTimeoutVal', 'ms');
    updateSliderLabel('cfgConfirm', 'cfgConfirmVal', 'ms');
  } catch (e) {
    console.error('Failed to load config:', e);
  }
}

function updateSliderLabel(sliderId, labelId, unit) {
  const val = document.getElementById(sliderId).value;
  document.getElementById(labelId).textContent = `${val}${unit}`;
}

async function saveConfig() {
  const config = {
    unlock_threshold: parseInt(document.getElementById('cfgUnlock').value),
    lock_threshold: parseInt(document.getElementById('cfgLock').value),
    scan_interval_ms: parseInt(document.getElementById('cfgScan').value),
    challenge_timeout_ms: parseInt(document.getElementById('cfgTimeout').value),
    lock_confirmation_ms: parseInt(document.getElementById('cfgConfirm').value),
  };

  try {
    await tauriInvoke('update_config', { configJson: JSON.stringify(config) });
    showToast('Configuration saved');
    refreshLogs();
  } catch (e) {
    console.error('Failed to save config:', e);
    alert('Failed to save configuration: ' + e);
  }
}

// ===== Logs =====
async function refreshLogs() {
  try {
    const logs = await tauriInvoke('get_logs');
    const viewer = document.getElementById('logViewer');

    if (logs && logs.length > 0) {
      viewer.innerHTML = logs.map(log => {
        let cls = 'info';
        if (log.includes('ERROR') || log.includes('error') || log.includes('FAILED')) cls = 'error';
        if (log.includes('WARN') || log.includes('warn')) cls = 'warn';
        return `<div class="log-entry ${cls}">${escapeHtml(log)}</div>`;
      }).join('');
    }
  } catch (e) {
    console.error('Failed to load logs:', e);
  }
}

// ===== Auto-Refresh =====
function startAutoRefresh() {
  refreshInterval = setInterval(() => {
    loadStatus();
  }, 2000);
}

// ===== Toast =====
function showToast(message) {
  const toast = document.createElement('div');
  toast.style.cssText = `
    position: fixed;
    bottom: 24px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--accent);
    color: white;
    padding: 10px 24px;
    border-radius: 20px;
    font-size: 13px;
    font-weight: 500;
    z-index: 1000;
    animation: fadeInUp 0.3s ease;
  `;
  toast.textContent = message;
  document.body.appendChild(toast);

  setTimeout(() => {
    toast.style.opacity = '0';
    toast.style.transition = 'opacity 0.3s';
    setTimeout(() => toast.remove(), 300);
  }, 2000);
}

// Add fadeInUp animation
const style = document.createElement('style');
style.textContent = `
  @keyframes fadeInUp {
    from { opacity: 0; transform: translateX(-50%) translateY(10px); }
    to { opacity: 1; transform: translateX(-50%) translateY(0); }
  }
`;
document.head.appendChild(style);

// ===== Utilities =====
function escapeHtml(str) {
  const div = document.createElement('div');
  div.textContent = str ?? '';
  return div.innerHTML;
}

function formatTimestamp(value) {
  const numeric = Number(value);
  if (!Number.isFinite(numeric) || numeric <= 0) {
    return '--';
  }
  return new Date(numeric * 1000).toLocaleString();
}
