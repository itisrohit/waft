//! UDP multicast peer discovery.
//!
//! This module broadcasts the local peer's presence periodically and listens
//! passively on the LAN to detect and track active peers.

use crate::error::WaftError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use tokio::time;

/// Configuration options for the discovery service.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// IP address to bind announcer and listener sockets to.
    pub bind_ip: std::net::IpAddr,
    /// UDP multicast group address and port.
    pub multicast_addr: SocketAddr,
    /// How frequently to broadcast the peer presence.
    pub announce_interval: Duration,
    /// Time period after which inactive peers are evicted.
    pub peer_timeout: Duration,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            bind_ip: std::net::IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            multicast_addr: SocketAddr::from(([239, 255, 77, 77], 7777)),
            announce_interval: Duration::from_secs(2),
            peer_timeout: Duration::from_secs(10),
        }
    }
}

/// Payload sent over UDP multicast to announce a peer's presence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerAnnouncement {
    /// Friendly name of the peer.
    pub name: String,
    /// Cryptographic fingerprint (hex public key) of the peer.
    pub fingerprint: String,
    /// TCP port on which the peer's transfer server is listening.
    pub port: u16,
}

/// Information representing a discovered LAN peer.
#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    /// Friendly name of the peer.
    pub name: String,
    /// Cryptographic fingerprint (hex public key) of the peer.
    pub fingerprint: String,
    /// Direct network address (IP and transfer port) of the peer.
    pub addr: SocketAddr,
    /// Timestamp when the last announcement was received.
    pub last_seen: Instant,
}

/// Parses a received announcement payload into a discovered peer record.
///
/// Invalid payloads and self-announcements are ignored by returning `None`.
#[must_use]
pub fn parse_announcement(
    payload: &[u8],
    src_addr: SocketAddr,
    my_fingerprint: &str,
) -> Option<DiscoveredPeer> {
    let announcement = std::str::from_utf8(payload)
        .ok()
        .and_then(|s| toml::from_str::<PeerAnnouncement>(s).ok())?;

    if announcement.port == 0 {
        return None;
    }
    if announcement.fingerprint.len() != 64
        || !announcement
            .fingerprint
            .chars()
            .all(|c| c.is_ascii_hexdigit())
    {
        return None;
    }
    if announcement.name.is_empty() || announcement.name.len() > 63 {
        return None;
    }
    if announcement.fingerprint == my_fingerprint {
        return None;
    }

    let mut tcp_addr = src_addr;
    tcp_addr.set_port(announcement.port);

    Some(DiscoveredPeer {
        name: announcement.name,
        fingerprint: announcement.fingerprint,
        addr: tcp_addr,
        last_seen: Instant::now(),
    })
}

/// Collection mapping peer fingerprints to their discovered details.
#[derive(Debug, Default)]
pub struct PeerMap {
    peers: HashMap<String, DiscoveredPeer>,
}

impl PeerMap {
    /// Creates a new empty `PeerMap`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
        }
    }

    /// Inserts or updates a discovered peer in the map.
    pub fn insert(&mut self, fingerprint: String, peer: DiscoveredPeer) {
        self.peers.insert(fingerprint, peer);
    }

    /// Returns a list of all active discovered peers.
    #[must_use]
    pub fn get_all(&self) -> Vec<DiscoveredPeer> {
        self.peers.values().cloned().collect()
    }

    /// Checks if a peer announcement warrants an update to the database.
    ///
    /// Returns `true` if the peer is new, or if its metadata (name or address) has changed,
    /// or if the last seen timestamp is older than the `throttle` duration.
    #[must_use]
    pub fn should_update(
        &self,
        fingerprint: &str,
        name: &str,
        addr: SocketAddr,
        throttle: Duration,
    ) -> bool {
        self.peers.get(fingerprint).is_none_or(|existing| {
            existing.name != name
                || existing.addr != addr
                || existing.last_seen.elapsed() >= throttle
        })
    }

    /// Retains only peers whose last seen timestamp is within the specified timeout.
    pub fn clean_expired(&mut self, timeout: Duration) {
        let now = Instant::now();
        self.peers.retain(|_, peer| {
            now.checked_duration_since(peer.last_seen)
                .is_none_or(|elapsed| elapsed < timeout)
        });
    }
}

