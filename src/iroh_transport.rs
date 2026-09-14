//! Minimal iroh transport spike.
//!
//! This intentionally does not replace the LAN transport yet. It validates
//! public-key addressing, direct QUIC connectivity, and iroh relay fallback
//! before we migrate the production daemon.

#![cfg(any(feature = "iroh-spike", feature = "iroh-internet"))]

use anyhow::{Context, Result, anyhow};
use iroh::{Endpoint, EndpointAddr, endpoint::presets};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(feature = "iroh-internet")]
use {
    crate::identity::Identity,
    crate::trust::{TrustStore, TrustTier},
    ed25519_dalek::{Signature, Verifier, VerifyingKey},
    serde::{Deserialize, Serialize},
    std::path::{Path, PathBuf},
    std::sync::Arc,
};

const ALPN: &[u8] = b"waft/iroh-file-v1";
const MAX_FILE_NAME_BYTES: usize = 255;
#[cfg(feature = "iroh-internet")]
const MAX_HEADER_BYTES: usize = 64 * 1024;

#[cfg(feature = "iroh-internet")]
pub type SharedEndpoint = Arc<Endpoint>;

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
    let endpoint = bind_endpoint().await?;
    println!(
        "WAFT_IROH_ENDPOINT={}",
        serde_json::to_string(&endpoint.addr())?
    );
    Ok(endpoint)
}

/// Starts an endpoint for daemon integrations without producing CLI output.
pub async fn bind_endpoint() -> Result<Endpoint> {
    let endpoint = Endpoint::builder(presets::N0)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await?;
    endpoint.online().await;
    Ok(endpoint)
}

/// Serializes an endpoint address for rendezvous announcements.
pub fn endpoint_address_json(endpoint: &Endpoint) -> Result<String> {
    Ok(serde_json::to_string(&endpoint.addr())?)
}

#[cfg(feature = "iroh-internet")]
#[derive(Debug, Deserialize, Serialize)]
struct AuthenticatedHeader {
    name: String,
    size: u64,
    hash: String,
    fingerprint: String,
    public_key: String,
    signature: String,
}

#[cfg(feature = "iroh-internet")]
fn signed_header_bytes(header: &AuthenticatedHeader) -> Vec<u8> {
    format!(
        "waft-iroh-v1\n{}\n{}\n{}\n{}",
        header.name, header.size, header.hash, header.fingerprint
    )
    .into_bytes()
}

#[cfg(feature = "iroh-internet")]
async fn hash_file(path: &Path) -> Result<[u8; 32]> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 2 * 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(*hasher.finalize().as_bytes())
}

#[cfg(feature = "iroh-internet")]
fn decode_fixed<const N: usize>(value: &str, label: &str) -> Result<[u8; N]> {
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, value)
        .with_context(|| format!("invalid {label}"))?;
    bytes
        .try_into()
        .map_err(|_| anyhow!("invalid {label} length"))
}

#[cfg(feature = "iroh-internet")]
fn verify_header(header: &AuthenticatedHeader) -> Result<()> {
    validate_file_name(&header.name)?;
    if header.fingerprint.len() != 64 || header.hash.len() != 64 {
        anyhow::bail!("invalid file identity or hash");
    }
    let public_key = decode_fixed::<32>(&header.public_key, "public key")?;
    let key = VerifyingKey::from_bytes(&public_key)?;
    if crate::transfer::fingerprint_from_bytes(&public_key) != header.fingerprint {
        anyhow::bail!("remote fingerprint mismatch");
    }
    let signature = Signature::from_bytes(&decode_fixed::<64>(&header.signature, "signature")?);
    key.verify(&signed_header_bytes(header), &signature)?;
    Ok(())
}

