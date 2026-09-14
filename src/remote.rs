//! Optional internet signaling and WebRTC transport.
//!
//! The core daemon remains LAN-only unless the `internet` feature is enabled and
//! a signaling URL and room are configured. The signaling service never sees
//! file contents; it only forwards SDP messages used to establish a direct
//! WebRTC data channel.

#![cfg(feature = "internet")]
#![allow(
    clippy::collapsible_if,
    clippy::format_collect,
    clippy::manual_let_else,
    clippy::needless_continue,
    clippy::needless_pass_by_value
)]

use anyhow::{Context, Result, anyhow};
use blake3::Hasher;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use uuid::Uuid;
use webrtc::api::APIBuilder;
use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_init::RTCDataChannelInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

const MAX_SIGNAL_MESSAGE_BYTES: usize = 256 * 1024;
const MAX_IROH_ENDPOINT_BYTES: usize = 16 * 1024;
/// Default rendezvous deployment for normal waft builds.
pub const DEFAULT_SIGNALING_URL: &str = "wss://waft-signaling.nooks-license.workers.dev";

/// Runtime settings for the optional internet path.
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    /// WebSocket URL of a self-hosted signaling service.
    pub signaling_url: String,
    /// Shared room secret. Peers in different rooms cannot see each other.
    pub room: String,
    /// STUN/TURN URLs, comma-separated in `WAFT_ICE_SERVERS`.
    pub ice_servers: Vec<String>,
}

impl RemoteConfig {
    /// Loads settings from environment variables.
    pub fn from_env() -> Result<Option<Self>> {
        Self::from_overrides(None, None)
    }

