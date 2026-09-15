//! Typed client for the existing waft daemon IPC protocol.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrustTier {
    Blocked,
    Ask,
    Trusted,
    Own,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub name: String,
    pub fingerprint: String,
    pub addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonCommand {
    ListPeers,
    SendFile {
        peer: String,
        file_path: String,
    },
    ListTrust,
    SetTrust {
        fingerprint: String,
        tier: TrustTier,
    },
    GetTrust {
        fingerprint: String,
    },
    ListIncoming,
    AcceptIncoming {
        transfer_id: String,
    },
    RejectIncoming {
        transfer_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonResponse {
    Ok(String),
    Error(String),
    PeerList(Vec<PeerInfo>),
    TrustList(Vec<(String, TrustTier)>),
    TrustStatus(TrustTier),
    Progress { bytes_sent: u64, total_bytes: u64 },
    IncomingList(Vec<IncomingTransfer>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingTransfer {
    pub id: String,
    pub sender_name: String,
    pub fingerprint: String,
    pub file_name: String,
    pub file_size: u64,
    pub bytes_received: u64,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonStatus {
    pub state: &'static str,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NearbyPeer {
    pub name: String,
    pub initials: String,
    pub route: &'static str,
    pub available: bool,
}

#[derive(Debug, Clone)]
pub struct DaemonClient {
    endpoint: PathBuf,
}

impl DaemonClient {
    #[must_use]
    pub fn from_environment() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        Self {
            endpoint: home.join(".waft").join("daemon.sock"),
        }
    }

    pub fn ensure_running(&self) -> Result<DaemonStatus, String> {
        match self.request(DaemonCommand::ListPeers) {
            Ok(DaemonResponse::PeerList(_)) => Ok(DaemonStatus {
                state: "connected",
                detail: "Connected to waft daemon".to_string(),
            }),
            Ok(DaemonResponse::Error(error)) => Err(error),
            Ok(_) => Err("Daemon returned an unexpected response".to_string()),
            Err(first_error) => {
                start_daemon().map_err(|start_error| {
                    format!("Daemon unavailable: {first_error}. Could not start it: {start_error}")
                })?;
                for _ in 0..20 {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    if let Ok(DaemonResponse::PeerList(_)) = self.request(DaemonCommand::ListPeers)
                    {
                        return Ok(DaemonStatus {
                            state: "connected",
                            detail: "Started and connected to waft daemon".to_string(),
                        });
                    }
                }
                Err(format!("Daemon did not become ready: {first_error}"))
            }
        }
    }

    pub fn request(&self, command: DaemonCommand) -> Result<DaemonResponse, String> {
        #[cfg(unix)]
        {
            use std::os::unix::net::UnixStream;
            let mut stream = UnixStream::connect(&self.endpoint)
                .map_err(|error| format!("connect {}: {error}", self.endpoint.display()))?;
            let encoded = serde_json::to_string(&command).map_err(|error| error.to_string())?;
            writeln!(stream, "{encoded}").map_err(|error| error.to_string())?;
            stream.flush().map_err(|error| error.to_string())?;
            read_response(BufReader::new(stream))
        }

        #[cfg(windows)]
        {
            let mut stream = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(r"\\.\pipe\waft")
                .map_err(|error| format!("connect named pipe: {error}"))?;
            let encoded = serde_json::to_string(&command).map_err(|error| error.to_string())?;
            writeln!(stream, "{encoded}").map_err(|error| error.to_string())?;
            stream.flush().map_err(|error| error.to_string())?;
            read_response(BufReader::new(stream))
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = command;
            Err("waft daemon IPC is not supported on this platform".to_string())
        }
    }

    pub fn list_peers(&self) -> Result<Vec<PeerInfo>, String> {
        match self.request(DaemonCommand::ListPeers)? {
            DaemonResponse::PeerList(peers) => Ok(peers),
            DaemonResponse::Error(error) => Err(error),
            _ => Err("Daemon returned an unexpected peer response".to_string()),
        }
    }

    pub fn list_incoming(&self) -> Result<Vec<IncomingTransfer>, String> {
        match self.request(DaemonCommand::ListIncoming)? {
            DaemonResponse::IncomingList(transfers) => Ok(transfers),
            DaemonResponse::Error(error) => Err(error),
            _ => Err("Daemon returned an unexpected incoming-transfer response".to_string()),
        }
    }

    pub fn decide_incoming(&self, transfer_id: String, accept: bool) -> Result<String, String> {
        let command = if accept {
            DaemonCommand::AcceptIncoming { transfer_id }
        } else {
            DaemonCommand::RejectIncoming { transfer_id }
        };
        match self.request(command)? {
            DaemonResponse::Ok(message) => Ok(message),
            DaemonResponse::Error(error) => Err(error),
            _ => Err("Daemon returned an unexpected transfer decision response".to_string()),
        }
    }

    pub fn send_file(&self, peer: String, file_path: String) -> Result<String, String> {
        #[cfg(unix)]
        {
            use std::os::unix::net::UnixStream;
            let mut stream = UnixStream::connect(&self.endpoint)
                .map_err(|error| format!("connect daemon: {error}"))?;
            let command = serde_json::to_string(&DaemonCommand::SendFile { peer, file_path })
                .map_err(|error| error.to_string())?;
            writeln!(stream, "{command}").map_err(|error| error.to_string())?;
            stream.flush().map_err(|error| error.to_string())?;
            let mut reader = BufReader::new(stream);
            loop {
                let mut line = String::new();
                reader
                    .read_line(&mut line)
                    .map_err(|error| error.to_string())?;
                if line.is_empty() {
                    return Err("daemon closed the send connection".to_string());
                }
                match serde_json::from_str::<DaemonResponse>(&line)
                    .map_err(|error| format!("invalid daemon response: {error}"))?
                {
                    DaemonResponse::Ok(message) => return Ok(message),
                    DaemonResponse::Error(error) => return Err(error),
                    DaemonResponse::Progress { .. } => {}
                    _ => return Err("daemon returned an unexpected send response".to_string()),
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (peer, file_path);
            Err("sending is not supported on this platform yet".to_string())
        }
    }
}

pub fn nearby_peers() -> Result<Vec<NearbyPeer>, String> {
    let client = DaemonClient::from_environment();
    client.ensure_running()?;
    client
        .list_peers()
        .map(|peers| peers.into_iter().map(to_nearby_peer).collect())
}

fn to_nearby_peer(peer: PeerInfo) -> NearbyPeer {
    NearbyPeer {
        initials: initials_for(&peer.name),
        route: if peer.addr.starts_with("iroh:") {
            "Internet"
        } else {
            "LAN"
        },
        name: peer.name,
        available: true,
    }
}

fn initials_for(name: &str) -> String {
    let initials: String = name
        .split_whitespace()
        .filter_map(|part| part.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    if initials.is_empty() {
        "?".to_string()
    } else {
        initials
    }
}

fn read_response<R: BufRead>(mut reader: R) -> Result<DaemonResponse, String> {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    if line.is_empty() {
        return Err("daemon closed the IPC connection".to_string());
    }
    serde_json::from_str(&line).map_err(|error| format!("invalid daemon response: {error}"))
}

fn start_daemon() -> Result<(), String> {
    let executable = std::env::var_os("WAFT_DAEMON_PATH")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|path| find_sibling_daemon(&path))
        })
        .unwrap_or_else(|| PathBuf::from("waft"));
    Command::new(&executable)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("start {}: {error}", executable.display()))
}

fn find_sibling_daemon(app_executable: &Path) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(parent) = app_executable.parent() {
        candidates.push(parent.join("waft"));
        // Development layout: app/src-tauri/target/debug/waft-desktop
        // alongside the repository's target/debug/waft daemon.
        candidates.push(parent.join("../../../../target/debug/waft"));
    }
    candidates.into_iter().find(|path| path.is_file())
}
