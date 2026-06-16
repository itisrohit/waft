use anyhow::{Context, Result};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ClientConfig, ServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use std::sync::Arc;

/// Initializes the process-wide default `CryptoProvider` for `rustls`.
/// Since `rustls` 0.23, an explicit `CryptoProvider` must be set to prevent runtime panics.
pub fn init_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

/// A custom `ServerCertVerifier` that bypasses standard TLS certificate and hostname verification.
/// Necessary because waft operates peer-to-peer without standard Certificate Authorities.
#[derive(Debug)]
pub struct SkipServerVerification;

impl ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

/// Generates a self-signed X.509 certificate and key pair on the fly.
pub fn generate_self_signed_cert() -> Result<(
    CertificateDer<'static>,
    rustls::pki_types::PrivateKeyDer<'static>,
)> {
    let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let certified_key = rcgen::generate_simple_self_signed(subject_alt_names)
        .context("failed to generate self-signed certificate")?;

    let cert_der = CertificateDer::from(certified_key.cert.der().to_vec());
    let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
        rustls::pki_types::PrivatePkcs8KeyDer::from(certified_key.key_pair.serialize_der()),
    );

    Ok((cert_der, key_der))
}

/// Builds the Quinn `ServerConfig` using a dynamically generated self-signed certificate.
pub fn make_server_config() -> Result<ServerConfig> {
    let (cert, key) = generate_self_signed_cert()?;
    let certs = vec![cert];

    let mut tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("failed to configure rustls server certificate")?;

    // Set ALPN protocol to ensure both client/server negotiate the same protocol
    tls_config.alpn_protocols = vec![b"waft/1".to_vec()];

    let quic_server_config =
        QuicServerConfig::try_from(tls_config).context("failed to convert to QuicServerConfig")?;

    Ok(ServerConfig::with_crypto(Arc::new(quic_server_config)))
}

/// Builds the Quinn `ClientConfig` that bypasses server verification.
pub fn make_client_config() -> Result<ClientConfig> {
    let mut tls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth();

    // Set ALPN protocol matching the server
    tls_config.alpn_protocols = vec![b"waft/1".to_vec()];

    let quic_client_config =
        QuicClientConfig::try_from(tls_config).context("failed to convert to QuicClientConfig")?;

    Ok(ClientConfig::new(Arc::new(quic_client_config)))
}
