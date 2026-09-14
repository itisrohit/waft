//! Minimal iroh transport spike.
//!
//! This intentionally does not replace the LAN transport yet. It validates
//! public-key addressing, direct QUIC connectivity, and iroh relay fallback
//! before we migrate the production daemon.

#![cfg(feature = "iroh-spike")]

use anyhow::{Context, Result, anyhow};
use iroh::{Endpoint, EndpointAddr, endpoint::presets};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const ALPN: &[u8] = b"waft/iroh-file-v1";
const MAX_FILE_NAME_BYTES: usize = 255;

fn validate_file_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > MAX_FILE_NAME_BYTES {
        anyhow::bail!("file name is empty or too long");
    }
    if name.contains(['/', '\\']) || name == "." || name == ".." {
        anyhow::bail!("invalid remote file name");
    }
    Ok(())
}

/// Starts an endpoint and prints its endpoint address for a second machine.
pub async fn bind() -> Result<Endpoint> {
    let endpoint = Endpoint::bind(presets::N0).await?;
    endpoint.online().await;
    println!(
        "WAFT_IROH_ENDPOINT={}",
        serde_json::to_string(&endpoint.addr())?
    );
    Ok(endpoint)
}

/// Sends a file using a bidirectional QUIC stream.
pub async fn send(endpoint: &Endpoint, peer: EndpointAddr, path: &std::path::Path) -> Result<()> {
    let connection = endpoint
        .connect(peer, ALPN)
        .await
        .context("connect iroh peer")?;
    let (mut tx, mut rx) = connection.open_bi().await?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("invalid file name"))?;
    validate_file_name(name)?;
    let metadata = tokio::fs::metadata(path).await?;
    tx.write_u16(name.len() as u16).await?;
    tx.write_u64(metadata.len()).await?;
    tx.write_all(name.as_bytes()).await?;
    let mut file = tokio::fs::File::open(path).await?;
    tokio::io::copy(&mut file, &mut tx).await?;
    tx.finish()?;
    let status = rx.read_u8().await?;
    if status != 1 {
        anyhow::bail!("iroh peer rejected file");
    }
    connection.close(0u32.into(), b"done");
    Ok(())
}

/// Accepts one file stream and writes it to `downloads`.
pub async fn receive(endpoint: &Endpoint, downloads: &std::path::Path) -> Result<()> {
    let incoming = endpoint.accept().await.context("accept iroh connection")?;
    let connection = incoming.await.context("complete iroh connection")?;
    let (mut tx, mut rx) = connection.accept_bi().await?;
    let name_len = usize::from(rx.read_u16().await?);
    let size = rx.read_u64().await?;
    if name_len == 0 || name_len > MAX_FILE_NAME_BYTES {
        anyhow::bail!("invalid remote file name length");
    }
    let mut name = vec![0_u8; name_len];
    rx.read_exact(&mut name).await?;
    let name = String::from_utf8(name).context("remote file name is not UTF-8")?;
    validate_file_name(&name)?;
    tokio::fs::create_dir_all(downloads).await?;
    let target = downloads.join(name);
    let mut file = tokio::fs::File::create(target).await?;
    let copied = tokio::io::copy(&mut rx.take(size), &mut file).await?;
    file.flush().await?;
    tx.write_u8(u8::from(copied == size)).await?;
    tx.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_file_name;

    #[test]
    fn accepts_plain_file_names() {
        assert!(validate_file_name("photo.jpg").is_ok());
    }

    #[test]
    fn rejects_path_traversal_names() {
        assert!(validate_file_name("../photo.jpg").is_err());
        assert!(validate_file_name("folder/photo.jpg").is_err());
        assert!(validate_file_name("..").is_err());
    }
}
