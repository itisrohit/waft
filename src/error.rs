//! Custom error types for the waft application.

use thiserror::Error;

/// Core error type representing any error that can occur in waft.
#[derive(Error, Debug)]
pub enum WaftError {
    /// Peer with the given identifier could not be found.
    #[error("peer not found: {0}")]
    PeerNotFound(String),

    /// Peer explicitly rejected the file transfer request.
    #[error("transfer rejected by peer")]
    Rejected,

    /// Connection was interrupted/lost in the middle of a transfer.
    #[error("connection lost mid-transfer after {bytes_sent} bytes")]
    Interrupted {
        /// Number of bytes successfully sent before the interruption.
        bytes_sent: u64,
    },

    /// An I/O error occurred.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// A TOML serialization error.
    #[error("toml serialization: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    /// A TOML deserialization error.
    #[error("toml deserialization: {0}")]
    TomlDeserialize(#[from] toml::de::Error),
}
