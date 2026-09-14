//! Daemon lifecycle and core event loop.

use crate::trust::TrustTier;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[cfg(any(unix, windows))]
use crate::discovery::{DiscoveryConfig, PeerMap, start_announcer, start_listener};
#[cfg(any(unix, windows))]
use crate::identity::Identity;
#[cfg(all(feature = "internet", any(unix, windows)))]
use crate::remote::RemoteConfig;
#[cfg(all(feature = "internet", any(unix, windows)))]
use crate::remote::RemotePeerRegistry;
#[cfg(any(unix, windows))]
use crate::transfer::start_receiver;
#[cfg(any(unix, windows))]
use crate::trust::TrustStore;
#[cfg(any(unix, windows))]
use anyhow::Context;
#[cfg(any(unix, windows))]
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
#[cfg(any(unix, windows))]
use std::path::PathBuf;
#[cfg(any(unix, windows))]
use std::sync::Arc;
#[cfg(any(unix, windows))]
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
#[cfg(windows)]
use tokio::net::windows::named_pipe::ServerOptions;
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
#[cfg(any(unix, windows))]
use tokio::sync::RwLock;
#[cfg(any(unix, windows))]
use tracing::{error, info, warn};

/// Returns the local IPC endpoint used by the daemon and CLI.
#[must_use]
pub fn ipc_endpoint(base_dir: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        base_dir.join("daemon.sock")
    }

    #[cfg(windows)]
    {
        let _ = base_dir;
        PathBuf::from(r"\\.\pipe\waft")
    }

    #[cfg(not(any(unix, windows)))]
    {
        base_dir.join("daemon.sock")
    }
}

/// Shared peer information structure for IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub name: String,
    pub fingerprint: String,
    pub addr: String,
}

/// Commands sent from the CLI to the daemon.
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
}

/// Responses sent from the daemon to the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonResponse {
    Ok(String),
    Error(String),
    PeerList(Vec<PeerInfo>),
    TrustList(Vec<(String, TrustTier)>),
    TrustStatus(TrustTier),
    Progress { bytes_sent: u64, total_bytes: u64 },
}

/// Resolves a friendly peer name to a local identifier.
fn get_local_peer_name() -> String {
    if let Ok(name) = std::env::var("WAFT_PEER_NAME") {
        return name;
    }
    if let Ok(host) = std::env::var("HOSTNAME") {
        return host;
    }
    if let Ok(output) = std::process::Command::new("hostname").output() {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }
    if let Ok(user) = std::env::var("USER") {
        return user;
    }
    "waft-peer".to_string()
}