    /// Loads settings using command-line overrides before environment fallbacks.
    pub fn from_overrides(
        signaling_url_override: Option<&str>,
        room_override: Option<&str>,
    ) -> Result<Option<Self>> {
        let room = room_override
            .map(str::to_owned)
            .or_else(|| std::env::var("WAFT_SIGNALING_ROOM").ok());
        let Some(room) = room else {
            return Ok(None);
        };
        let signaling_url = signaling_url_override
            .map(str::to_owned)
            .or_else(|| {
                std::env::var_os("WAFT_SIGNALING_URL").and_then(|value| value.into_string().ok())
            })
            .unwrap_or_else(|| DEFAULT_SIGNALING_URL.to_string());
        if room.is_empty() || room.len() > 128 {
            return Err(anyhow!("WAFT_SIGNALING_ROOM must be 1-128 characters"));
        }
        let ice_servers = std::env::var("WAFT_ICE_SERVERS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect();
        Ok(Some(Self {
            signaling_url,
            room,
            ice_servers,
        }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePeer {
    pub id: Uuid,
    pub name: String,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iroh_endpoint: Option<String>,
}

/// Remote peers learned from the rendezvous service.
pub type RemotePeerRegistry = Arc<RwLock<HashMap<Uuid, RemotePeer>>>;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignalMessage {
    Register { room: String, peer: RemotePeer },
    Welcome { peers: Vec<RemotePeer> },
    PeerJoined { peer: RemotePeer },
    PeerLeft { id: Uuid },
    Offer { from: Uuid, to: Uuid, sdp: String },
    Answer { from: Uuid, to: Uuid, sdp: String },
    Error { message: String },
}

/// A bidirectional rendezvous connection. It carries SDP negotiation and
/// optional endpoint metadata; file bytes never pass through the server.
pub struct SignalingConnection {
    pub outgoing: tokio::sync::mpsc::UnboundedSender<SignalMessage>,
    pub incoming: tokio::sync::mpsc::UnboundedReceiver<SignalMessage>,
}

type PeerSender = tokio::sync::mpsc::UnboundedSender<Message>;
type Rooms = Arc<RwLock<HashMap<String, HashMap<Uuid, (RemotePeer, PeerSender)>>>>;

/// Runs the standalone signaling server used by the optional internet path.
pub async fn run_signaling_server(bind: SocketAddr) -> Result<()> {
    let listener = TcpListener::bind(bind)
        .await
        .context("bind signaling server")?;
    let rooms: Rooms = Arc::new(RwLock::new(HashMap::new()));
    tracing::info!(%bind, "waft signaling server listening");
    loop {
        let (stream, peer_addr) = listener.accept().await.context("accept signaling client")?;
        let rooms = Arc::clone(&rooms);
        tokio::spawn(async move {
            if let Err(error) = handle_signaling_connection(stream, rooms).await {
                tracing::debug!(%peer_addr, %error, "signaling connection ended");
            }
        });
    }
}

async fn handle_signaling_connection(stream: TcpStream, rooms: Rooms) -> Result<()> {
    let websocket = accept_async(stream).await.context("websocket handshake")?;
    let (mut sink, mut source) = websocket.split();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            sink.send(message).await.map_err(anyhow::Error::from)?;
        }
        Ok::<(), anyhow::Error>(())
    });

    let Some(Ok(Message::Text(first))) = source.next().await else {
        writer.abort();
        return Ok(());
    };
    if first.len() > MAX_SIGNAL_MESSAGE_BYTES {
        writer.abort();
        return Err(anyhow!("registration message too large"));
    }
    let SignalMessage::Register { room, peer } =
        serde_json::from_str(&first).context("invalid signaling registration")?
    else {
        writer.abort();
        return Err(anyhow!("first signaling message must register"));
    };
    if room.is_empty()
        || peer.name.is_empty()
        || peer.fingerprint.len() != 64
        || peer
            .iroh_endpoint
            .as_ref()
            .is_some_and(|endpoint| endpoint.is_empty() || endpoint.len() > MAX_IROH_ENDPOINT_BYTES)
    {
        writer.abort();
        return Err(anyhow!("invalid peer registration"));
    }

    let existing = {
        let mut rooms_guard = rooms.write().await;
        let room_peers = rooms_guard.entry(room.clone()).or_default();
        let existing = room_peers
            .values()
            .map(|(peer, _)| peer.clone())
            .collect::<Vec<_>>();
        room_peers.insert(peer.id, (peer.clone(), out_tx.clone()));
        existing
    };
    send_signal(&out_tx, SignalMessage::Welcome { peers: existing })?;
    broadcast(
        &rooms,
        &room,
        peer.id,
        SignalMessage::PeerJoined { peer: peer.clone() },
    )
    .await;

    while let Some(message) = source.next().await {
        let message = message.context("read signaling message")?;
        let Message::Text(text) = message else {
            continue;
        };
        if text.len() > MAX_SIGNAL_MESSAGE_BYTES {
            break;
        }
        let parsed: SignalMessage =
            serde_json::from_str(&text).context("invalid signaling message")?;
        let target = match parsed {
            SignalMessage::Offer { from, to, sdp } => {
                Some((to, SignalMessage::Offer { from, to, sdp }))
            }
            SignalMessage::Answer { from, to, sdp } => {
                Some((to, SignalMessage::Answer { from, to, sdp }))
            }
            _ => None,
        };
        if let Some((target_id, forwarded)) = target {
            let rooms_guard = rooms.read().await;
            if let Some((_, target_tx)) = rooms_guard
                .get(&room)
                .and_then(|peers| peers.get(&target_id))
            {
                send_signal(target_tx, forwarded)?;
            } else {
                send_signal(
                    &out_tx,
                    SignalMessage::Error {
                        message: "target peer is offline".to_string(),
                    },
                )?;
            }
        }
    }

    {
        let mut rooms_guard = rooms.write().await;
        if let Some(room_peers) = rooms_guard.get_mut(&room) {
            room_peers.remove(&peer.id);
            if room_peers.is_empty() {
                rooms_guard.remove(&room);
            }
        }
    }
    broadcast(
        &rooms,
        &room,
        peer.id,
        SignalMessage::PeerLeft { id: peer.id },
    )
    .await;
    writer.abort();
    Ok(())
}

async fn broadcast(rooms: &Rooms, room: &str, except: Uuid, message: SignalMessage) {
    let rooms_guard = rooms.read().await;
    if let Some(peers) = rooms_guard.get(room) {
        if let Ok(text) = serde_json::to_string(&message) {
            for (id, (_, sender)) in peers {
                if *id != except {
                    let _ = sender.send(Message::Text(text.clone().into()));
                }
            }
        }
    }
}

fn send_signal(sender: &PeerSender, message: SignalMessage) -> Result<()> {
    let text = serde_json::to_string(&message).context("serialize signaling message")?;
    sender
        .send(Message::Text(text.into()))
        .map_err(|_| anyhow!("signaling peer disconnected"))
}

/// Connects to the signaling server. This is kept separate from the daemon so
/// callers can use it for diagnostics and for future background peer tracking.
pub async fn connect_signaling(
    config: &RemoteConfig,
    peer: RemotePeer,
) -> Result<SignalingConnection> {
    let room_token = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        config.room.as_bytes(),
    );
    let separator = if config.signaling_url.contains('?') {
        '&'
    } else {
        '?'
    };
    let signaling_url = format!("{}{separator}room={room_token}", config.signaling_url);
    let (socket, _) = tokio_tungstenite::connect_async(signaling_url)
        .await
        .context("connect signaling server")?;
    let (mut sink, mut source) = socket.split();
    let (outgoing_tx, mut outgoing_rx) = tokio::sync::mpsc::unbounded_channel::<SignalMessage>();
    let (incoming_tx, incoming_rx) = tokio::sync::mpsc::unbounded_channel::<SignalMessage>();
    let registration = SignalMessage::Register {
        room: config.room.clone(),
        peer,
    };
    sink.send(Message::Text(serde_json::to_string(&registration)?.into()))
        .await?;
    tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(message) = outgoing_rx.recv() => {
                    let text = match serde_json::to_string(&message) { Ok(text) => text, Err(_) => break };
                    if sink.send(Message::Text(text.into())).await.is_err() { break; }
                }
                incoming = source.next() => {
                    let Some(Ok(Message::Text(text))) = incoming else { break };
                    if let Ok(message) = serde_json::from_str(&text) {
                        if incoming_tx.send(message).is_err() { break; }
                    }
                }
            }
        }
    });
    Ok(SignalingConnection {
        outgoing: outgoing_tx,
        incoming: incoming_rx,
    })
}

