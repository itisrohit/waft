//! TCP receiver server and wire protocol implementation.

use crate::error::WaftError;
use crate::trust::{TrustStore, TrustTier};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, info, warn};

const CHUNK_SIZE: usize = 2 * 1024 * 1024; // 2MB
const READ_TIMEOUT_SECS: u64 = 10;

/// Helper to convert a 32-byte public key into a hex string fingerprint.
fn fingerprint_from_bytes(public_key_bytes: &[u8; 32]) -> String {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(64);
    for &b in public_key_bytes {
        s.push(HEX_CHARS[usize::from(b >> 4)] as char);
        s.push(HEX_CHARS[usize::from(b & 0x0f)] as char);
    }
    s
}

/// Reads exactly N bytes from the socket with a timeout.
async fn read_exact_with_timeout<const N: usize>(
    socket: &mut TcpStream,
) -> Result<[u8; N], WaftError> {
    let mut buf = [0u8; N];
    tokio::time::timeout(
        std::time::Duration::from_secs(READ_TIMEOUT_SECS),
        socket.read_exact(&mut buf),
    )
    .await
    .map_err(|_| {
        WaftError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "Header read timed out",
        ))
    })?
    .map_err(WaftError::Io)?;
    Ok(buf)
}

/// Reads exactly `len` bytes from the socket with a timeout.
async fn read_exact_vec_with_timeout(
    socket: &mut TcpStream,
    len: usize,
) -> Result<Vec<u8>, WaftError> {
    let mut buf = vec![0u8; len];
    tokio::time::timeout(
        std::time::Duration::from_secs(READ_TIMEOUT_SECS),
        socket.read_exact(&mut buf),
    )
    .await
    .map_err(|_| {
        WaftError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "Metadata payload read timed out",
        ))
    })?
    .map_err(WaftError::Io)?;
    Ok(buf)
}

/// Starts the TCP receiver server listening on the specified address.
///
/// # Errors
/// Returns an error if the TCP listener fails to bind.
pub async fn start_receiver(
    bind_addr: SocketAddr,
    trust_store: Arc<TrustStore>,
    download_dir: PathBuf,
) -> Result<(), WaftError> {
    let listener = TcpListener::bind(bind_addr).await?;
    info!(addr = %bind_addr, "TCP receiver started");

    loop {
        let (socket, peer_ip) = match listener.accept().await {
            Ok(val) => val,
            Err(e) => {
                error!(error = %e, "Failed to accept incoming TCP connection");
                continue;
            }
        };

        let trust = Arc::clone(&trust_store);
        let downloads = download_dir.clone();

        tokio::spawn(async move {
            let _ = socket.set_nodelay(true);
            let socket = match (|| -> Result<TcpStream, WaftError> {
                let std_stream = socket.into_std()?;
                let sys_socket = socket2::Socket::from(std_stream);
                let _ = sys_socket.set_recv_buffer_size(4 * 1024 * 1024);
                let std_stream: std::net::TcpStream = sys_socket.into();
                let stream = TcpStream::from_std(std_stream)?;
                Ok(stream)
            })() {
                Ok(s) => s,
                Err(e) => {
                    error!(peer = %peer_ip, error = %e, "Failed to configure socket buffers");
                    return;
                }
            };
            if let Err(e) = handle_connection(socket, peer_ip, trust, downloads).await {
                error!(peer = %peer_ip, error = %e, "Error handling transfer connection");
            }
        });
    }
}

struct TransferHeader {
    filename: PathBuf,
    file_size: u64,
    expected_hash: [u8; 32],
    fingerprint: String,
    tier: TrustTier,
}

fn parse_fixed_header(header_bytes: &[u8; 64]) -> Result<(usize, u64, [u8; 32]), WaftError> {
    let magic = &header_bytes[0..2];
    if magic != [0xFA, 0x57] {
        return Err(WaftError::InvalidHeader(format!(
            "Invalid magic bytes: expected [0xFA, 0x57], got {magic:?}"
        )));
    }

    let version = header_bytes[2];
    if version != 1 {
        return Err(WaftError::InvalidHeader(format!(
            "Unsupported protocol version: expected 1, got {version}"
        )));
    }

    let name_len = u16::from_be_bytes([header_bytes[4], header_bytes[5]]) as usize;

    let mut file_size_bytes = [0u8; 8];
    file_size_bytes.copy_from_slice(&header_bytes[6..14]);
    let file_size = u64::from_be_bytes(file_size_bytes);

    let mut expected_hash = [0u8; 32];
    expected_hash.copy_from_slice(&header_bytes[14..46]);

    Ok((name_len, file_size, expected_hash))
}

fn sanitize_filename(raw_name: &str) -> Result<PathBuf, WaftError> {
    Path::new(raw_name)
        .file_name()
        .map(PathBuf::from)
        .ok_or_else(|| WaftError::InvalidHeader("Filename cannot be empty or invalid".into()))
}

