use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time;
use waft::discovery::{DiscoveryConfig, PeerMap, start_announcer, start_listener};

#[tokio::test]
#[ignore = "requires multicast support on the runner"]
async fn multicast_smoke_discovers_peer() -> Result<(), anyhow::Error> {
    let multicast_addr: SocketAddr = "239.255.77.77:21001".parse()?;
    let peers = Arc::new(RwLock::new(PeerMap::new()));
    let (announcer_tx, announcer_rx) = tokio::sync::watch::channel(false);
    let (listener_tx, listener_rx) = tokio::sync::watch::channel(false);

    let config = DiscoveryConfig {
        bind_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        multicast_addr,
        announce_interval: Duration::from_millis(200),
        peer_timeout: Duration::from_secs(2),
    };

    let peers_clone = Arc::clone(&peers);
    let listener_config = config.clone();
    let listener = tokio::spawn(async move {
        let _ = start_listener(
            "fingerprint_B".into(),
            peers_clone,
            listener_config,
            listener_rx,
        )
        .await;
    });
    let announcer = tokio::spawn(async move {
        let _ = start_announcer(
            "PeerA".into(),
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
            9999,
            config,
            announcer_rx,
        )
        .await;
    });

    time::sleep(Duration::from_secs(1)).await;
    assert_eq!(peers.read().await.get_all().len(), 1);

    let _ = announcer_tx.send(true);
    let _ = listener_tx.send(true);
    let _ = announcer.await;
    let _ = listener.await;
    Ok(())
}
