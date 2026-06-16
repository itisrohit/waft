//! Peer trust configuration and trust.toml database.

use crate::error::WaftError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// The trust level associated with a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TrustTier {
    /// Silent rejection, no notification.
    Blocked = 0,
    /// Prompt the user to accept/reject (default for new peers).
    #[default]
    Ask = 1,
    /// Automatically accept transfers.
    Trusted = 2,
    /// Automatically accept transfers and open the received file.
    Own = 3,
}

/// Inner wrapper structure for serializing and deserializing the database file.
#[derive(Debug, Serialize, Deserialize, Default)]
struct TrustData {
    #[serde(default)]
    peers: HashMap<String, TrustTier>,
}

/// Thread-safe database managing peer trust tier configurations.
#[derive(Debug)]
pub struct TrustStore {
    path: PathBuf,
    data: RwLock<TrustData>,
}

impl TrustStore {
    /// Creates a new `TrustStore` bound to the given file path.
    ///
    /// It loads the data if the file exists, or initializes an empty store.
    ///
    /// # Errors
    /// Returns a `WaftError` if the file exists but is malformed,
    /// or if parent directories and the initial file cannot be created.
    pub fn load_or_create(path: impl AsRef<Path>) -> Result<Self, WaftError> {
        let path = path.as_ref().to_path_buf();
        let data = if path.exists() {
            let content = fs::read_to_string(&path)?;
            toml::from_str(&content)?
        } else {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let default_data = TrustData::default();
            let content = toml::to_string(&default_data)?;
            fs::write(&path, content)?;
            default_data
        };

        Ok(Self {
            path,
            data: RwLock::new(data),
        })
    }

    /// Returns the trust tier of a peer by its fingerprint.
    ///
    /// Defaults to `TrustTier::Ask` for unknown peers.
    pub fn get_tier(&self, fingerprint: &str) -> TrustTier {
        let read_guard = self
            .data
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        read_guard
            .peers
            .get(fingerprint)
            .copied()
            .unwrap_or(TrustTier::Ask)
    }

    /// Returns a list of all configured peer trust tiers.
    pub fn get_all(&self) -> Vec<(String, TrustTier)> {
        let read_guard = self
            .data
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        read_guard
            .peers
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }

    /// Sets the trust tier for a peer and persists the change to disk.
    ///
    /// # Errors
    /// Returns a `WaftError` if the updated store cannot be persisted to disk.
    pub fn set_tier(&self, fingerprint: &str, tier: TrustTier) -> Result<(), WaftError> {
        {
            let mut write_guard = self
                .data
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            write_guard.peers.insert(fingerprint.to_string(), tier);
        }
        self.save()?;
        Ok(())
    }

    /// Persists the current trust data to disk.
    fn save(&self) -> Result<(), WaftError> {
        let content = {
            let read_guard = self
                .data
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            toml::to_string(&*read_guard)?
        };
        fs::write(&self.path, content)?;
        Ok(())
    }
}
