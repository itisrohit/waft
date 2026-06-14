use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};
use waft::discovery::{DiscoveredPeer, PeerAnnouncement, PeerMap, parse_announcement};

fn valid_fingerprint() -> &'static str {
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
}

fn valid_payload(name: &str, fingerprint: &str, port: u16) -> Result<Vec<u8>, anyhow::Error> {
    Ok(toml::to_string(&PeerAnnouncement {
        name: name.to_string(),
        fingerprint: fingerprint.to_string(),
        port,
    })
    .map_err(|err| anyhow::anyhow!("announcement should serialize: {err}"))?
    .into_bytes())
}

#[test]
fn test_parse_announcement_accepts_valid_payload() -> Result<(), anyhow::Error> {
    let src_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 40000);
    let peer = parse_announcement(
        valid_payload("PeerA", valid_fingerprint(), 9999)?.as_slice(),
        src_addr,
        "fingerprint_B",
    )
    .ok_or_else(|| anyhow::anyhow!("valid payload should parse"))?;

    assert_eq!(peer.name, "PeerA");
    assert_eq!(peer.fingerprint, valid_fingerprint());
    assert_eq!(
        peer.addr,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9999)
    );
    Ok(())
}

#[test]
fn test_parse_announcement_rejects_self_and_invalid_payloads() -> Result<(), anyhow::Error> {
    let src_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 40000);

    let cases = [
        valid_payload("PeerA", valid_fingerprint(), 0)?,
        valid_payload(
            "0123456789012345678901234567890123456789012345678901234567890123",
            valid_fingerprint(),
            7777,
        )?,
        valid_payload("PeerA", "abc", 7777)?,
        valid_payload(
            "PeerA",
            "z123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            7777,
        )?,
        valid_payload("PeerA", valid_fingerprint(), 7777)?,
        b"not valid toml".to_vec(),
    ];

    assert!(parse_announcement(cases[0].as_slice(), src_addr, "fingerprint_B").is_none());
    assert!(parse_announcement(cases[1].as_slice(), src_addr, "fingerprint_B").is_none());
    assert!(parse_announcement(cases[2].as_slice(), src_addr, "fingerprint_B").is_none());
    assert!(parse_announcement(cases[3].as_slice(), src_addr, "fingerprint_B").is_none());
    assert!(parse_announcement(cases[4].as_slice(), src_addr, valid_fingerprint()).is_none());
    assert!(parse_announcement(cases[5].as_slice(), src_addr, "fingerprint_B").is_none());
    Ok(())
}

#[test]
fn test_peer_map_updates_and_throttles() {
    let mut map = PeerMap::new();
    let fingerprint = valid_fingerprint().to_string();
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9999);

    map.insert(
        fingerprint.clone(),
        DiscoveredPeer {
            name: "PeerA".to_string(),
            fingerprint: fingerprint.clone(),
            addr,
            last_seen: Instant::now(),
        },
    );

    assert_eq!(map.get_all().len(), 1);
    assert!(!map.should_update(&fingerprint, "PeerA", addr, Duration::from_secs(5)));
    assert!(map.should_update(&fingerprint, "PeerRenamed", addr, Duration::from_secs(5)));
    assert!(map.should_update(
        &fingerprint,
        "PeerA",
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9000),
        Duration::from_secs(5)
    ));
}

#[test]
fn test_peer_map_cleans_expired_entries() {
    let mut map = PeerMap::new();
    let fingerprint = valid_fingerprint().to_string();

    map.insert(
        fingerprint.clone(),
        DiscoveredPeer {
            name: "PeerA".to_string(),
            fingerprint,
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9999),
            last_seen: Instant::now()
                .checked_sub(Duration::from_secs(2))
                .unwrap_or_else(Instant::now),
        },
    );

    map.clean_expired(Duration::from_secs(1));
    assert!(map.get_all().is_empty());
}
