//! TCP client and file sending implementation.

use crate::error::WaftError;
use crate::identity::Identity;
use std::net::SocketAddr;
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{info, warn};

const HASH_BUFFER_SIZE: usize = 64 * 1024; // 64KB read buffer
const CHUNK_SIZE: usize = 2 * 1024 * 1024; // 2MB

/// Computes the BLAKE3 hash of a file at the specified path.
async fn compute_blake3_hash(path: &Path) -> Result<(blake3::Hash, u64), WaftError> {
    let mut file = File::open(path).await?;
    let metadata = file.metadata().await?;
    let file_size = metadata.len();

    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; HASH_BUFFER_SIZE];

    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    Ok((hasher.finalize(), file_size))
}

/// Sends a file to a peer over TCP using the custom wire protocol.
///
/// # Errors
/// Returns an error if the file cannot be opened/hashed, the connection fails,
/// the transfer is rejected by the peer, or the transfer is interrupted.
pub async fn send_file(
    peer_addr: SocketAddr,
    identity: &Identity,
    file_path: &Path,
    progress_tx: Option<tokio::sync::mpsc::UnboundedSender<(u64, u64)>>,
) -> Result<(), WaftError> {
    info!(path = ?file_path, peer = %peer_addr, "Preparing to send file");

    // 1. Compute the BLAKE3 hash and get the file size
    let (blake3_hash, file_size) = compute_blake3_hash(file_path).await?;
    let hash_bytes = blake3_hash.as_bytes();

    // 2. Extract and validate filename
    let raw_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| {
            WaftError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid file path or filename",
            ))
        })?;
    let name_bytes = raw_name.as_bytes();
    let name_len = name_bytes.len();

    if name_len > u16::MAX as usize {
        return Err(WaftError::InvalidHeader(
            "Filename exceeds maximum supported length (65535 bytes)".into(),
        ));
    }

    // 3. Connect to the receiver
    let socket = TcpStream::connect(peer_addr).await?;
    let _ = socket.set_nodelay(true);
    let std_stream = socket.into_std()?;
    let sys_socket = socket2::Socket::from(std_stream);
    let _ = sys_socket.set_send_buffer_size(4 * 1024 * 1024);
    let std_stream: std::net::TcpStream = sys_socket.into();
    let mut socket = TcpStream::from_std(std_stream)?;
    info!(peer = %peer_addr, "Connected to receiver");

    // 4. Construct the 64-byte fixed metadata block
    let mut header_bytes = [0u8; 64];
    header_bytes[0..2].copy_from_slice(&[0xFA, 0x57]); // magic
    header_bytes[2] = 1; // version
    header_bytes[3] = 0; // flags
    let name_len_u16 = u16::try_from(name_len)
        .map_err(|_| WaftError::InvalidHeader("Filename too long".into()))?;
    header_bytes[4..6].copy_from_slice(&name_len_u16.to_be_bytes()); // name_len
    header_bytes[6..14].copy_from_slice(&file_size.to_be_bytes()); // file_size
    header_bytes[14..46].copy_from_slice(hash_bytes); // blake3

    // 5. Generate signature over: 64-byte header + raw name bytes
    let mut signed_message = Vec::with_capacity(64 + name_len);
    signed_message.extend_from_slice(&header_bytes);
    signed_message.extend_from_slice(name_bytes);

    let signature = identity.sign(&signed_message);
    let sig_bytes = signature.to_bytes();

    // 6. Write protocol payload:
    // - 64-byte header
    // - name_bytes
    // - public_key (32 bytes)
    // - signature (64 bytes)
    socket.write_all(&header_bytes).await?;
    socket.write_all(name_bytes).await?;
    socket.write_all(&identity.public_key().to_bytes()).await?;
    socket.write_all(&sig_bytes).await?;
    socket.flush().await?;

    // 7. Read 1-byte ACK
    let mut ack = [0u8; 1];
    socket.read_exact(&mut ack).await?;

    if ack[0] == 0x00 {
        warn!(peer = %peer_addr, "Transfer rejected by receiver");
        return Err(WaftError::Rejected);
    } else if ack[0] != 0x01 {
        return Err(WaftError::InvalidHeader(format!(
            "Invalid ACK byte received: expected 0x01, got {:#04X}",
            ack[0]
        )));
    }

    info!(peer = %peer_addr, "Transfer accepted. Streaming file contents...");

    // 8. Stream file body in 2MB chunks
    let mut file = File::open(file_path).await?;
    let mut buffer = vec![0u8; CHUNK_SIZE];
    let mut bytes_sent = 0u64;

    loop {
        let n = match file.read(&mut buffer).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => return Err(WaftError::Io(e)),
        };

        if socket.write_all(&buffer[..n]).await.is_err() {
            return Err(WaftError::Interrupted { bytes_sent });
        }

        bytes_sent += n as u64;
        if let Some(ref tx) = progress_tx {
            let _ = tx.send((bytes_sent, file_size));
        }
    }
    socket.flush().await?;

    // 9. Read final 1-byte status verification
    let mut status = [0u8; 1];
    socket.read_exact(&mut status).await?;

    if status[0] == 0x00 {
        return Err(WaftError::HashMismatch {
            expected: blake3_hash.to_hex().to_string(),
            actual: "mismatched hash at receiver".into(),
        });
    } else if status[0] != 0x02 {
        return Err(WaftError::InvalidHeader(format!(
            "Invalid final status: expected 0x02, got {:#04X}",
            status[0]
        )));
    }

    info!(peer = %peer_addr, "File sent and verified successfully");
    Ok(())
}
