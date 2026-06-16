use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use waft::identity::Identity;
use waft::send::send_file;
use waft::transfer::start_receiver;
use waft::trust::{TrustStore, TrustTier};

/// Helper to generate a temp file with specific byte content.
fn create_temp_file(name: &str, content: &[u8]) -> Result<(PathBuf, PathBuf), anyhow::Error> {
    let temp_dir = std::env::temp_dir();
    let src_path = temp_dir.join(format!("waft_test_src_{name}"));
    fs::write(&src_path, content)?;
    let dest_dir = temp_dir.join(format!("waft_test_dest_{name}"));
    if dest_dir.exists() {
        fs::remove_dir_all(&dest_dir)?;
    }
    fs::create_dir_all(&dest_dir)?;
    Ok((src_path, dest_dir))
}

/// Poll the given address until a TCP connection succeeds, confirming the
/// receiver is actually listening. Retries up to 50 times with 20ms spacing
/// (max 1 second), which is robust on slow CI runners without adding latency
/// on fast machines.
async fn wait_for_port(addr: std::net::SocketAddr) -> Result<(), anyhow::Error> {
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }
    anyhow::bail!("Receiver did not start listening on {addr} within 1 second");
}

#[tokio::test]
async fn test_small_file_transfer() -> Result<(), anyhow::Error> {
    let content = b"Hello world! Waft file transfer integration test.";
    let (src_path, dest_dir) = create_temp_file("small", content)?;

    // Load temp trust store
    let trust_path = std::env::temp_dir().join("waft_test_trust_small");
    if trust_path.exists() {
        fs::remove_file(&trust_path)?;
    }
    let trust_store = Arc::new(TrustStore::load_or_create(&trust_path)?);

    // Generate identities
    let sender_identity = Identity::generate();
    let _receiver_identity = Identity::generate();

    // Bind receiver to loopback on ephemeral port
    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    drop(listener); // release so start_receiver can bind it

    let trust = Arc::clone(&trust_store);
    let dest = dest_dir.clone();
    let receiver_handle =
        tokio::spawn(async move { start_receiver(local_addr, trust, dest).await });

    // Wait 100ms for listener to bind
    wait_for_port(local_addr).await?;

    // Send file
    send_file(local_addr, &sender_identity, &src_path, None).await?;

    // Verify file received and contents match
    let dest_file_path = dest_dir.join("waft_test_src_small");
    assert!(dest_file_path.exists());
    let received_content = fs::read(&dest_file_path)?;
    assert_eq!(received_content, content);

    // Verify sender was promoted from Ask to Trusted
    let sender_fingerprint = sender_identity.fingerprint();
    assert_eq!(
        trust_store.get_tier(&sender_fingerprint),
        TrustTier::Trusted
    );

    // Cleanup
    let _ = fs::remove_file(&src_path);
    let _ = fs::remove_dir_all(&dest_dir);
    let _ = fs::remove_file(&trust_path);
    receiver_handle.abort();
    Ok(())
}

#[tokio::test]
async fn test_large_file_transfer() -> Result<(), anyhow::Error> {
    // Generate a 5MB random file
    let mut content = vec![0u8; 5 * 1024 * 1024];
    for (i, val) in content.iter_mut().enumerate() {
        *val = u8::try_from(i % 256).unwrap_or(0);
    }
    let (src_path, dest_dir) = create_temp_file("large", &content)?;

    let trust_path = std::env::temp_dir().join("waft_test_trust_large");
    if trust_path.exists() {
        fs::remove_file(&trust_path)?;
    }
    let trust_store = Arc::new(TrustStore::load_or_create(&trust_path)?);

    let sender_identity = Identity::generate();

    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    drop(listener);

    let trust = Arc::clone(&trust_store);
    let dest = dest_dir.clone();
    let receiver_handle =
        tokio::spawn(async move { start_receiver(local_addr, trust, dest).await });

    wait_for_port(local_addr).await?;

    send_file(local_addr, &sender_identity, &src_path, None).await?;

    let dest_file_path = dest_dir.join("waft_test_src_large");
    assert!(dest_file_path.exists());
    let received_content = fs::read(&dest_file_path)?;
    assert_eq!(received_content, content);

    let _ = fs::remove_file(&src_path);
    let _ = fs::remove_dir_all(&dest_dir);
    let _ = fs::remove_file(&trust_path);
    receiver_handle.abort();
    Ok(())
}