/// Sends an authenticated file over the daemon's shared iroh endpoint.
#[cfg(feature = "iroh-internet")]
pub async fn send_authenticated(
    endpoint: &Endpoint,
    peer: EndpointAddr,
    path: &Path,
    identity: &Identity,
    progress: Option<tokio::sync::mpsc::UnboundedSender<(u64, u64)>>,
) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("invalid file name"))?;
    validate_file_name(name)?;
    let size = tokio::fs::metadata(path).await?.len();
    let hash = hash_file(path).await?;
    let header = AuthenticatedHeader {
        name: name.to_string(),
        size,
        hash: blake3::Hash::from_bytes(hash).to_hex().to_string(),
        fingerprint: identity.fingerprint(),
        public_key: base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            identity.public_key().to_bytes(),
        ),
        signature: String::new(),
    };
    let signature = identity.sign(&signed_header_bytes(&header));
    let header = AuthenticatedHeader {
        signature: base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            signature.to_bytes(),
        ),
        ..header
    };
    let encoded = serde_json::to_vec(&header)?;
    if encoded.len() > MAX_HEADER_BYTES {
        anyhow::bail!("iroh file header is too large");
    }

    let connection = endpoint
        .connect(peer, ALPN)
        .await
        .context("connect iroh peer")?;
    let (mut tx, mut rx) = connection.open_bi().await?;
    tx.write_u32(u32::try_from(encoded.len())?).await?;
    tx.write_all(&encoded).await?;
    let mut file = tokio::fs::File::open(path).await?;
    let mut buffer = vec![0_u8; 2 * 1024 * 1024];
    let mut sent = 0_u64;
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        tx.write_all(&buffer[..read]).await?;
        sent += read as u64;
        if let Some(progress) = &progress {
            let _ = progress.send((sent, size));
        }
    }
    tx.finish()?;
    if rx.read_u8().await? != 1 {
        anyhow::bail!("iroh peer rejected file");
    }
    connection.close(0u32.into(), b"done");
    Ok(())
}

#[cfg(feature = "iroh-internet")]
async fn receive_authenticated_connection(
    incoming: iroh::endpoint::Incoming,
    downloads: PathBuf,
    trust: Arc<TrustStore>,
) -> Result<()> {
    let accepting = incoming.accept()?;
    let connection = accepting.await?;
    let (mut tx, mut rx) = connection.accept_bi().await?;
    let header_len = usize::try_from(rx.read_u32().await?)?;
    if header_len == 0 || header_len > MAX_HEADER_BYTES {
        anyhow::bail!("invalid iroh file header length");
    }
    let mut encoded = vec![0_u8; header_len];
    rx.read_exact(&mut encoded).await?;
    let header: AuthenticatedHeader = serde_json::from_slice(&encoded)?;
    verify_header(&header)?;
    if trust.get_tier(&header.fingerprint) == TrustTier::Blocked {
        anyhow::bail!("remote peer is blocked");
    }
    tokio::fs::create_dir_all(&downloads).await?;
    let temp = downloads.join(format!("{}.part", header.hash));
    let final_path = downloads.join(&header.name);
    let mut output = tokio::fs::File::create(&temp).await?;
    let mut hasher = blake3::Hasher::new();
    let mut received = 0_u64;
    let mut buffer = vec![0_u8; 2 * 1024 * 1024];
    while received < header.size {
        let wanted = usize::try_from((header.size - received).min(buffer.len() as u64))?;
        let read = rx.read(&mut buffer[..wanted]).await?;
        let Some(read) = read else {
            anyhow::bail!("iroh file stream ended early");
        };
        output.write_all(&buffer[..read]).await?;
        hasher.update(&buffer[..read]);
        received += read as u64;
    }
    output.flush().await?;
    if hasher.finalize().to_hex().as_str() != header.hash {
        anyhow::bail!("iroh file hash mismatch");
    }
    tokio::fs::rename(temp, final_path).await?;
    tx.write_u8(1).await?;
    tx.finish()?;
    Ok(())
}

/// Accepts authenticated daemon transfers until the endpoint is closed.
#[cfg(feature = "iroh-internet")]
pub async fn receive_authenticated_loop(
    endpoint: SharedEndpoint,
    downloads: PathBuf,
    trust: Arc<TrustStore>,
) -> Result<()> {
    while let Some(incoming) = endpoint.accept().await {
        let downloads = downloads.clone();
        let trust = Arc::clone(&trust);
        tokio::spawn(async move {
            if let Err(error) = receive_authenticated_connection(incoming, downloads, trust).await {
                tracing::warn!(%error, "iroh file receive failed");
            }
        });
    }
    Ok(())
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
