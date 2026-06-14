# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added
- `identity.rs` module for Ed25519 cryptographic identity key generation, file storage, and signing.
- `error.rs` module for custom error types representing core system and I/O states.
- `trust.rs` module implementing `TrustTier` configurations and thread-safe file-backed `TrustStore`.
- Integration tests in `tests/test_trust.rs` checking trust level query, update, persistence, and invalid file recovery.
- Integration tests in `tests/test_identity.rs` checking uniqueness, save/load, and sign/verify functions.
- `lib.rs` to expose the library target of the crate.
- `rand_core` dependency in `Cargo.toml` to access `OsRng` directly.
- License metadata and `deny.toml` rules update to allow required dependency licenses.
- Initial workspace setups, lint configurations, and CI/CD pipelines.
