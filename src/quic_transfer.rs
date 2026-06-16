//! QUIC receiver server and wire protocol implementation.

use crate::error::WaftError;
use crate::transfer::{fingerprint_from_bytes, hex, parse_fixed_header, sanitize_filename};
use crate::trust::{TrustStore, TrustTier};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tracing::{error, info, warn};

const CHUNK_SIZE: usize = 2 * 1024 * 1024; // 2MB
const READ_TIMEOUT_SECS: u64 = 10;

/// Starts the QUIC receiver server listening on the specified address.
///
/// # Errors
/// Returns an error if the UDP listener/QUIC endpoint fails to bind.
pub async fn start_quic_receiver(
    bind_addr: SocketAddr,
    trust_store: Arc<TrustStore>,
    download_dir: PathBuf,
) -> Result<(), WaftError> {
    let server_config = crate::quic_transport::make_server_config()
        .map_err(|e| WaftError::Io(std::io::Error::other(e.to_string())))?;

    let endpoint = quinn::Endpoint::server(server_config, bind_addr)?;
    info!(addr = %bind_addr, "QUIC receiver started");

    while let Some(incoming) = endpoint.accept().await {
        let trust = Arc::clone(&trust_store);
        let downloads = download_dir.clone();
        tokio::spawn(async move {
            match incoming.accept() {
                Ok(conn) => {
                    if let Err(e) = handle_quic_connection(conn, trust, downloads).await {
                        error!("Error handling QUIC connection: {e}");
                    }
                }
                Err(e) => {
                    error!("Failed to accept incoming connection attempt: {e}");
                }
            }
        });
    }

    Ok(())
}

async fn handle_quic_connection(
    conn: quinn::Connecting,
    trust_store: Arc<TrustStore>,
    download_dir: PathBuf,
) -> Result<(), WaftError> {
    let peer_ip = conn.remote_address();
    let connection = conn.await.map_err(|e| {
        WaftError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            format!("QUIC connection handshake failed: {e}"),
        ))
    })?;

    info!(peer = %peer_ip, "QUIC connection established");

    // Accept bidirectional streams from this connection
    while let Ok((mut send_stream, mut recv_stream)) = connection.accept_bi().await {
        let trust = Arc::clone(&trust_store);
        let downloads = download_dir.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_quic_stream(
                &mut send_stream,
                &mut recv_stream,
                peer_ip,
                trust,
                downloads,
            )
            .await
            {
                error!(peer = %peer_ip, error = %e, "Error handling QUIC stream");
            }
        });
    }

    Ok(())
}

struct TransferHeader {
    filename: PathBuf,
    file_size: u64,
    expected_hash: [u8; 32],
    fingerprint: String,
    tier: TrustTier,
}

async fn read_exact_with_timeout<const N: usize>(
    recv_stream: &mut quinn::RecvStream,
) -> Result<[u8; N], WaftError> {
    let mut buf = [0u8; N];
    tokio::time::timeout(
        std::time::Duration::from_secs(READ_TIMEOUT_SECS),
        recv_stream.read_exact(&mut buf),
    )
    .await
    .map_err(|_| {
        WaftError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "Header read timed out",
        ))
    })?
    .map_err(|e| WaftError::Io(std::io::Error::other(e)))?;
    Ok(buf)
}

async fn read_exact_vec_with_timeout(
    recv_stream: &mut quinn::RecvStream,
    len: usize,
) -> Result<Vec<u8>, WaftError> {
    let mut buf = vec![0u8; len];
    tokio::time::timeout(
        std::time::Duration::from_secs(READ_TIMEOUT_SECS),
        recv_stream.read_exact(&mut buf),
    )
    .await
    .map_err(|_| {
        WaftError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "Metadata payload read timed out",
        ))
    })?
    .map_err(|e| WaftError::Io(std::io::Error::other(e)))?;
    Ok(buf)
}