/// Creates a WebRTC peer connection using the configured STUN/TURN servers.
pub async fn create_peer_connection(ice_servers: &[String]) -> Result<Arc<RTCPeerConnection>> {
    let api = APIBuilder::new().build();
    let configuration = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: ice_servers.to_vec(),
            ..Default::default()
        }],
        ..Default::default()
    };
    Ok(Arc::new(api.new_peer_connection(configuration).await?))
}

/// Creates an ordered reliable data channel and returns the fully gathered SDP offer.
pub async fn create_offer(
    peer_connection: &Arc<RTCPeerConnection>,
) -> Result<(Arc<RTCDataChannel>, RTCSessionDescription)> {
    let data_channel = peer_connection
        .create_data_channel(
            "waft",
            Some(RTCDataChannelInit {
                ordered: Some(true),
                max_packet_life_time: None,
                max_retransmits: None,
                protocol: Some("waft-file-v1".to_string()),
                negotiated: None,
            }),
        )
        .await?;
    let offer = peer_connection.create_offer(None).await?;
    let mut gathering = peer_connection.gathering_complete_promise().await;
    peer_connection.set_local_description(offer).await?;
    let _ = gathering.recv().await;
    let local = peer_connection
        .local_description()
        .await
        .ok_or_else(|| anyhow!("WebRTC did not produce a local description"))?;
    Ok((data_channel, local))
}

