#![allow(unsafe_code)]

use crate::error::WaftError;
use crate::identity::Identity;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tracing::info;

const CHUNK_SIZE: usize = 2 * 1024 * 1024; // 2MB

/// Sends a file to a peer over QUIC.
pub async fn send_file_quic(
    peer_addr: SocketAddr,
    identity: &Identity,
    file_path: &Path,
    progress_tx: Option<tokio::sync::mpsc::UnboundedSender<(u64, u64)>>,
) -> Result<(), WaftError> {
    info!(path = ?file_path, peer = %peer_addr, "Preparing to send file over QUIC");

    // 1. Open file and get metadata
    let file = std::fs::File::open(file_path)?;
    let metadata = file.metadata()?;
    let file_size = metadata.len();

    // 2. Memory-map the file (if size > 0)
    let mmap = if file_size > 0 {
        let m = unsafe { memmap2::Mmap::map(&file)? };
        Some(Arc::new(m))
    } else {
        None
    };

    // 3. Spawn background hashing task
    let mmap_clone = mmap.clone();
    let hash_task = tokio::task::spawn_blocking(move || {
        mmap_clone
            .as_ref()
            .map_or_else(|| blake3::hash(&[]), |m| blake3::hash(m))
    });

    // 4. Bind client UDP endpoint and connect
    let mut endpoint = quinn::Endpoint::client(SocketAddr::new(
        std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
        0,
    ))?;
    endpoint.set_default_client_config(
        crate::quic_transport::make_client_config()
            .map_err(|e| WaftError::Io(std::io::Error::other(e.to_string())))?,
    );

    let connecting = endpoint
        .connect(peer_addr, "localhost")
        .map_err(|e| WaftError::Io(std::io::Error::other(e.to_string())))?;

    // 5. Await both hashing and connection tasks concurrently
    let (blake3_hash, connection) = tokio::try_join!(
        async {
            hash_task.await.map_err(|e| {
                WaftError::Io(std::io::Error::other(format!("hashing task panicked: {e}")))
            })
        },
        async {
            connecting.await.map_err(|e| {
                WaftError::Io(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    format!("QUIC connection failed: {e}"),
                ))
            })
        }
    )?;

    let hash_bytes = blake3_hash.as_bytes();
    info!(peer = %peer_addr, "QUIC connection established");

    // 6. Open a bidirectional stream
    let (mut send_stream, mut recv_stream) = connection.open_bi().await.map_err(|e| {
        WaftError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            format!("failed to open bidirectional QUIC stream: {e}"),
        ))
    })?;

    // 7. Extract and validate filename
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

    // 8. Construct the 64-byte fixed metadata block
    let mut header_bytes = [0u8; 64];
    header_bytes[0..2].copy_from_slice(&[0xFA, 0x57]); // magic
    header_bytes[2] = 1; // version
    header_bytes[3] = 0; // flags (QUIC)
    let name_len_u16 = u16::try_from(name_len)
        .map_err(|_| WaftError::InvalidHeader("Filename too long".into()))?;
    header_bytes[4..6].copy_from_slice(&name_len_u16.to_be_bytes()); // name_len
    header_bytes[6..14].copy_from_slice(&file_size.to_be_bytes()); // file_size
    header_bytes[14..46].copy_from_slice(hash_bytes); // blake3

    // 9. Generate signature over: 64-byte header + raw name bytes
    let mut signed_message = Vec::with_capacity(64 + name_len);
    signed_message.extend_from_slice(&header_bytes);
    signed_message.extend_from_slice(name_bytes);

    let signature = identity.sign(&signed_message);
    let sig_bytes = signature.to_bytes();

    // 10. Write protocol payload
    send_stream
        .write_all(&header_bytes)
        .await
        .map_err(to_io_err)?;
    send_stream.write_all(name_bytes).await.map_err(to_io_err)?;
    send_stream
        .write_all(&identity.public_key().to_bytes())
        .await
        .map_err(to_io_err)?;
    send_stream.write_all(&sig_bytes).await.map_err(to_io_err)?;
    send_stream.flush().await.map_err(to_io_err)?;

    // 11. Read 1-byte ACK
    let mut ack = [0u8; 1];
    recv_stream.read_exact(&mut ack).await.map_err(to_io_err)?;

    if ack[0] == 0x00 {
        tracing::warn!(peer = %peer_addr, "Transfer rejected by receiver over QUIC");
        return Err(WaftError::Rejected);
    } else if ack[0] != 0x01 {
        return Err(WaftError::InvalidHeader(format!(
            "Invalid ACK byte received: expected 0x01, got {:#04X}",
            ack[0]
        )));
    }

    info!(peer = %peer_addr, "Transfer accepted. Streaming file contents over QUIC...");

    // 12. Stream file body
    let mut bytes_sent = 0u64;
    if let Some(m) = mmap {
        while bytes_sent < file_size {
            let remaining = file_size - bytes_sent;
            let to_send = std::cmp::min(remaining, CHUNK_SIZE as u64) as usize;
            let chunk = &m[bytes_sent as usize..(bytes_sent as usize + to_send)];

            send_stream.write_all(chunk).await.map_err(to_io_err)?;
            bytes_sent += to_send as u64;
            if let Some(ref tx) = progress_tx {
                let _ = tx.send((bytes_sent, file_size));
            }
        }
    } else if let Some(ref tx) = progress_tx {
        let _ = tx.send((0, 0));
    }

    send_stream.flush().await.map_err(to_io_err)?;
    send_stream.finish().map_err(|e| {
        WaftError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            format!("failed to finish QUIC stream: {e:?}"),
        ))
    })?;

    // 13. Read final 1-byte status verification
    let mut status = [0u8; 1];
    recv_stream
        .read_exact(&mut status)
        .await
        .map_err(to_io_err)?;

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

    info!(peer = %peer_addr, "File sent and verified successfully over QUIC");
    Ok(())
}

fn to_io_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> std::io::Error {
    std::io::Error::other(e)
}
