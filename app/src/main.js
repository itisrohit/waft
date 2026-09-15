import { invoke } from '@tauri-apps/api/core';
import './styles.css';

const statusText = document.querySelector('#daemon-status');
const statusDot = document.querySelector('.status-dot');
const peerState = document.querySelector('#peer-state');
const peerList = document.querySelector('#peer-list');
const stateTitle = document.querySelector('#state-title');
const stateDescription = document.querySelector('#state-description');
const settingsButton = document.querySelector('#settings-button');
const deviceName = document.querySelector('#device-name');
let hasInitialResult = false;
let hasPeerSnapshot = false;
let refreshInFlight = false;

invoke('device_name')
  .then((name) => { deviceName.textContent = name; })
  .catch(() => { /* Keep the neutral fallback label. */ });

function setPeerState(state, title, description) {
  peerState.className = `state-card ${state}`;
  stateTitle.textContent = title;
  stateDescription.textContent = description;
  peerState.hidden = false;
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
    peerList.append(card);
  }
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
  statusText.textContent = 'Settings will be available in a later step';
});

refreshPeers();
window.setInterval(refreshPeers, 3000);