/// Applies a signaling answer to an offerer connection.
pub async fn apply_answer(peer_connection: &Arc<RTCPeerConnection>, sdp: String) -> Result<()> {
    peer_connection
        .set_remote_description(RTCSessionDescription::answer(sdp)?)
        .await?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct FileHeader {
    name: String,
    size: u64,
    hash: String,
    fingerprint: String,
    public_key: String,
    signature: String,
}

fn signed_header_bytes(name: &str, size: u64, hash: &str, fingerprint: &str) -> Vec<u8> {
    format!("waft-webrtc-v1\n{name}\n{size}\n{hash}\n{fingerprint}").into_bytes()
}

/// Sends one file through an already negotiated WebRTC data channel.
pub async fn send_file_over_channel(
    channel: &Arc<RTCDataChannel>,
    identity: &crate::identity::Identity,
    file_path: &std::path::Path,
    progress: Option<tokio::sync::mpsc::UnboundedSender<(u64, u64)>>,
) -> Result<()> {
    use tokio::io::AsyncReadExt;
    let metadata = tokio::fs::metadata(file_path).await?;
    let name = file_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("file has no valid name"))?
        .to_string();
    let mut hash = Hasher::new();
    let mut input = tokio::fs::File::open(file_path).await?;
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes).await?;
    hash.update(&bytes);
    let digest = hash.finalize().to_hex().to_string();
    let fingerprint = identity.fingerprint();
    let header = FileHeader {
        name: name.clone(),
        size: metadata.len(),
        hash: digest.clone(),
        fingerprint: fingerprint.clone(),
        public_key: base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            identity.public_key().to_bytes(),
        ),
        signature: base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            identity
                .sign(&signed_header_bytes(
                    &name,
                    metadata.len(),
                    &digest,
                    &fingerprint,
                ))
                .to_bytes(),
        ),
    };
    channel.send_text(serde_json::to_string(&header)?).await?;
    let chunk_size = 64 * 1024;
    let mut sent = 0_u64;
    for chunk in bytes.chunks(chunk_size) {
        channel.send(&bytes::Bytes::copy_from_slice(chunk)).await?;
        sent += chunk.len() as u64;
        if let Some(progress) = &progress {
            let _ = progress.send((sent, metadata.len()));
        }
    }
    channel.send_text("{\"type\":\"done\"}").await?;
    Ok(())
}

/// Negotiates a direct WebRTC channel with a named or fingerprinted peer and
/// sends one file. This is the daemon-facing internet fallback entry point.
pub async fn send_file_over_signal(
    config: &RemoteConfig,
    identity: &crate::identity::Identity,
    name: String,
    target: &str,
    file_path: &std::path::Path,
    progress: Option<tokio::sync::mpsc::UnboundedSender<(u64, u64)>>,
) -> Result<()> {
    let local_id = Uuid::new_v4();
    #[cfg(feature = "iroh-internet")]
    let iroh_endpoint_handle = crate::iroh_transport::bind_endpoint().await?;
    #[cfg(feature = "iroh-internet")]
    let iroh_endpoint = Some(crate::iroh_transport::endpoint_address_json(
        &iroh_endpoint_handle,
    )?);
    #[cfg(not(feature = "iroh-internet"))]
    let iroh_endpoint = None;
    let mut signaling = connect_signaling(
        config,
        RemotePeer {
            id: local_id,
            name,
            fingerprint: identity.fingerprint(),
            iroh_endpoint,
        },
    )
    .await?;
    let target_peer = loop {
        match signaling.incoming.recv().await {
            Some(SignalMessage::Welcome { peers }) => {
                break peers
                    .into_iter()
                    .find(|peer| peer.name == target || peer.fingerprint == target)
                    .ok_or_else(|| {
                        anyhow!("remote peer '{target}' not found in signaling room")
                    })?;
            }
            Some(SignalMessage::PeerJoined { peer })
                if peer.name == target || peer.fingerprint == target =>
            {
                break peer;
            }
            Some(_) => continue,
            None => return Err(anyhow!("signaling connection closed")),
        }
    };
    let peer_connection = create_peer_connection(&config.ice_servers).await?;
    let (channel, offer) = create_offer(&peer_connection).await?;
    let (open_tx, open_rx) = tokio::sync::oneshot::channel();
    channel.on_open(Box::new(move || {
        Box::pin(async move {
            let _ = open_tx.send(());
        })
    }));
    signaling
        .outgoing
        .send(SignalMessage::Offer {
            from: local_id,
            to: target_peer.id,
            sdp: offer.sdp,
        })
        .map_err(|_| anyhow!("signaling connection closed"))?;
    loop {
        match signaling.incoming.recv().await {
            Some(SignalMessage::Answer { from, to, sdp })
                if from == target_peer.id && to == local_id =>
            {
                apply_answer(&peer_connection, sdp).await?;
                break;
            }
            Some(_) => continue,
            None => return Err(anyhow!("signaling connection closed before answer")),
        }
    }
    tokio::time::timeout(std::time::Duration::from_secs(60), open_rx).await??;
    send_file_over_channel(&channel, identity, file_path, progress).await?;
    peer_connection.close().await?;
    Ok(())
}

