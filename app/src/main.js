import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import './styles.css';

const statusText = document.querySelector('#daemon-status');
const statusDot = document.querySelector('.status-dot');
const peerState = document.querySelector('#peer-state');
const peerList = document.querySelector('#peer-list');
const stateTitle = document.querySelector('#state-title');
const stateDescription = document.querySelector('#state-description');
const settingsButton = document.querySelector('#settings-button');
const deviceName = document.querySelector('#device-name');
const deviceAvatar = document.querySelector('#device-avatar');
const incomingSection = document.querySelector('#incoming-section');
const incomingList = document.querySelector('#incoming-list');
const sendButton = document.querySelector('#send-button');
const receivingMode = document.querySelector('#receiving-mode');
const settingsPanel = document.querySelector('#settings-panel');
const closeSettings = document.querySelector('#close-settings');
const settingsDeviceName = document.querySelector('#settings-device-name');
const settingsReceivingMode = document.querySelector('#settings-receiving-mode');
let selectedPeer = null;
let hasInitialResult = false;
let hasPeerSnapshot = false;
let refreshInFlight = false;

invoke('device_name')
  .then((name) => {
    deviceName.textContent = name;
    deviceAvatar.textContent = initialsFor(name);
    settingsDeviceName.textContent = name;
  })
  .catch(() => { /* Keep the neutral fallback label. */ });

function setPeerState(state, title, description) {
  peerState.className = `state-card ${state}`;
  stateTitle.textContent = title;
  stateDescription.textContent = description;
  peerState.hidden = false;
}

function initialsFor(name) {
  const initials = name
    .trim()
    .split(/\s+/)
    .map((part) => part[0])
    .filter(Boolean)
    .slice(0, 2)
    .join('')
    .toUpperCase();
  return initials || '?';
}

function renderPeers(peers) {
  peerList.replaceChildren();
  if (peers.length === 0) {
    hasPeerSnapshot = true;
    setPeerState('empty', 'No peers nearby', 'When another waft device is available, it will appear here.');
    return;
  }

  hasPeerSnapshot = true;
  peerState.hidden = true;
  for (const peer of peers) {
    const card = document.createElement('article');
    card.className = 'peer-card';
    if (selectedPeer?.name === peer.name) card.classList.add('selected');
    card.tabIndex = 0;
    card.setAttribute('aria-label', `${peer.name}, ${peer.route}`);

    const avatar = document.createElement('div');
    avatar.className = 'peer-avatar';
    avatar.setAttribute('aria-hidden', 'true');
    avatar.textContent = peer.initials;

    const details = document.createElement('div');
    details.className = 'peer-details';
    const name = document.createElement('h3');
    name.textContent = peer.name;
    const availability = document.createElement('p');
    availability.textContent = peer.available ? 'Available' : 'Unavailable';
    details.append(name, availability);

    const route = document.createElement('span');
    route.className = `route-badge ${peer.route.toLowerCase()}`;
    route.textContent = peer.route;

    card.append(avatar, details, route);
    const selectPeer = () => {
      selectedPeer = selectedPeer?.name === peer.name ? null : peer;
      renderPeers(peers);
      updateSendButton();
    };
    card.addEventListener('click', selectPeer);
    card.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); selectPeer(); }
    });
    peerList.append(card);
  }
}

function updateSendButton() {
  sendButton.disabled = !selectedPeer;
  sendButton.setAttribute('aria-disabled', String(!selectedPeer));
  sendButton.innerHTML = selectedPeer
    ? '<span aria-hidden="true">＋</span> Send a file'
    : '<span aria-hidden="true">＋</span> Select a peer to send';
}

