#![allow(unsafe_code)]

//! TCP client and file sending implementation.

use crate::error::WaftError;
use crate::identity::Identity;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{info, warn};

const CHUNK_SIZE: usize = 2 * 1024 * 1024; // 2MB

#[cfg(unix)]
#[allow(clippy::cast_possible_wrap)]
async fn send_file_zero_copy(
    socket: &TcpStream,
    file: &std::fs::File,
    file_size: u64,
    offset: u64,
    chunk_size: usize,
    progress_tx: Option<&tokio::sync::mpsc::UnboundedSender<(u64, u64)>>,
) -> Result<(), WaftError> {
    use std::os::unix::io::AsRawFd;

    let out_fd = socket.as_raw_fd();
    let in_fd = file.as_raw_fd();
    let mut bytes_sent = 0u64;
    let bytes_to_send = file_size - offset;

    while bytes_sent < bytes_to_send {
        let remaining = bytes_to_send - bytes_sent;
        let to_send = std::cmp::min(remaining, chunk_size as u64) as usize;

        socket.writable().await?;

        let ret = socket.try_io(tokio::io::Interest::WRITABLE, || {
            #[cfg(target_os = "macos")]
            {
                let mut len = to_send as libc::off_t;
                let res = unsafe {
                    libc::sendfile(
                        in_fd,
                        out_fd,
                        (offset + bytes_sent) as libc::off_t,
                        &raw mut len,
                        std::ptr::null_mut(),
                        0,
                    )
                };
                if res == 0 {
                    Ok(len as usize)
                } else {
                    let err = std::io::Error::last_os_error();
                    let errno = err.raw_os_error();
                    if errno == Some(libc::EAGAIN) || errno == Some(libc::EINTR) {
                        if len > 0 { Ok(len as usize) } else { Err(err) }
                    } else {
                        Err(err)
                    }
                }
            }

            #[cfg(not(target_os = "macos"))]
            {
                let mut file_offset = (offset + bytes_sent) as libc::off_t;
                let res = unsafe { libc::sendfile(out_fd, in_fd, &raw mut file_offset, to_send) };
                if res >= 0 {
                    Ok(res as usize)
                } else {
                    Err(std::io::Error::last_os_error())
                }
            }
        });

        match ret {
            Ok(n) => {
                if n == 0 {
                    return Err(WaftError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "sendfile sent 0 bytes",
                    )));
                }
                bytes_sent += n as u64;
                if let Some(tx) = progress_tx {
                    let _ = tx.send((offset + bytes_sent, file_size));
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => {
                return Err(WaftError::Interrupted {
                    bytes_sent: offset + bytes_sent,
                });
            }
        }
    }

    Ok(())
}

#[cfg(not(unix))]
async fn send_file_fallback(
    socket: &mut TcpStream,
    mmap: Option<&[u8]>,
    file_size: u64,
    offset: u64,
    chunk_size: usize,
    progress_tx: Option<&tokio::sync::mpsc::UnboundedSender<(u64, u64)>>,
) -> Result<(), WaftError> {
    let mut bytes_sent = 0u64;
    let bytes_to_send = file_size - offset;

    if let Some(m) = mmap {
        while bytes_sent < bytes_to_send {
            let remaining = bytes_to_send - bytes_sent;
            let to_send = std::cmp::min(remaining, chunk_size as u64) as usize;
            let start = (offset + bytes_sent) as usize;
            let chunk = &m[start..start + to_send];

            socket.write_all(chunk).await?;
            bytes_sent += to_send as u64;
            if let Some(tx) = progress_tx {
                let _ = tx.send((offset + bytes_sent, file_size));
            }
        }
    } else if let Some(tx) = progress_tx {
        let _ = tx.send((0, 0));
    }

    Ok(())
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

    // 4. Start TCP connection concurrently
    let connect_task = TcpStream::connect(peer_addr);

    // 5. Await both hashing and connection tasks concurrently
    let (blake3_hash, socket) = tokio::try_join!(
        async {
            hash_task.await.map_err(|e| {
                WaftError::Io(std::io::Error::other(format!("hashing task panicked: {e}")))
            })
        },
        async { connect_task.await.map_err(WaftError::Io) }
    )?;

    let hash_bytes = blake3_hash.as_bytes();

    // 6. Extract and validate filename
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

    // 7. Tune socket buffers
    let _ = socket.set_nodelay(true);
    let std_stream = socket.into_std()?;
    let sys_socket = socket2::Socket::from(std_stream);
    let _ = sys_socket.set_send_buffer_size(4 * 1024 * 1024);
    let chunk_size = sys_socket
        .send_buffer_size()
        .unwrap_or(CHUNK_SIZE)
        .clamp(64 * 1024, CHUNK_SIZE);
    let std_stream: std::net::TcpStream = sys_socket.into();
    let mut socket = TcpStream::from_std(std_stream)?;
    info!(peer = %peer_addr, "Connected to receiver");

    // 8. Construct the 64-byte fixed metadata block
    let mut header_bytes = [0u8; 64];
    header_bytes[0..2].copy_from_slice(&[0xFA, 0x57]); // magic
    header_bytes[2] = 1; // version
    header_bytes[3] = 0; // flags
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
    socket.write_all(&header_bytes).await?;
    socket.write_all(name_bytes).await?;
    socket.write_all(&identity.public_key().to_bytes()).await?;
    socket.write_all(&sig_bytes).await?;
    socket.flush().await?;

    // 11. Read 1-byte ACK
    let mut ack = [0u8; 1];
    socket.read_exact(&mut ack).await?;

    let mut offset = 0u64;
    if ack[0] == 0x00 {
        warn!(peer = %peer_addr, "Transfer rejected by receiver");
        return Err(WaftError::Rejected);
    } else if ack[0] == 0x02 {
        info!(peer = %peer_addr, "File already exists on receiver and matches hash. Skipped transfer.");
        if let Some(ref tx) = progress_tx {
            let _ = tx.send((file_size, file_size));
        }
        return Ok(());
    } else if ack[0] == 0x03 {
        // Read 8-byte offset
        let mut offset_bytes = [0u8; 8];
        socket.read_exact(&mut offset_bytes).await?;
        offset = u64::from_be_bytes(offset_bytes);
        info!(peer = %peer_addr, offset = offset, "Resuming file transfer from offset");
    } else if ack[0] != 0x01 {
        return Err(WaftError::InvalidHeader(format!(
            "Invalid ACK byte received: expected 0x01, 0x02, or 0x03, got {:#04X}",
            ack[0]
        )));
    }

    info!(peer = %peer_addr, "Transfer accepted. Streaming file contents...");

    // 12. Stream file body
    #[cfg(unix)]
    {
        send_file_zero_copy(
            &socket,
            &file,
            file_size,
            offset,
            chunk_size,
            progress_tx.as_ref(),
        )
        .await?;
    }
    #[cfg(not(unix))]
    {
        send_file_fallback(
            &mut socket,
            mmap.as_deref().map(|m| &**m),
            file_size,
            offset,
            chunk_size,
            progress_tx.as_ref(),
        )
        .await?;
    }

    socket.flush().await?;

    // 13. Read final 1-byte status verification
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