/// Accepts an offer, receives one signed file, and writes it atomically.
pub async fn accept_file_offer(
    peer_connection: &Arc<RTCPeerConnection>,
    offer_sdp: String,
    downloads: std::path::PathBuf,
    trust: Arc<crate::trust::TrustStore>,
    signaling: &tokio::sync::mpsc::UnboundedSender<SignalMessage>,
    local_id: Uuid,
    remote_id: Uuid,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let (channel_tx, channel_rx) = tokio::sync::oneshot::channel::<Arc<RTCDataChannel>>();
    let channel_tx = Arc::new(tokio::sync::Mutex::new(Some(channel_tx)));
    peer_connection.on_data_channel(Box::new(move |channel: Arc<RTCDataChannel>| {
        let channel_tx = Arc::clone(&channel_tx);
        Box::pin(async move {
            if channel.label() == "waft" {
                let channel_for_open = Arc::clone(&channel);
                channel.on_open(Box::new(move || {
                    let channel_tx = Arc::clone(&channel_tx);
                    let channel = Arc::clone(&channel_for_open);
                    Box::pin(async move {
                        if let Some(sender) = channel_tx.lock().await.take() {
                            let _ = sender.send(channel);
                        }
                    })
                }));
            }
        })
    }));
    peer_connection
        .set_remote_description(RTCSessionDescription::offer(offer_sdp)?)
        .await?;
    let answer = peer_connection.create_answer(None).await?;
    let mut gathering = peer_connection.gathering_complete_promise().await;
    peer_connection.set_local_description(answer).await?;
    let _ = gathering.recv().await;
    let local_description = peer_connection
        .local_description()
        .await
        .ok_or_else(|| anyhow!("WebRTC did not produce an answer"))?;
    signaling
        .send(SignalMessage::Answer {
            from: local_id,
            to: remote_id,
            sdp: local_description.sdp,
        })
        .map_err(|_| anyhow!("signaling connection closed"))?;
    let channel = tokio::time::timeout(std::time::Duration::from_secs(60), channel_rx).await??;
    let (message_tx, mut message_rx) = tokio::sync::mpsc::unbounded_channel();
    channel.on_message(Box::new(move |message| {
        let tx = message_tx.clone();
        Box::pin(async move {
            let _ = tx.send(message);
        })
    }));
    let header = loop {
        let message = tokio::time::timeout(std::time::Duration::from_secs(60), message_rx.recv())
            .await?
            .ok_or_else(|| anyhow!("remote data channel closed"))?;
        if message.is_string {
            break serde_json::from_slice::<FileHeader>(&message.data)
                .context("invalid remote file header")?;
        }
    };
    if header.name.is_empty()
        || header.name.contains(['/', '\\'])
        || header.name == "."
        || header.name == ".."
    {
        anyhow::bail!("remote file name is invalid");
    }
    if trust.get_tier(&header.fingerprint) == crate::trust::TrustTier::Blocked {
        anyhow::bail!("remote peer is blocked");
    }
    let public_key = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        header.public_key,
    )?;
    let key = VerifyingKey::from_bytes(
        public_key
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("invalid public key"))?,
    )?;
    if key
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        != header.fingerprint
    {
        anyhow::bail!("remote fingerprint mismatch");
    }
    let signature = Signature::from_bytes(
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, header.signature)?
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("invalid signature"))?,
    );
    key.verify(
        &signed_header_bytes(&header.name, header.size, &header.hash, &header.fingerprint),
        &signature,
    )?;
    tokio::fs::create_dir_all(&downloads).await?;
    let temp = downloads.join(format!("{}.part", header.hash));
    let final_path = downloads.join(&header.name);
    let mut output = tokio::fs::File::create(&temp).await?;
    let mut received = 0_u64;
    let mut hasher = Hasher::new();
    while let Some(message) = message_rx.recv().await {
        if message.is_string {
            break;
        }
        output.write_all(&message.data).await?;
        hasher.update(&message.data);
        received += message.data.len() as u64;
    }
    output.flush().await?;
    if received != header.size || hasher.finalize().to_hex().as_str() != header.hash {
        anyhow::bail!("remote file hash mismatch");
    }
    tokio::fs::rename(temp, final_path).await?;
    peer_connection.close().await?;
    Ok(())
}

