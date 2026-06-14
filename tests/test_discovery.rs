use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time;
use waft::discovery::{PeerMap, start_announcer, start_listener};

#[tokio::test]
async fn test_peer_appears_on_announce() -> Result<(), anyhow::Error> {
    let multicast_addr: SocketAddr = "239.255.77.77:21001".parse()?;
    let bind_ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let peers = Arc::new(RwLock::new(PeerMap::new()));

    let (announcer_sender, announcer_receiver) = tokio::sync::watch::channel(false);
    let (listener_sender, listener_receiver) = tokio::sync::watch::channel(false);

    // Start listener on peer B
    let peers_clone = Arc::clone(&peers);
    let listener_handle = tokio::spawn(async move {
        let _ = start_listener(
            "fingerprint_B".to_string(),
            peers_clone,
            multicast_addr,
            bind_ip,
            listener_receiver,
        )
        .await;
    });

    // Start announcer on peer A
    let announcer_handle = tokio::spawn(async move {
        let _ = start_announcer(
            "PeerA".to_string(),
            "fingerprint_A".to_string(),
            9999,
            multicast_addr,
            bind_ip,
            announcer_receiver,
        )
        .await;
    });

    // Poll until peer A is discovered (timeout 3 seconds)
    let mut found = false;
    for _ in 0..30 {
        time::sleep(Duration::from_millis(100)).await;
        let map = peers.read().await;
        let active_peers = map.get_all();
        if active_peers
            .iter()
            .any(|p| p.fingerprint == "fingerprint_A")
        {
            let peer = &active_peers[0];
            assert_eq!(peer.name, "PeerA");
            assert_eq!(peer.addr.port(), 9999);
            found = true;
            break;
        }
    }
    assert!(found, "Peer A was not discovered within 3 seconds");

    // Clean shutdown
    let _ = announcer_sender.send(true);
    let _ = listener_sender.send(true);
    let _ = announcer_handle.await;
    let _ = listener_handle.await;

    Ok(())
}

#[tokio::test]
async fn test_peer_disappears_on_timeout() -> Result<(), anyhow::Error> {
    let multicast_addr: SocketAddr = "239.255.77.77:21002".parse()?;
    let bind_ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let peers = Arc::new(RwLock::new(PeerMap::new()));

    let (announcer_sender, announcer_receiver) = tokio::sync::watch::channel(false);
    let (listener_sender, listener_receiver) = tokio::sync::watch::channel(false);

    // Start listener on peer B
    let peers_clone = Arc::clone(&peers);
    let listener_handle = tokio::spawn(async move {
        let _ = start_listener(
            "fingerprint_B".to_string(),
            peers_clone,
            multicast_addr,
            bind_ip,
            listener_receiver,
        )
        .await;
    });

    // Start announcer on peer A
    let announcer_handle = tokio::spawn(async move {
        let _ = start_announcer(
            "PeerA".to_string(),
            "fingerprint_A".to_string(),
            9999,
            multicast_addr,
            bind_ip,
            announcer_receiver,
        )
        .await;
    });

    // Wait until peer A is discovered
    let mut found = false;
    for _ in 0..30 {
        time::sleep(Duration::from_millis(100)).await;
        let map = peers.read().await;
        if map
            .get_all()
            .iter()
            .any(|p| p.fingerprint == "fingerprint_A")
        {
            found = true;
            break;
        }
    }
    assert!(found, "Peer A was not discovered");

    // Stop announcer A
    let _ = announcer_sender.send(true);
    let _ = announcer_handle.await;

    // Simulate expiration check: we manually clean with a tiny timeout (e.g. 50ms)
    // after sleeping a bit to make sure it expires
    time::sleep(Duration::from_millis(200)).await;
    {
        let mut map = peers.write().await;
        map.clean_expired(Duration::from_millis(50));
        let active_peers = map.get_all();
        assert!(
            !active_peers
                .iter()
                .any(|p| p.fingerprint == "fingerprint_A"),
            "Peer A should have been cleaned up"
        );
    }

    // Clean shutdown listener
    let _ = listener_sender.send(true);
    let _ = listener_handle.await;

    Ok(())
}

#[tokio::test]
async fn test_no_self_discovery() -> Result<(), anyhow::Error> {
    let multicast_addr: SocketAddr = "239.255.77.77:21003".parse()?;
    let bind_ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let peers = Arc::new(RwLock::new(PeerMap::new()));

    let (announcer_sender, announcer_receiver) = tokio::sync::watch::channel(false);
    let (listener_sender, listener_receiver) = tokio::sync::watch::channel(false);

    // Start listener on peer A
    let peers_clone = Arc::clone(&peers);
    let listener_handle = tokio::spawn(async move {
        let _ = start_listener(
            "fingerprint_A".to_string(),
            peers_clone,
            multicast_addr,
            bind_ip,
            listener_receiver,
        )
        .await;
    });

    // Start announcer on peer A
    let announcer_handle = tokio::spawn(async move {
        let _ = start_announcer(
            "PeerA".to_string(),
            "fingerprint_A".to_string(),
            9999,
            multicast_addr,
            bind_ip,
            announcer_receiver,
        )
        .await;
    });

    // Wait 1.5 seconds and assert A's map is still empty
    time::sleep(Duration::from_millis(1500)).await;
    {
        let map = peers.read().await;
        assert!(map.get_all().is_empty(), "Should not discover itself");
    }

    // Clean shutdown
    let _ = announcer_sender.send(true);
    let _ = listener_sender.send(true);
    let _ = announcer_handle.await;
    let _ = listener_handle.await;

    Ok(())
}