#[tokio::test]
async fn test_invalid_magic_rejected() -> Result<(), anyhow::Error> {
    let content = b"Dummy content";
    let (_src_path, dest_dir) = create_temp_file("invalid_magic", content)?;

    let trust_path = std::env::temp_dir().join("waft_test_trust_magic");
    if trust_path.exists() {
        fs::remove_file(&trust_path)?;
    }
    let trust_store = Arc::new(TrustStore::load_or_create(&trust_path)?);

    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    drop(listener);

    let trust = Arc::clone(&trust_store);
    let dest = dest_dir.clone();
    let receiver_handle =
        tokio::spawn(async move { start_receiver(local_addr, trust, dest).await });

    wait_for_port(local_addr).await?;

    // Connect manually and send a full 64-byte header with bad magic bytes
    let mut socket = TcpStream::connect(local_addr).await?;
    let mut bad_header = [0u8; 64];
    bad_header[0..2].copy_from_slice(&[0x00, 0x00]); // invalid magic
    socket.write_all(&bad_header).await?;
    socket.flush().await?;

    // Read response, socket should close or error
    let mut buf = [0u8; 10];
    let n = socket.read(&mut buf).await?;
    assert_eq!(n, 0); // closed by server due to header error

    let _ = fs::remove_dir_all(&dest_dir);
    let _ = fs::remove_file(&trust_path);
    receiver_handle.abort();
    Ok(())
}

#[tokio::test]
async fn test_blocked_peer_rejected() -> Result<(), anyhow::Error> {
    let content = b"Secret data";
    let (src_path, dest_dir) = create_temp_file("blocked", content)?;

    let trust_path = std::env::temp_dir().join("waft_test_trust_blocked");
    if trust_path.exists() {
        fs::remove_file(&trust_path)?;
    }
    let trust_store = Arc::new(TrustStore::load_or_create(&trust_path)?);

    let sender_identity = Identity::generate();
    let sender_fingerprint = sender_identity.fingerprint();

    // Mark sender as blocked in the store first
    trust_store.set_tier(&sender_fingerprint, TrustTier::Blocked)?;

    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    drop(listener);

    let trust = Arc::clone(&trust_store);
    let dest = dest_dir.clone();
    let receiver_handle =
        tokio::spawn(async move { start_receiver(local_addr, trust, dest).await });

    wait_for_port(local_addr).await?;

    // Attempt to send, should fail with Rejected error
    let send_result = send_file(local_addr, &sender_identity, &src_path, None).await;
    assert!(send_result.is_err());

    let dest_file_path = dest_dir.join("waft_test_src_blocked");
    assert!(!dest_file_path.exists()); // never received

    let _ = fs::remove_file(&src_path);
    let _ = fs::remove_dir_all(&dest_dir);
    let _ = fs::remove_file(&trust_path);
    receiver_handle.abort();
    Ok(())
}

#[tokio::test]
async fn test_interrupted_transfer_cleanup() -> Result<(), anyhow::Error> {
    let content = vec![0u8; 1024 * 1024]; // 1MB content
    let (src_path, dest_dir) = create_temp_file("interrupted", &content)?;

    let trust_path = std::env::temp_dir().join("waft_test_trust_interrupted");
    if trust_path.exists() {
        fs::remove_file(&trust_path)?;
    }
    let trust_store = Arc::new(TrustStore::load_or_create(&trust_path)?);

    let sender_identity = Identity::generate();

    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    drop(listener);

    let trust = Arc::clone(&trust_store);
    let dest = dest_dir.clone();
    let receiver_handle =
        tokio::spawn(async move { start_receiver(local_addr, trust, dest).await });

    wait_for_port(local_addr).await?;

    // Send part of the header manually then abort
    let mut socket = TcpStream::connect(local_addr).await?;

    let hash_bytes = blake3::hash(&content);
    let mut header_bytes = [0u8; 64];
    header_bytes[0..2].copy_from_slice(&[0xFA, 0x57]);
    header_bytes[2] = 1;
    header_bytes[3] = 0;
    let raw_name = "waft_test_src_interrupted";
    let name_len_u16 = u16::try_from(raw_name.len()).unwrap_or(0);
    header_bytes[4..6].copy_from_slice(&name_len_u16.to_be_bytes());
    header_bytes[6..14].copy_from_slice(&(content.len() as u64).to_be_bytes());
    header_bytes[14..46].copy_from_slice(hash_bytes.as_bytes());

    let mut signed_message = Vec::new();
    signed_message.extend_from_slice(&header_bytes);
    signed_message.extend_from_slice(raw_name.as_bytes());
    let sig_bytes = sender_identity.sign(&signed_message).to_bytes();

    socket.write_all(&header_bytes).await?;
    socket.write_all(raw_name.as_bytes()).await?;
    socket
        .write_all(&sender_identity.public_key().to_bytes())
        .await?;
    socket.write_all(&sig_bytes).await?;
    socket.flush().await?;

    // Read the ACK response (should be 0x01)
    let mut ack = [0u8; 1];
    socket.read_exact(&mut ack).await?;
    assert_eq!(ack[0], 0x01);

    // Send only 10KB of data, then drop the socket to simulate interruption
    socket.write_all(&content[..10 * 1024]).await?;
    socket.flush().await?;
    drop(socket);

    // Give the receiver time to observe the disconnect and remove the partial file.
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify partial file was deleted/cleaned up
    let dest_file_path = dest_dir.join("waft_test_src_interrupted");
    assert!(!dest_file_path.exists());

    let _ = fs::remove_file(&src_path);
    let _ = fs::remove_dir_all(&dest_dir);
    let _ = fs::remove_file(&trust_path);
    receiver_handle.abort();
    Ok(())
}

