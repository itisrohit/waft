//! Cryptographic identity management
//! This module handles generating, saving, loading, and identifying the local peer
//! using Ed25519 public-key cryptography.

use anyhow::{Context, Result};
use ed25519_dalek::{SECRET_KEY_LENGTH, SigningKey, VerifyingKey};
use rand_core::OsRng;
use std::fs;
use std::path::Path;

/// The cryptographic identity of the local peer.
pub struct Identity {
    signing_key: SigningKey,
}

impl Identity {
    /// Generates a new cryptographically secure random Ed25519 identity.
    #[must_use]
    pub fn generate() -> Self {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        Self { signing_key }
    }

    /// Saves the private key to the specified file path.
    ///
    /// # Errors
    /// Returns an error if the parent directories cannot be created, writing the
    /// file fails, or (on Unix) permissions cannot be set to `0600`.
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create parent directory {}", parent.display())
            })?;
        }
        let key_bytes = self.signing_key.to_bytes();
        fs::write(path, key_bytes)
            .with_context(|| format!("failed to write identity file at {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).with_context(|| {
                format!(
                    "Failed to restrict permissions (0600) on {}",
                    path.display()
                )
            })?;
        }
        Ok(())
    }

    /// Loads the identity from the specified file path.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read, or if the file contents
    /// do not match the expected 32-byte secret key size.
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = fs::read(path)
            .with_context(|| format!("failed to read identity file at {}", path.display()))?;
        if bytes.len() != SECRET_KEY_LENGTH {
            anyhow::bail!(
                "invalid identity file size: expected {} bytes, got {}",
                SECRET_KEY_LENGTH,
                bytes.len()
            );
        }
        let mut key_bytes = [0u8; SECRET_KEY_LENGTH];
        key_bytes.copy_from_slice(&bytes);
        let signing_key = SigningKey::from_bytes(&key_bytes);
        Ok(Self { signing_key })
    }

    /// Loads the identity from the given path if it exists, or generates and
    /// saves a new identity to the path.
    ///
    /// # Errors
    /// Returns an error if loading the existing file fails, or if generating
    /// and saving the new file fails.
    pub fn load_or_generate(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.exists() {
            Self::load_from_file(path)
        } else {
            let identity = Self::generate();
            identity.save_to_file(path)?;
            Ok(identity)
        }
    }

    /// Returns the public key (`VerifyingKey`) associated with this identity.
    #[must_use]
    pub fn public_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Returns a unique hex-encoded representation of the public key.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
        let public_key_bytes = self.public_key().to_bytes();
        let mut s = String::with_capacity(public_key_bytes.len() * 2);
        for &b in &public_key_bytes {
            s.push(HEX_CHARS[usize::from(b >> 4)] as char);
            s.push(HEX_CHARS[usize::from(b & 0x0f)] as char);
        }
        s
    }

    /// Signs a message using the private signing key.
    #[must_use]
    pub fn sign(&self, message: &[u8]) -> ed25519_dalek::Signature {
        use ed25519_dalek::Signer;
        self.signing_key.sign(message)
    }
}