/// Runs a periodic loop to broadcast the local peer presence over UDP multicast.
///
/// # Errors
/// Returns a `WaftError` if socket creation, binding, or serialization fails.
pub async fn start_announcer(
    name: String,
    fingerprint: String,
    port: u16,
    config: DiscoveryConfig,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<(), WaftError> {
    let raw_socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    raw_socket.set_reuse_address(true)?;
    #[cfg(unix)]
    raw_socket.set_reuse_port(true)?;

    // Bind to bind_ip and let OS assign a random outgoing port
    let bind_addr = SocketAddr::new(config.bind_ip, 0);
    raw_socket.bind(&bind_addr.into())?;
    raw_socket.set_nonblocking(true)?;

    if let std::net::IpAddr::V4(ipv4) = config.bind_ip {
        let _ = raw_socket.set_multicast_if_v4(&ipv4);
    }

    let std_socket: std::net::UdpSocket = raw_socket.into();
    let socket = UdpSocket::from_std(std_socket)?;
    socket.set_multicast_loop_v4(true)?;

    let announcement = PeerAnnouncement {
        name,
        fingerprint,
        port,
    };
    let payload_str = toml::to_string(&announcement)?;
    let payload = payload_str.into_bytes();

    let mut interval = time::interval(config.announce_interval);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let _ = socket.send_to(&payload, config.multicast_addr).await;
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Listens passively for peer presence announcements on the multicast group.
///
/// # Errors
/// Returns a `WaftError` if socket binding, joining the multicast group, or parsing fails.
pub async fn start_listener(
    my_fingerprint: String,
    peers: Arc<RwLock<PeerMap>>,
    config: DiscoveryConfig,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<(), WaftError> {
    let raw_socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    raw_socket.set_reuse_address(true)?;
    #[cfg(unix)]
    raw_socket.set_reuse_port(true)?;

    // Binding specifically to the multicast port to receive group traffic
    let port = config.multicast_addr.port();
    let bind_addr = SocketAddr::from(([0, 0, 0, 0], port));
    raw_socket.bind(&bind_addr.into())?;
    raw_socket.set_nonblocking(true)?;

    let std_socket: std::net::UdpSocket = raw_socket.into();
    let socket = UdpSocket::from_std(std_socket)?;

    let ip = match config.multicast_addr.ip() {
        std::net::IpAddr::V4(ipv4) => ipv4,
        std::net::IpAddr::V6(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Only IPv4 multicast is supported",
            )
            .into());
        }
    };

    let join_interface = match config.bind_ip {
        std::net::IpAddr::V4(ipv4) => ipv4,
        std::net::IpAddr::V6(_) => Ipv4Addr::UNSPECIFIED,
    };
    socket.join_multicast_v4(ip, join_interface)?;

    let mut buf = [0u8; 1024];
    let mut clean_interval = time::interval(Duration::from_secs(5));
    let throttle_dur = Duration::from_secs(1);

    loop {
        tokio::select! {
            res = socket.recv_from(&mut buf) => {
                match res {
                    Ok((len, src_addr)) => {
                        if let Some(discovered) =
                            parse_announcement(&buf[..len], src_addr, &my_fingerprint)
                        {
                            let fingerprint = discovered.fingerprint.clone();
                            let name = discovered.name.clone();
                            let tcp_addr = discovered.addr;

                            // Lock optimization: check with a read lock first to throttle writes
                            let should_update = {
                                let map = peers.read().await;
                                map.should_update(&fingerprint, &name, tcp_addr, throttle_dur)
                            };

                            if should_update {
                                let mut map = peers.write().await;
                                map.insert(fingerprint, discovered);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "multicast socket recv_from error");
                    }
                }
            }
            _ = clean_interval.tick() => {
                let mut map = peers.write().await;
                map.clean_expired(config.peer_timeout);
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }

    Ok(())
}