async fn read_and_verify_header(
    send_stream: &mut quinn::SendStream,
    recv_stream: &mut quinn::RecvStream,
    trust_store: &TrustStore,
) -> Result<TransferHeader, WaftError> {
    // 1. Read the 64-byte fixed metadata block
    let header_bytes = read_exact_with_timeout::<64>(recv_stream).await?;

    let (name_len, file_size, expected_hash) = parse_fixed_header(&header_bytes)?;

    // 2. Read variable-length filename string
    let name_bytes = read_exact_vec_with_timeout(recv_stream, name_len).await?;
    let raw_name = String::from_utf8(name_bytes)
        .map_err(|e| WaftError::InvalidHeader(format!("Filename is not valid UTF-8: {e}")))?;

    // Securely resolve filename to prevent path traversal
    let filename = sanitize_filename(&raw_name)?;

    // 3. Read sender's public key (32 bytes)
    let pubkey_bytes = read_exact_with_timeout::<32>(recv_stream).await?;
    let verifying_key = VerifyingKey::from_bytes(&pubkey_bytes)
        .map_err(|e| WaftError::SignatureVerification(e.to_string()))?;

    // 4. Read sender's signature (64 bytes)
    let sig_bytes = read_exact_with_timeout::<64>(recv_stream).await?;
    let signature = Signature::from_bytes(&sig_bytes);

    // Verify cryptographic signature over: 64-byte header + raw name bytes
    let mut signed_message = Vec::with_capacity(64 + name_len);
    signed_message.extend_from_slice(&header_bytes);
    signed_message.extend_from_slice(raw_name.as_bytes());

    verifying_key
        .verify(&signed_message, &signature)
        .map_err(|e| WaftError::SignatureVerification(e.to_string()))?;

    // Check fingerprint trust tier
    let fingerprint = fingerprint_from_bytes(&pubkey_bytes);
    let mut tier = trust_store.get_tier(&fingerprint);

    if tier == TrustTier::Blocked {
        warn!(fingerprint = %fingerprint, "Connection rejected: peer is blocked");
        let _ = send_stream.write_all(&[0x00]).await; // REJECT
        return Err(WaftError::Rejected);
    }

    // Auto-promote Ask to Trusted on first file acceptance
    if tier == TrustTier::Ask {
        info!(fingerprint = %fingerprint, "Promoting new peer to Trusted tier on first transfer");
        trust_store.set_tier(&fingerprint, TrustTier::Trusted)?;
        tier = TrustTier::Trusted;
    }

    Ok(TransferHeader {
        filename,
        file_size,
        expected_hash,
        fingerprint,
        tier,
    })
}

async fn handle_quic_stream(
    send_stream: &mut quinn::SendStream,
    recv_stream: &mut quinn::RecvStream,
    peer_ip: SocketAddr,
    trust_store: Arc<TrustStore>,
    download_dir: PathBuf,
) -> Result<(), WaftError> {
    info!(peer = %peer_ip, "Handling incoming file transfer over QUIC stream");

    let header = read_and_verify_header(send_stream, recv_stream, &trust_store).await?;

    info!(
        filename = ?header.filename,
        size = header.file_size,
        peer_fingerprint = %header.fingerprint,
        tier = ?header.tier,
        "Accepting QUIC file transfer request"
    );

    send_stream
        .write_all(&[0x01])
        .await
        .map_err(std::io::Error::other)?; // ACK / ACCEPT
    send_stream.flush().await?;

    // Prepare target file path
    let file_path = download_dir.join(&header.filename);
    tokio::fs::create_dir_all(&download_dir).await?;

    let mut file = match tokio::fs::File::create(&file_path).await {
        Ok(f) => f,
        Err(e) => {
            error!(error = %e, "Failed to create target file on disk");
            let _ = send_stream.write_all(&[0x00]).await;
            return Err(WaftError::Io(e));
        }
    };

    // Stream and write file body in CHUNK_SIZE chunks while computing BLAKE3 hash
    let mut hasher = blake3::Hasher::new();
    let mut remaining = header.file_size;
    let mut buffer = vec![0u8; CHUNK_SIZE];

    while remaining > 0 {
        let to_read = usize::try_from(std::cmp::min(remaining, CHUNK_SIZE as u64))
            .map_err(|_| WaftError::InvalidHeader("Chunk size exceeds usize limits".into()))?;

        // Apply read timeout to body stream chunks to prevent slowloris hanging
        let n = match tokio::time::timeout(
            std::time::Duration::from_secs(READ_TIMEOUT_SECS),
            recv_stream.read(&mut buffer[..to_read]),
        )
        .await
        {
            Ok(Ok(None)) => {
                // Premature EOF
                let _ = tokio::fs::remove_file(&file_path).await;
                return Err(WaftError::Interrupted {
                    bytes_sent: header.file_size - remaining,
                });
            }
            Ok(Ok(Some(n))) => n,
            Ok(Err(e)) => {
                let _ = tokio::fs::remove_file(&file_path).await;
                return Err(WaftError::Io(std::io::Error::other(e)));
            }
            Err(_) => {
                let _ = tokio::fs::remove_file(&file_path).await;
                return Err(WaftError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Body read timed out",
                )));
            }
        };

        file.write_all(&buffer[..n]).await?;
        hasher.update(&buffer[..n]);
        remaining -= n as u64;
    }

    // Finalize hash and verify
    let computed_hash = hasher.finalize();
    if computed_hash.as_bytes() != &header.expected_hash {
        error!(
            expected = %hex::encode(header.expected_hash),
            actual = %computed_hash.to_hex(),
            "BLAKE3 hash verification failed. Deleting corrupted file."
        );
        let _ = tokio::fs::remove_file(&file_path).await;
        send_stream
            .write_all(&[0x00])
            .await
            .map_err(std::io::Error::other)?; // Hash Mismatch / Failure
        return Err(WaftError::HashMismatch {
            expected: hex::encode(header.expected_hash),
            actual: computed_hash.to_hex().to_string(),
        });
    }

    // Ensure all data is fully flushed and synced to disk before acknowledging success
    file.flush().await?;
    file.sync_all().await?;

    info!(filename = ?header.filename, "File received and verified successfully over QUIC");
    send_stream
        .write_all(&[0x02])
        .await
        .map_err(std::io::Error::other)?; // DONE / SUCCESS
    send_stream.flush().await?;

    Ok(())
}
