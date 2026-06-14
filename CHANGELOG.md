# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added
- Core TCP file transfer receiver server and wire protocol in `transfer.rs` with automatic trust promotion, path traversal sanitization, and parallel BLAKE3 hash verification.
- Core TCP file transfer client in `send.rs` supporting protocol serialization, metadata signing, and file streaming.
- Cross-platform socket performance optimizations (enabling `TCP_NODELAY` and setting 4MB send/recv buffers on streams safely via `socket2`).
- Comprehensive transfer integration test suite in `tests/test_transfer.rs` covering small/large file roundtrips, blocked peer rejection, partial download cleanup on interruption, virtual time-warped read timeouts, and fuzzing robustness.
- New protocol-specific error variants (`InvalidHeader`, `SignatureVerification`, and `HashMismatch`) in `error.rs`.
- Unified `/review` agent skill under `.agents/skills/review/SKILL.md` supporting clippy, semgrep, and auto-installation check.
- Unified `/optimize` agent skill under `.agents/skills/optimize/SKILL.md` for automated performance auditing: static perf lints, binary size analysis, assembly inspection, syscall tracing, and CPU flamegraph generation.
- `identity.rs` module for Ed25519 cryptographic identity key generation, file storage, and signing.
- `discovery.rs` module for UDP multicast peer presence discovery and mapping.
- `error.rs` module for custom error types representing core system and I/O states.
- `trust.rs` module implementing `TrustTier` configurations and thread-safe file-backed `TrustStore`.
- Integration tests in `tests/test_discovery.rs` verifying peer announcement, timeout, and self-discovery filtering.
- Integration tests in `tests/test_trust.rs` checking trust level query, update, persistence, and invalid file recovery.
- Integration tests in `tests/test_identity.rs` checking uniqueness, save/load, and sign/verify functions.
- `lib.rs` to expose the library target of the crate.
- `rand_core` dependency in `Cargo.toml` to access `OsRng` directly.
- License metadata and `deny.toml` rules update to allow required dependency licenses.
- Initial workspace setups, lint configurations, and CI/CD pipelines.

### Fixed
- Critical shell injection security vulnerability in GitHub Actions `Promotion Gate` workflow by utilizing intermediate environment variables.