/// Keeps a daemon registered and accepts incoming offers from its signaling room.
pub async fn run_remote_receiver(
    config: &RemoteConfig,
    identity: Arc<crate::identity::Identity>,
    name: String,
    downloads: std::path::PathBuf,
    trust: Arc<crate::trust::TrustStore>,
    peers: RemotePeerRegistry,
) -> Result<()> {
    let local_id = Uuid::new_v4();
    #[cfg(feature = "iroh-internet")]
    let iroh_endpoint_handle = crate::iroh_transport::bind_endpoint().await?;
    #[cfg(feature = "iroh-internet")]
    let iroh_endpoint = Some(crate::iroh_transport::endpoint_address_json(
        &iroh_endpoint_handle,
    )?);
    #[cfg(not(feature = "iroh-internet"))]
    let iroh_endpoint = None;
    let mut signaling = connect_signaling(
        config,
        RemotePeer {
            id: local_id,
            name,
            fingerprint: identity.fingerprint(),
            iroh_endpoint,
        },
    )
    .await?;
    while let Some(message) = signaling.incoming.recv().await {
        match message {
            SignalMessage::Welcome { peers: discovered } => {
                let mut registry = peers.write().await;
                registry.clear();
                registry.extend(discovered.into_iter().map(|peer| (peer.id, peer)));
            }
            SignalMessage::PeerJoined { peer } => {
                peers.write().await.insert(peer.id, peer);
            }
            SignalMessage::PeerLeft { id } => {
                peers.write().await.remove(&id);
            }
            SignalMessage::Offer { from, to, sdp } if to == local_id => {
                let peer_connection = create_peer_connection(&config.ice_servers).await?;
                let downloads = downloads.clone();
                let trust = Arc::clone(&trust);
                let outgoing = signaling.outgoing.clone();
                tokio::spawn(async move {
                    if let Err(error) = accept_file_offer(
                        &peer_connection,
                        sdp,
                        downloads,
                        trust,
                        &outgoing,
                        local_id,
                        from,
                    )
                    .await
                    {
                        tracing::warn!(%error, "remote file receive failed");
                    }
                });
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{RemoteConfig, RemotePeer, SignalMessage};
    use uuid::Uuid;

    #[test]
    fn endpoint_is_optional_for_existing_registrations() -> Result<(), serde_json::Error> {
        let message = r#"{"type":"register","room":"room","peer":{"id":"00000000-0000-0000-0000-000000000000","name":"mac","fingerprint":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#;
        let parsed: SignalMessage = serde_json::from_str(message)?;
        assert!(matches!(parsed, SignalMessage::Register { .. }));
        let SignalMessage::Register { peer, .. } = parsed else {
            return Ok(());
        };
        assert!(peer.iroh_endpoint.is_none());
        Ok(())
    }

    #[test]
    fn endpoint_is_preserved_in_registration() -> Result<(), serde_json::Error> {
        let peer = RemotePeer {
            id: Uuid::nil(),
            name: "mac".to_string(),
            fingerprint: "a".repeat(64),
            iroh_endpoint: Some("{\"id\":\"endpoint\"}".to_string()),
        };
        let json = serde_json::to_string(&SignalMessage::Register {
            room: "room".to_string(),
            peer,
        })?;
        assert!(json.contains("iroh_endpoint"));
        Ok(())
    }

    #[test]
    fn command_line_overrides_supply_remote_configuration() -> anyhow::Result<()> {
        let config = RemoteConfig::from_overrides(
            Some("wss://override.example.test"),
            Some("private-room"),
        )?
        .ok_or_else(|| anyhow::anyhow!("expected remote configuration"))?;
        assert_eq!(config.signaling_url, "wss://override.example.test");
        assert_eq!(config.room, "private-room");
        Ok(())
    }
}
