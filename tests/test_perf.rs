#![allow(clippy::cast_precision_loss, clippy::uninlined_format_args)]

use std::fs;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use waft::identity::Identity;
use waft::send::send_file;
use waft::transfer::start_receiver;
use waft::trust::TrustStore;

/// Helper to generate a temp file with specific byte content.
fn create_temp_file(name: &str, size: usize) -> Result<(PathBuf, PathBuf), anyhow::Error> {
    let temp_dir = std::env::temp_dir();
    let src_path = temp_dir.join(format!("waft_perf_src_{name}"));

    // Write file in chunks to be memory efficient for large sizes
    let chunk = vec![0u8; 1024 * 1024]; // 1MB chunk
    let mut remaining = size;
    let mut file = std::fs::File::create(&src_path)?;
    while remaining > 0 {
        let to_write = std::cmp::min(remaining, chunk.len());
        file.write_all(&chunk[..to_write])?;
        remaining -= to_write;
    }

    let dest_dir = temp_dir.join(format!("waft_perf_dest_{name}"));
    if dest_dir.exists() {
        fs::remove_dir_all(&dest_dir)?;
    }
    fs::create_dir_all(&dest_dir)?;
    Ok((src_path, dest_dir))
}

async fn wait_for_port(addr: SocketAddr) -> Result<(), anyhow::Error> {
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }
    anyhow::bail!("Receiver did not start listening on {addr} within 1 second");
}

#[tokio::test]
async fn perf_1mb_under_100ms() -> Result<(), anyhow::Error> {
    if cfg!(debug_assertions) {
        println!("Skipping performance test in debug mode");
        return Ok(());
    }
    let size = 1024 * 1024; // 1MB
    let (src_path, dest_dir) = create_temp_file("1mb", size)?;

    let trust_path = std::env::temp_dir().join("waft_perf_trust_1mb");
    if trust_path.exists() {
        let _ = fs::remove_file(&trust_path);
    }
    let trust_store = Arc::new(TrustStore::load_or_create(&trust_path)?);
    let sender_identity = Identity::generate();

    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    drop(listener);

    let trust = Arc::clone(&trust_store);
    let dest = dest_dir.clone();
    let receiver_handle = tokio::spawn(async move {
        let _ = start_receiver(local_addr, trust, dest).await;
    });

    wait_for_port(local_addr).await?;

    let start = Instant::now();
    send_file(local_addr, &sender_identity, &src_path, None).await?;
    let elapsed = start.elapsed();

    println!("1MB Loopback Roundtrip: {:?}", elapsed);

    // Verify file exists
    let dest_file_path = dest_dir.join("waft_perf_src_1mb");
    assert!(dest_file_path.exists());
    assert_eq!(fs::metadata(&dest_file_path)?.len(), size as u64);

    // Under 100ms target (we allow a fallback margin of 300ms on heavily loaded CI runners)
    let limit = if std::env::var("CI").is_ok() {
        std::time::Duration::from_millis(300)
    } else {
        std::time::Duration::from_millis(100)
    };
    assert!(
        elapsed < limit,
        "1MB transfer took too long: elapsed={:?}, limit={:?}",
        elapsed,
        limit
    );

    // Cleanup
    let _ = fs::remove_file(&src_path);
    let _ = fs::remove_dir_all(&dest_dir);
    let _ = fs::remove_file(&trust_path);
    receiver_handle.abort();
    Ok(())
}

#[tokio::test]
async fn perf_100mb_throughput_floor() -> Result<(), anyhow::Error> {
    if cfg!(debug_assertions) {
        println!("Skipping performance test in debug mode");
        return Ok(());
    }
    let size = 100 * 1024 * 1024; // 100MB
    let (src_path, dest_dir) = create_temp_file("100mb", size)?;

    let trust_path = std::env::temp_dir().join("waft_perf_trust_100mb");
    if trust_path.exists() {
        let _ = fs::remove_file(&trust_path);
    }
    let trust_store = Arc::new(TrustStore::load_or_create(&trust_path)?);
    let sender_identity = Identity::generate();

    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    drop(listener);

    let trust = Arc::clone(&trust_store);
    let dest = dest_dir.clone();
    let receiver_handle = tokio::spawn(async move {
        let _ = start_receiver(local_addr, trust, dest).await;
    });

    wait_for_port(local_addr).await?;

    let start = Instant::now();
    send_file(local_addr, &sender_identity, &src_path, None).await?;
    let elapsed = start.elapsed();

    let secs = elapsed.as_secs_f64();
    let mb_per_sec = (size as f64 / (1024.0 * 1024.0)) / secs;
    println!(
        "100MB Throughput: {:.2} MB/s (elapsed: {:.2}s)",
        mb_per_sec, secs
    );

    // Verify file exists
    let dest_file_path = dest_dir.join("waft_perf_src_100mb");
    assert!(dest_file_path.exists());
    assert_eq!(fs::metadata(&dest_file_path)?.len(), size as u64);

    // Throughput floor target of 300 MB/s on local machine (we set a relaxed limit of 100 MB/s for CI runners)
    let floor = if std::env::var("CI").is_ok() {
        100.0
    } else {
        300.0
    };

    assert!(
        mb_per_sec >= floor,
        "100MB throughput below floor: {:.2} MB/s (floor: {:.2} MB/s)",
        mb_per_sec,
        floor
    );

    // Cleanup
    let _ = fs::remove_file(&src_path);
    let _ = fs::remove_dir_all(&dest_dir);
    let _ = fs::remove_file(&trust_path);
    receiver_handle.abort();
    Ok(())
}

