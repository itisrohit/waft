import { invoke } from '@tauri-apps/api/core';
import './styles.css';

const statusText = document.querySelector('#daemon-status');
const statusDot = document.querySelector('.status-dot');
const peerState = document.querySelector('#peer-state');
const stateTitle = document.querySelector('#state-title');
const stateDescription = document.querySelector('#state-description');
const settingsButton = document.querySelector('#settings-button');

function setPeerState(state, title, description) {
  peerState.className = `state-card ${state}`;
  stateTitle.textContent = title;
  stateDescription.textContent = description;
}

async function refreshDaemonStatus() {
  setPeerState('loading', 'Connecting to waft', 'Checking the daemon for nearby peers…');
  try {
    const status = await invoke('daemon_status');
    statusText.textContent = status.detail;
    statusDot.classList.add('connected');
    setPeerState('empty', 'No peers nearby', 'When another waft device is available, it will appear here.');
  } catch (error) {
    statusText.textContent = String(error);
    statusDot.classList.remove('connected');
    setPeerState('unavailable', 'Waft is unavailable', 'The daemon could not be reached. Check that it is running, then try again.');
  }
}

settingsButton.addEventListener('click', () => {
  statusText.textContent = 'Settings will be available in a later step';
});

refreshDaemonStatus();
window.setInterval(refreshDaemonStatus, 3000);