/// Starts the daemon loop.
///
/// This function:
/// 1. Loads/creates identity and trust store in `base_dir`.
/// 2. Starts TCP file receiver.
/// 3. Starts UDP discovery announcer & listener.
/// 4. Listens on a Unix socket or Windows named pipe for CLI IPC.
#[cfg(any(unix, windows))]
pub async fn start_daemon(base_dir: &Path) -> Result<()> {
    info!(dir = ?base_dir, "Starting waft daemon");

    // 1. Create base directory
    std::fs::create_dir_all(base_dir)
        .with_context(|| format!("Failed to create base directory: {base_dir:?}"))?;

    // 2. Load or generate identity
    let identity_path = base_dir.join("identity");
    let identity = Arc::new(
        Identity::load_or_generate(&identity_path)
            .context("Failed to load or generate identity")?,
    );
    let fingerprint = identity.fingerprint();
    let name = get_local_peer_name();
    info!(name = %name, fingerprint = %fingerprint, "Identity loaded");

    // 3. Load or create trust store
    let trust_path = base_dir.join("trust.toml");
    let trust_store = Arc::new(
        TrustStore::load_or_create(&trust_path).context("Failed to load or create trust store")?,
    );

    // 4. Initialize PeerMap
    let peers = Arc::new(RwLock::new(PeerMap::new()));
    #[cfg(feature = "internet")]
    let remote_peers: RemotePeerRegistry = Arc::new(RwLock::new(std::collections::HashMap::new()));

    // 5. Start TCP receiver on port 7777
    let tcp_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 7777);
    let download_dir = std::env::var("HOME")
        .map_or_else(
            |_| std::env::temp_dir(),
            |home| PathBuf::from(home).join("Downloads"),
        )
        .join("waft");
    std::fs::create_dir_all(&download_dir)
        .with_context(|| format!("Failed to create download directory: {download_dir:?}"))?;

    let receiver_trust = Arc::clone(&trust_store);
    let receiver_downloads = download_dir.clone();
    tokio::spawn(async move {
        if let Err(e) = start_receiver(tcp_addr, receiver_trust, receiver_downloads).await {
            error!(error = %e, "TCP Receiver failed");
        }
    });
    info!(port = 7777, downloads = ?download_dir, "TCP receiver started");

    // 6. Start UDP Multicast Discovery
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let discovery_config = DiscoveryConfig::default();

    // Spawn announcer
    let announcer_name = name.clone();
    let announcer_fingerprint = fingerprint.clone();
    let announcer_config = discovery_config.clone();
    let announcer_shutdown = shutdown_rx.clone();
    tokio::spawn(async move {
        if let Err(e) = start_announcer(
            announcer_name,
            announcer_fingerprint,
            7777,
            announcer_config,
            announcer_shutdown,
        )
        .await
        {
            error!(error = %e, "Discovery announcer failed");
        }
    });

    // Spawn listener
    let listener_fingerprint = fingerprint.clone();
    let listener_peers = Arc::clone(&peers);
    let listener_config = discovery_config.clone();
    let listener_shutdown = shutdown_rx.clone();
    tokio::spawn(async move {
        if let Err(e) = start_listener(
            listener_fingerprint,
            listener_peers,
            listener_config,
            listener_shutdown,
        )
        .await
        {
            error!(error = %e, "Discovery listener failed");
        }
    });
    info!("Discovery service started");

    #[cfg(feature = "internet")]
    if let Some(remote_config) = RemoteConfig::from_env()? {
        let remote_identity = Arc::clone(&identity);
        let remote_trust = Arc::clone(&trust_store);
        let remote_downloads = download_dir.clone();
        let remote_name = name.clone();
        let remote_peers_for_task = Arc::clone(&remote_peers);
        tokio::spawn(async move {
            if let Err(error) = crate::remote::run_remote_receiver(
                &remote_config,
                remote_identity,
                remote_name,
                remote_downloads,
                remote_trust,
                remote_peers_for_task,
            )
            .await
            {
                error!(%error, "Remote signaling receiver stopped");
            }
        });
        info!("Optional internet receiver started");
    }

    #[cfg(unix)]
    {
        let socket_path = ipc_endpoint(base_dir);
        if socket_path.exists() && UnixStream::connect(&socket_path).await.is_ok() {
            anyhow::bail!("Another daemon instance is already running.");
        }
        if socket_path.exists() {
            std::fs::remove_file(&socket_path).context("Failed to clean up stale socket file")?;
        }

        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("Failed to bind Unix socket at {socket_path:?}"))?;
        info!(socket = ?socket_path, "IPC Unix socket listener started");

        loop {
            match listener.accept().await {
                Ok((stream, _)) => spawn_client_handler(
                    stream,
                    &peers,
                    &trust_store,
                    &identity,
                    #[cfg(feature = "internet")]
                    &remote_peers,
                ),
                Err(e) => error!(error = %e, "Failed to accept IPC connection"),
            }
        }
    }

    #[cfg(windows)]
    {
        let pipe_name = ipc_endpoint(base_dir);
        let mut first_instance = true;
        loop {
            let server = ServerOptions::new()
                .first_pipe_instance(first_instance)
                .create(&pipe_name)
                .with_context(|| format!("Failed to create named pipe at {pipe_name:?}"))?;
            first_instance = false;
            server
                .connect()
                .await
                .context("Failed to accept named pipe client")?;
            spawn_client_handler(
                server,
                &peers,
                &trust_store,
                &identity,
                #[cfg(feature = "internet")]
                &remote_peers,
            );
        }
    }
}

