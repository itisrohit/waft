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
    multicast_addr: SocketAddr,
    bind_ip: std::net::IpAddr,
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
    let bind_addr = SocketAddr::new(bind_ip, 0);
    raw_socket.bind(&bind_addr.into())?;
    raw_socket.set_nonblocking(true)?;

    if let std::net::IpAddr::V4(ipv4) = bind_ip {
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

    let mut interval = time::interval(Duration::from_secs(2));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let _ = socket.send_to(&payload, multicast_addr).await;
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
    multicast_addr: SocketAddr,
    bind_ip: std::net::IpAddr,
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
    let port = multicast_addr.port();
    let bind_addr = SocketAddr::from(([0, 0, 0, 0], port));
    raw_socket.bind(&bind_addr.into())?;
    raw_socket.set_nonblocking(true)?;

    let std_socket: std::net::UdpSocket = raw_socket.into();
    let socket = UdpSocket::from_std(std_socket)?;

    let ip = match multicast_addr.ip() {
        std::net::IpAddr::V4(ipv4) => ipv4,
        std::net::IpAddr::V6(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Only IPv4 multicast is supported",
            )
            .into());
        }
    };

    let join_interface = match bind_ip {
        std::net::IpAddr::V4(ipv4) => ipv4,
        std::net::IpAddr::V6(_) => Ipv4Addr::UNSPECIFIED,
    };
    socket.join_multicast_v4(ip, join_interface)?;

    let mut buf = [0u8; 1024];
    let mut clean_interval = time::interval(Duration::from_secs(5));

    loop {
        tokio::select! {
            res = socket.recv_from(&mut buf) => {
                match res {
                    Ok((len, src_addr)) => {
                        let parsed = std::str::from_utf8(&buf[..len])
                            .ok()
                            .and_then(|s| toml::from_str::<PeerAnnouncement>(s).ok());

                        if let Some(announcement) = parsed {
                            // Ignore self-announcements
                            if announcement.fingerprint == my_fingerprint {
                                continue;
                            }

                            let mut tcp_addr = src_addr;
                            tcp_addr.set_port(announcement.port);

                            let discovered = DiscoveredPeer {
                                name: announcement.name,
                                fingerprint: announcement.fingerprint.clone(),
                                addr: tcp_addr,
                                last_seen: Instant::now(),
                            };

                            let mut map = peers.write().await;
                            map.insert(announcement.fingerprint, discovered);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "multicast socket recv_from error");
                    }
                }
            }
            _ = clean_interval.tick() => {
                let mut map = peers.write().await;
                map.clean_expired(Duration::from_secs(10));
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