function renderIncoming(transfers) {
  incomingList.replaceChildren();
  incomingSection.hidden = transfers.length === 0;
  for (const transfer of transfers) {
    const card = document.createElement('article');
    card.className = 'incoming-card';
    const title = document.createElement('h3');
    title.textContent = `${transfer.sender_name} wants to send a file`;
    const file = document.createElement('p');
    file.textContent = `${transfer.file_name} · ${formatBytes(transfer.file_size)}`;
    card.append(title, file);

    if (transfer.state === 'awaiting_approval') {
      const actions = document.createElement('div');
      actions.className = 'incoming-actions';
      for (const [label, accept] of [['Accept', true], ['Reject', false]]) {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = accept ? 'accept-button' : 'reject-button';
        button.textContent = label;
        button.addEventListener('click', async () => {
          button.disabled = true;
          try { await invoke('decide_incoming', { transferId: transfer.id, accept }); }
          catch (error) { console.warn('Unable to decide incoming transfer', error); }
          await refreshIncoming();
        });
        actions.append(button);
      }
      card.append(actions);
    } else {
      const progress = document.createElement('p');
      progress.className = 'incoming-progress';
      progress.textContent = `Receiving · ${formatBytes(transfer.bytes_received)} of ${formatBytes(transfer.file_size)}`;
      card.append(progress);
    }
    incomingList.append(card);
  }
}

function formatBytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

async function refreshIncoming() {
  try { renderIncoming(await invoke('incoming_transfers')); }
  catch (error) { console.warn('Unable to refresh incoming transfers', error); }
}

async function refreshPeers() {
  if (refreshInFlight) return;
  refreshInFlight = true;
  if (!hasInitialResult) {
    setPeerState('loading', 'Finding nearby peers', 'Checking LAN and Internet availability…');
  }

  try {
    const peers = await invoke('nearby_peers');
    hasInitialResult = true;
    statusText.textContent = 'Connected to waft daemon';
    statusDot.classList.add('connected');
    renderPeers(peers);
    await refreshIncoming();
  } catch (error) {
    hasInitialResult = true;
    statusText.textContent = 'Waft daemon unavailable';
    statusDot.classList.remove('connected');
    if (!hasPeerSnapshot) {
      peerList.replaceChildren();
      setPeerState('unavailable', 'Waft is unavailable', 'The daemon could not be reached. Check that it is running, then try again.');
    }
    console.warn('Unable to refresh peers', error);
  } finally {
    refreshInFlight = false;
  }
}

settingsButton.addEventListener('click', () => {
  settingsPanel.hidden = false;
  settingsPanel.querySelector('button').focus();
});

closeSettings.addEventListener('click', () => {
  settingsPanel.hidden = true;
  settingsButton.focus();
});

receivingMode.addEventListener('change', async () => {
  const modes = { off: 'ReceivingOff', contacts: 'ContactsOnly', everyone: 'Everyone' };
  try {
    await invoke('set_receiving_mode', { mode: modes[receivingMode.value] });
    settingsReceivingMode.textContent = receivingMode.options[receivingMode.selectedIndex].text;
    statusText.textContent = `Receiving mode: ${receivingMode.options[receivingMode.selectedIndex].text}`;
  } catch (error) {
    statusText.textContent = `Unable to change receiving mode: ${String(error)}`;
  }
});

sendButton.addEventListener('click', async () => {
  if (!selectedPeer) return;
  let path;
  try {
    path = await open({ multiple: false, directory: false, title: 'Choose a file to send' });
  } catch (error) {
    statusText.textContent = `File picker failed: ${String(error)}`;
    sendButton.textContent = 'File picker failed';
    window.setTimeout(updateSendButton, 2500);
    return;
  }
  if (!path || Array.isArray(path)) return;
  sendButton.disabled = true;
  sendButton.textContent = 'Sending…';
  statusText.textContent = `Sending to ${selectedPeer.name}…`;
  try {
    const result = await invoke('send_file', { peer: selectedPeer.name, filePath: path });
    statusText.textContent = result;
    sendButton.textContent = '✓ Sent successfully';
  } catch (error) {
    statusText.textContent = `Send failed: ${String(error)}`;
    sendButton.textContent = 'Send failed';
  } finally {
    window.setTimeout(updateSendButton, 2500);
  }
});

refreshPeers();
window.setInterval(refreshPeers, 3000);