/// Reads the protocol header, verifies the signature, and validates the trust tier.
async fn read_and_verify_header(
    socket: &mut TcpStream,
    trust_store: &TrustStore,
) -> Result<TransferHeader, WaftError> {
    // 1. Read the 64-byte fixed metadata block
    let header_bytes = read_exact_with_timeout::<64>(socket).await?;

    let (name_len, file_size, expected_hash) = parse_fixed_header(&header_bytes)?;

    // 2. Read variable-length filename string
    let name_bytes = read_exact_vec_with_timeout(socket, name_len).await?;
    let raw_name = String::from_utf8(name_bytes)
        .map_err(|e| WaftError::InvalidHeader(format!("Filename is not valid UTF-8: {e}")))?;

    // Securely resolve filename to prevent path traversal
    let filename = sanitize_filename(&raw_name)?;

    // 3. Read sender's public key (32 bytes)
    let pubkey_bytes = read_exact_with_timeout::<32>(socket).await?;
    let verifying_key = VerifyingKey::from_bytes(&pubkey_bytes)
        .map_err(|e| WaftError::SignatureVerification(e.to_string()))?;

    // 4. Read sender's signature (64 bytes)
    let sig_bytes = read_exact_with_timeout::<64>(socket).await?;
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
        socket.write_all(&[0x00]).await?; // REJECT
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

#[cfg(test)]
mod tests {
    use super::{parse_fixed_header, sanitize_filename};
    use crate::error::WaftError;
    use std::path::PathBuf;

    fn valid_header() -> [u8; 64] {
        let mut header = [0u8; 64];
        header[0..2].copy_from_slice(&[0xFA, 0x57]);
        header[2] = 1;
        header[4..6].copy_from_slice(&12u16.to_be_bytes());
        header[6..14].copy_from_slice(&1024u64.to_be_bytes());
        header[14..46].copy_from_slice(&[0xAB; 32]);
        header
    }

    #[test]
    fn parse_fixed_header_accepts_valid_header() -> Result<(), WaftError> {
        let (name_len, file_size, expected_hash) = parse_fixed_header(&valid_header())?;
        assert_eq!(name_len, 12);
        assert_eq!(file_size, 1024);
        assert_eq!(expected_hash, [0xAB; 32]);
        Ok(())
    }

    #[test]
    fn parse_fixed_header_rejects_invalid_magic() -> Result<(), String> {
        let mut header = valid_header();
        header[0..2].copy_from_slice(&[0x00, 0x00]);

        let Some(error) = parse_fixed_header(&header).err() else {
            return Err("invalid magic must fail".into());
        };
        match error {
            WaftError::InvalidHeader(message) => {
                assert!(message.contains("Invalid magic bytes"));
                Ok(())
            }
            other => Err(format!("unexpected error: {other}")),
        }
    }

    #[test]
    fn parse_fixed_header_rejects_invalid_version() -> Result<(), String> {
        let mut header = valid_header();
        header[2] = 2;

        let Some(error) = parse_fixed_header(&header).err() else {
            return Err("invalid version must fail".into());
        };
        match error {
            WaftError::InvalidHeader(message) => {
                assert!(message.contains("Unsupported protocol version"));
                Ok(())
            }
            other => Err(format!("unexpected error: {other}")),
        }
    }

    #[test]
    fn sanitize_filename_strips_directories() -> Result<(), WaftError> {
        assert_eq!(
            sanitize_filename("nested/file.txt")?,
            PathBuf::from("file.txt")
        );
        assert_eq!(
            sanitize_filename("../escape.txt")?,
            PathBuf::from("escape.txt")
        );
        Ok(())
    }
}

/// Handles a single incoming TCP transfer connection.
async fn handle_connection(
    mut socket: TcpStream,
    peer_ip: SocketAddr,
    trust_store: Arc<TrustStore>,
    download_dir: PathBuf,
) -> Result<(), WaftError> {
    info!(peer = %peer_ip, "Handling incoming file transfer connection");

    let header = read_and_verify_header(&mut socket, &trust_store).await?;

    info!(
        filename = ?header.filename,
        size = header.file_size,
        peer_fingerprint = %header.fingerprint,
        tier = ?header.tier,
        "Accepting file transfer request"
    );

    socket.write_all(&[0x01]).await?; // ACK / ACCEPT

    // Prepare target file path
    let file_path = download_dir.join(&header.filename);
    tokio::fs::create_dir_all(&download_dir).await?;

    let mut file = match tokio::fs::File::create(&file_path).await {
        Ok(f) => f,
        Err(e) => {
            error!(error = %e, "Failed to create target file on disk");
            socket.write_all(&[0x00]).await?;
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
            socket.read(&mut buffer[..to_read]),
        )
        .await
        {
            Ok(Ok(0)) => {
                // Premature EOF
                let _ = tokio::fs::remove_file(&file_path).await;
                return Err(WaftError::Interrupted {
                    bytes_sent: header.file_size - remaining,
                });
            }
            Ok(Ok(n)) => n,
            Ok(Err(e)) => {
                let _ = tokio::fs::remove_file(&file_path).await;
                return Err(WaftError::Io(e));
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
        socket.write_all(&[0x00]).await?; // Hash Mismatch / Failure
        return Err(WaftError::HashMismatch {
            expected: hex::encode(header.expected_hash),
            actual: computed_hash.to_hex().to_string(),
        });
    }

    info!(filename = ?header.filename, "File received and verified successfully");
    socket.write_all(&[0x02]).await?; // DONE / SUCCESS

    Ok(())
}

// Minimal hex module inline helper to avoid external dependencies
mod hex {
    pub fn encode(bytes: [u8; 32]) -> String {
        const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(64);
        for &b in &bytes {
            s.push(HEX_CHARS[usize::from(b >> 4)] as char);
            s.push(HEX_CHARS[usize::from(b & 0x0f)] as char);
        }
        s
    }
}