#[tokio::test]
async fn test_read_timeout_trigger() -> Result<(), anyhow::Error> {
    let trust_path = std::env::temp_dir().join("waft_test_trust_timeout");
    if trust_path.exists() {
        fs::remove_file(&trust_path)?;
    }
    let trust_store = Arc::new(TrustStore::load_or_create(&trust_path)?);

    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    drop(listener);

    let trust = Arc::clone(&trust_store);
    let dest_dir = std::env::temp_dir().join("waft_test_dest_timeout");
    let receiver_handle =
        tokio::spawn(async move { start_receiver(local_addr, trust, dest_dir).await });

    wait_for_port(local_addr).await?;

    // Connect manually and send only 10 bytes (header needs 64)
    let mut socket = TcpStream::connect(local_addr).await?;
    socket.write_all(&[0x00; 10]).await?;
    socket.flush().await?;

    // Pause tokio time and advance it past the 10-second read timeout
    tokio::time::pause();
    tokio::time::advance(tokio::time::Duration::from_secs(12)).await;
    tokio::time::resume();

    // Read response, socket should be closed by server due to read timeout
    let mut buf = [0u8; 10];
    let n = socket.read(&mut buf).await?;
    assert_eq!(n, 0); // socket closed cleanly by server on timeout

    let _ = fs::remove_file(&trust_path);
    receiver_handle.abort();
    Ok(())
}

#[tokio::test]
async fn test_header_fuzzing_robustness() -> Result<(), anyhow::Error> {
    let trust_path = std::env::temp_dir().join("waft_test_trust_fuzz");
    if trust_path.exists() {
        fs::remove_file(&trust_path)?;
    }
    let trust_store = Arc::new(TrustStore::load_or_create(&trust_path)?);

    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    drop(listener);

    let trust = Arc::clone(&trust_store);
    let dest_dir = std::env::temp_dir().join("waft_test_dest_fuzz");
    let receiver_handle =
        tokio::spawn(async move { start_receiver(local_addr, trust, dest_dir).await });

    wait_for_port(local_addr).await?;

    // Fuzz with random lengths and content
    for len in [1, 2, 10, 45, 63, 64, 65, 128, 200] {
        let mut socket = TcpStream::connect(local_addr).await?;
        let bad_payload = vec![0xAA; len]; // fuzzed bytes
        socket.write_all(&bad_payload).await?;
        socket.flush().await?;
        let _ = socket.shutdown().await; // signal EOF to the server to prevent hanging

        // Read response, socket should close (clean EOF, connection reset, or broken pipe)
        let mut buf = [0u8; 10];
        match socket.read(&mut buf).await {
            Ok(n) => assert_eq!(n, 0),
            Err(e) => {
                let kind = e.kind();
                assert!(
                    kind == std::io::ErrorKind::ConnectionReset
                        || kind == std::io::ErrorKind::BrokenPipe
                        || kind == std::io::ErrorKind::ConnectionAborted,
                    "Unexpected read error: {e:?}"
                );
            }
        }
    }

    let _ = fs::remove_file(&trust_path);
    receiver_handle.abort();
    Ok(())
}
