# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added
- Application-layer resumable file transfers (Atomic Offset Resumption) over TCP Zero-Copy to survive network drops.
- Receiver-side atomic writing using `<BLAKE3_HASH>.part` temp files and post-transfer BLAKE3 integrity verification.
- Handshake negotiation using resume ACK byte `0x03` and 8-byte big-endian `u64` offset representation.
- Offset-seeking sender support using macOS zero-copy `libc::sendfile` seek arguments and memory-mapped slice slicing fallback.
- Integration test `test_tcp_resume_transfer` verifying partial file persistence, offset resume negotiation, and completion.
- Zero-copy file transfers via raw OS `libc::sendfile` bindings on Unix (Linux and macOS) targets.
- Dynamic socket send buffer sizing and chunk size bounding (clamped between 64KB and 2MB) dynamically adjusted based on `SO_SNDBUF`.
- Virtual memory-mapped file reading via the `memmap2` crate for cross-platform file transfers and fast fallback streaming on Windows.
- Concurrently overlapped file hashing and TCP connection establishment using `tokio::try_join!`.
- Automated loopback latency, throughput, and performance integration test suite in `tests/test_perf.rs`.
- IPC Unix socket daemon server (`src/daemon.rs`) and CLI client (`src/cli.rs`) implementation to coordinate file transfers and LAN peer tracking.
- Subcommand CLI argument parser and routing dispatch (`src/main.rs`) using the `clap` derive macro.
- CLI user interface improvements: beautiful formatted terminal tables for listing peers and trust configurations, and a dynamic carriage-return progress bar.
- Automatic transfer cancellation in the daemon, aborting active file transfers immediately if the IPC client disconnects (Ctrl+C).
- Optimized release profile in `Cargo.toml` with LTO, abort panics, single codegen units, and symbol stripping.
- Lock contention optimizations: scoped lock guards in `src/trust.rs` and `src/daemon.rs` to release read locks before executing blocking I/O and loop iterations.
- Scaffolded Criterion loopback benchmarking harness (`benches/bench_transfer.rs`) and dependencies.
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
- Deterministic discovery tests in `tests/test_discovery.rs` covering announcement parsing, invalid/self-announcement rejection, throttling, and peer expiry cleanup.
- Ignored multicast smoke coverage in `tests/test_discovery_multicast.rs` for end-to-end announcer/listener verification on compatible runners.
- Integration tests in `tests/test_trust.rs` checking trust level query, update, persistence, and invalid file recovery.
- Integration tests in `tests/test_identity.rs` checking uniqueness, save/load, and sign/verify functions.
- `lib.rs` to expose the library target of the crate.
- `rand_core` dependency in `Cargo.toml` to access `OsRng` directly.
- License metadata and `deny.toml` rules update to allow required dependency licenses.
- Initial workspace setups, lint configurations, and CI/CD pipelines.

### Changed
- Replaced the experimental UDP/QUIC module stack (`wudp_send`, `wudp_transfer`, `wudp_transport`, Quinn, and rustls dependencies) with robust resumable TCP Zero-Copy.

### Fixed
- Critical shell injection security vulnerability in GitHub Actions `Promotion Gate` workflow by utilizing intermediate environment variables.
- Replaced `unwrap()` in performance integration test `tests/test_perf.rs` with safe error handling to adhere to pedantic guidelines.