#[cfg(any(unix, windows))]
fn spawn_client_handler<S>(
    stream: S,
    peers: &Arc<RwLock<PeerMap>>,
    trust_store: &Arc<TrustStore>,
    identity: &Arc<Identity>,
    #[cfg(feature = "internet")] remote_peers: &RemotePeerRegistry,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let peers_clone = Arc::clone(peers);
    let trust_clone = Arc::clone(trust_store);
    let identity_clone = Arc::clone(identity);
    #[cfg(feature = "internet")]
    let remote_peers_clone = Arc::clone(remote_peers);
    tokio::spawn(async move {
        if let Err(e) = handle_client(
            stream,
            peers_clone,
            trust_clone,
            identity_clone,
            #[cfg(feature = "internet")]
            remote_peers_clone,
        )
        .await
        {
            warn!(error = %e, "Error handling IPC client connection");
        }
    });
}

#[cfg(any(unix, windows))]
/// Handles a single IPC client connection.
async fn handle_client<S>(
    stream: S,
    peers: Arc<RwLock<PeerMap>>,
    trust_store: Arc<TrustStore>,
    identity: Arc<Identity>,
    #[cfg(feature = "internet")] remote_peers: RemotePeerRegistry,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();

    if buf_reader.read_line(&mut line).await? == 0 {
        return Ok(());
    }

    let command: DaemonCommand = serde_json::from_str(&line).context("Failed to parse command")?;

    match command {
        DaemonCommand::ListPeers => {
            let peer_list = peers.read().await.get_all();
            let peer_infos: Vec<PeerInfo> = peer_list
                .into_iter()
                .map(|p| PeerInfo {
                    name: p.name,
                    fingerprint: p.fingerprint,
                    addr: p.addr.to_string(),
                })
                .collect();
            #[cfg(feature = "internet")]
            let mut peer_infos = peer_infos;
            #[cfg(feature = "internet")]
            peer_infos.extend(remote_peers.read().await.values().filter_map(|peer| {
                peer.iroh_endpoint.as_ref().map(|endpoint| PeerInfo {
                    name: peer.name.clone(),
                    fingerprint: peer.fingerprint.clone(),
                    addr: format!("iroh:{endpoint}"),
                })
            }));
            let resp = DaemonResponse::PeerList(peer_infos);
            let serialized = serde_json::to_string(&resp)?;
            writer
                .write_all(format!("{serialized}\n").as_bytes())
                .await?;
        }
        DaemonCommand::SendFile { peer, file_path } => {
            // Locate target peer
            let found_peer = peers
                .read()
                .await
                .get_all()
                .into_iter()
                .find(|p| p.fingerprint == peer || p.name == peer);

            if let Some(peer_node) = found_peer {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                let identity_clone = Arc::clone(&identity);
                let path = PathBuf::from(file_path);
                let addr = peer_node.addr;

                let send_task = tokio::spawn(async move {
                    crate::send::send_file(addr, &identity_clone, &path, Some(tx)).await
                });

                // Read and forward progress reports to the CLI client
                let mut connection_active = true;
                while let Some((bytes_sent, total_bytes)) = rx.recv().await {
                    if connection_active {
                        let resp = DaemonResponse::Progress {
                            bytes_sent,
                            total_bytes,
                        };
                        if let Ok(serialized) = serde_json::to_string(&resp)
                            && writer
                                .write_all(format!("{serialized}\n").as_bytes())
                                .await
                                .is_err()
                        {
                            connection_active = false;
                            send_task.abort();
                        }
                    }
                }

                if connection_active {
                    match send_task.await {
                        Ok(Ok(())) => {
                            let resp = DaemonResponse::Ok("File sent successfully".to_string());
                            let serialized = serde_json::to_string(&resp)?;
                            writer
                                .write_all(format!("{serialized}\n").as_bytes())
                                .await?;
                        }
                        Ok(Err(e)) => {
                            let resp = DaemonResponse::Error(e.to_string());
                            let serialized = serde_json::to_string(&resp)?;
                            writer
                                .write_all(format!("{serialized}\n").as_bytes())
                                .await?;
                        }
                        Err(_) => {
                            let resp = DaemonResponse::Error("Send task panicked".to_string());
                            let serialized = serde_json::to_string(&resp)?;
                            writer
                                .write_all(format!("{serialized}\n").as_bytes())
                                .await?;
                        }
                    }
                }
            } else {
                #[cfg(feature = "internet")]
                if let Some(remote_config) = RemoteConfig::from_env()? {
                    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                    let remote_identity = Arc::clone(&identity);
                    let remote_name = get_local_peer_name();
                    let path = PathBuf::from(file_path);
                    let send_task = tokio::spawn(async move {
                        crate::remote::send_file_over_signal(
                            &remote_config,
                            &remote_identity,
                            remote_name,
                            &peer,
                            &path,
                            Some(tx),
                        )
                        .await
                    });
                    while let Some((bytes_sent, total_bytes)) = rx.recv().await {
                        let response = DaemonResponse::Progress {
                            bytes_sent,
                            total_bytes,
                        };
                        let serialized = serde_json::to_string(&response)?;
                        if writer
                            .write_all(format!("{serialized}\n").as_bytes())
                            .await
                            .is_err()
                        {
                            send_task.abort();
                            return Ok(());
                        }
                    }
                    match send_task.await {
                        Ok(Ok(())) => {
                            let response = DaemonResponse::Ok(
                                "File sent successfully over the internet path".to_string(),
                            );
                            let serialized = serde_json::to_string(&response)?;
                            writer
                                .write_all(format!("{serialized}\n").as_bytes())
                                .await?;
                        }
                        Ok(Err(error)) => {
                            let response = DaemonResponse::Error(error.to_string());
                            let serialized = serde_json::to_string(&response)?;
                            writer
                                .write_all(format!("{serialized}\n").as_bytes())
                                .await?;
                        }
                        Err(error) => {
                            let response =
                                DaemonResponse::Error(format!("Remote send task failed: {error}"));
                            let serialized = serde_json::to_string(&response)?;
                            writer
                                .write_all(format!("{serialized}\n").as_bytes())
                                .await?;
                        }
                    }
                } else {
                    let resp = DaemonResponse::Error(format!("Peer '{peer}' not found"));
                    let serialized = serde_json::to_string(&resp)?;
                    writer
                        .write_all(format!("{serialized}\n").as_bytes())
                        .await?;
                }
                #[cfg(not(feature = "internet"))]
                {
                    let resp = DaemonResponse::Error(format!("Peer '{peer}' not found"));
                    let serialized = serde_json::to_string(&resp)?;
                    writer
                        .write_all(format!("{serialized}\n").as_bytes())
                        .await?;
                }
            }
        }
        DaemonCommand::ListTrust => {
            let config_list = trust_store.get_all();
            let resp = DaemonResponse::TrustList(config_list);
            let serialized = serde_json::to_string(&resp)?;
            writer
                .write_all(format!("{serialized}\n").as_bytes())
                .await?;
        }
        DaemonCommand::SetTrust { fingerprint, tier } => {
            match trust_store.set_tier(&fingerprint, tier) {
                Ok(()) => {
                    let resp = DaemonResponse::Ok(format!(
                        "Trust tier for peer {fingerprint} updated to {tier:?}"
                    ));
                    let serialized = serde_json::to_string(&resp)?;
                    writer
                        .write_all(format!("{serialized}\n").as_bytes())
                        .await?;
                }
                Err(e) => {
                    let resp = DaemonResponse::Error(e.to_string());
                    let serialized = serde_json::to_string(&resp)?;
                    writer
                        .write_all(format!("{serialized}\n").as_bytes())
                        .await?;
                }
            }
        }
        DaemonCommand::GetTrust { fingerprint } => {
            let tier = trust_store.get_tier(&fingerprint);
            let resp = DaemonResponse::TrustStatus(tier);
            let serialized = serde_json::to_string(&resp)?;
            writer
                .write_all(format!("{serialized}\n").as_bytes())
                .await?;
        }
    }

    writer.flush().await?;
    Ok(())
}

#[cfg(not(any(unix, windows)))]
#[allow(clippy::unused_async)]
pub async fn start_daemon(_base_dir: &Path) -> Result<()> {
    anyhow::bail!("Daemon mode is not supported on this platform.");
}