#[tokio::test]
async fn perf_small_file_latency() -> Result<(), anyhow::Error> {
    if cfg!(debug_assertions) {
        println!("Skipping performance test in debug mode");
        return Ok(());
    }
    let size = 1024; // 1KB
    let (src_path, dest_dir) = create_temp_file("1kb", size)?;

    let trust_path = std::env::temp_dir().join("waft_perf_trust_1kb");
    if trust_path.exists() {
        let _ = fs::remove_file(&trust_path);
    }
    let trust_store = Arc::new(TrustStore::load_or_create(&trust_path)?);
    let sender_identity = Identity::generate();

    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    drop(listener);

    let trust = Arc::clone(&trust_store);
    let dest = dest_dir.clone();
    let receiver_handle = tokio::spawn(async move {
        let _ = start_receiver(local_addr, trust, dest).await;
    });

    wait_for_port(local_addr).await?;

    let start = Instant::now();
    send_file(local_addr, &sender_identity, &src_path, None).await?;
    let elapsed = start.elapsed();

    println!("1KB Loopback Latency: {:?}", elapsed);

    // Verify file exists
    let dest_file_path = dest_dir.join("waft_perf_src_1kb");
    assert!(dest_file_path.exists());

    // Small file transfer target under 50ms (we allow 100ms on CI)
    let limit = if std::env::var("CI").is_ok() {
        std::time::Duration::from_millis(100)
    } else {
        std::time::Duration::from_millis(50)
    };

    assert!(
        elapsed < limit,
        "Small file transfer took too long: elapsed={:?}, limit={:?}",
        elapsed,
        limit
    );

    // Cleanup
    let _ = fs::remove_file(&src_path);
    let _ = fs::remove_dir_all(&dest_dir);
    let _ = fs::remove_file(&trust_path);
    receiver_handle.abort();
    Ok(())
}

#[tokio::test]
async fn test_tcp_resume_transfer() -> Result<(), anyhow::Error> {
    let size = 10 * 1024 * 1024; // 10MB
    let (src_path, dest_dir) = create_temp_file("tcp_resume", size)?;

    let trust_path = std::env::temp_dir().join("waft_perf_trust_tcp_resume");
    if trust_path.exists() {
        let _ = fs::remove_file(&trust_path);
    }
    let trust_store = Arc::new(TrustStore::load_or_create(&trust_path)?);
    let sender_identity = Identity::generate();

    // Allow connections
    trust_store.set_tier(
        &sender_identity.fingerprint(),
        waft::trust::TrustTier::Trusted,
    )?;

    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    drop(listener);

    let trust = Arc::clone(&trust_store);
    let dest = dest_dir.clone();
    let receiver_handle = tokio::spawn(async move {
        let _ = start_receiver(local_addr, trust, dest).await;
    });

    wait_for_port(local_addr).await?;

    // Part 1: Simulate interrupted transfer
    // Connect to receiver and send header + first 3MB of file
    let mut socket = tokio::net::TcpStream::connect(local_addr).await?;

    let file_data = fs::read(&src_path)?;
    let hash = blake3::hash(&file_data);
    let hash_bytes = hash.as_bytes();

    // Construct header
    let filename_str = "waft_perf_src_tcp_resume";
    let name_bytes = filename_str.as_bytes();
    let mut header_bytes = [0u8; 64];
    header_bytes[0..2].copy_from_slice(&[0xFA, 0x57]);
    header_bytes[2] = 1;
    let name_len_u16 = u16::try_from(name_bytes.len())
        .map_err(|e| anyhow::anyhow!("Filename length overflow: {e}"))?;
    header_bytes[4..6].copy_from_slice(&name_len_u16.to_be_bytes());
    header_bytes[6..14].copy_from_slice(&(size as u64).to_be_bytes());
    header_bytes[14..46].copy_from_slice(hash_bytes);

    let mut signed_msg = Vec::new();
    signed_msg.extend_from_slice(&header_bytes);
    signed_msg.extend_from_slice(name_bytes);
    let sig = sender_identity.sign(&signed_msg);

    // Send handshake
    socket.write_all(&header_bytes).await?;
    socket.write_all(name_bytes).await?;
    socket
        .write_all(&sender_identity.public_key().to_bytes())
        .await?;
    socket.write_all(&sig.to_bytes()).await?;
    socket.flush().await?;

    // Read ACK
    let mut ack = [0u8; 1];
    socket.read_exact(&mut ack).await?;
    assert_eq!(ack[0], 0x01); // starts from 0

    // Send 3MB
    let partial_size = 3 * 1024 * 1024;
    socket.write_all(&file_data[..partial_size]).await?;
    socket.flush().await?;

    // Drop socket to simulate interruption
    drop(socket);

    // Wait briefly for receiver to register EOF
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Verify the partial file exists in download dir
    let part_file_path = dest_dir.join(format!("{}.part", hash.to_hex()));
    assert!(part_file_path.exists());
    assert_eq!(fs::metadata(&part_file_path)?.len(), partial_size as u64);

    // Part 2: Resume the transfer using the actual send_file implementation
    send_file(local_addr, &sender_identity, &src_path, None).await?;

    // Verify the final file exists, matches original size, and part file is removed
    let dest_file_path = dest_dir.join(filename_str);
    assert!(dest_file_path.exists());
    assert_eq!(fs::metadata(&dest_file_path)?.len(), size as u64);
    assert!(!part_file_path.exists());

    // Cleanup
    let _ = fs::remove_file(&src_path);
    let _ = fs::remove_dir_all(&dest_dir);
    let _ = fs::remove_file(&trust_path);
    receiver_handle.abort();
    Ok(())
}
