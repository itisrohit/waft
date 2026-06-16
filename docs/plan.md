# waft — project plan

> Cross-platform file transfer and clipboard sync daemon.
> Faster than LocalSend. Zero friction. Native Rust binary. No server required.

---

## North star

AirDrop feel on every OS. You run it once at boot, forget it exists, and files
move instantly between your devices. No open-app-on-both-sides. No IP addresses.
No pairing codes. Just `waft send r1-mac video.mp4` and it's done.

---

## Hard constraints

- Zero external services required for core transfer (relay is optional, not default)
- Single Rust binary per platform, no runtime dependencies
- No caching layer anywhere
- No HTTP, no multipart, no JSON bodies in the hot path

---

## Target benchmarks

Measure against LocalSend on identical hardware.

### Latency (time from "send" command to first byte received)

| Scenario                  | LocalSend (baseline)      | waft target  |
|---------------------------|---------------------------|--------------|
| Known peer, same LAN      | ~800ms                    | < 80ms       |
| Unknown peer, same LAN    | ~1200ms (scan + dialog)   | < 300ms      |
| Clipboard push            | not supported             | < 100ms      |

### Throughput (wired 1 Gbps LAN)

| File size | LocalSend baseline | waft target    |
|-----------|--------------------|----------------|
| 1 KB      | ~800ms total       | < 50ms total   |
| 1 MB      | ~900ms total       | < 80ms total   |
| 100 MB    | ~2.1s              | < 1.2s         |
| 1 GB      | ~11s               | < 8s           |

### Resource usage

| Metric           | LocalSend       | waft target                    |
|------------------|-----------------|--------------------------------|
| Binary size      | ~50MB (Flutter) | < 5MB                          |
| Idle RAM         | ~120MB          | < 15MB                         |
| CPU at idle      | ~2%             | < 0.1%                         |
| Startup time     | ~1.2s           | < 50ms (daemon already running)|

---

## Architecture

```
~/.waft/
  identity        # Ed25519 keypair, generated once
  trust.toml      # fingerprint → trust tier
  config.toml     # user preferences
  daemon.sock     # Unix socket, CLI talks here

Ports:
  7777/TCP        # transfer
  7777/UDP        # multicast discovery
```

### Trust tiers

```
0 — blocked    reject silently, no notification
1 — ask        OS notification, user accepts (default for new peers)
2 — trusted    auto-accept, save to ~/Downloads/waft/
3 — own        auto-accept + auto-open after receive
```

Tier promotion: first manual accept auto-promotes peer to tier 2.

### Transfer protocol (no HTTP, no JSON)

```
[sender]                              [receiver]
  open TCP :7777
  write 64-byte header ─────────────►
    magic:      [0xFA, 0x57]
    version:    u8
    flags:      u8
    name_len:   u16
    file_size:  u64
    blake3:     [u8; 32]
  write name:   [u8; name_len] ─────►
  write pubkey: [u8; 32] ───────────►
  write signature: [u8; 64] ────────►
                                       check trust tier & file status
                                       if already complete and matches hash:
                              ◄─────── 1-byte ACK (0x02 - Done/Skip)
                                       if partial file (.part) exists:
                              ◄─────── 9-byte ACK (0x03 - Resume + 8-byte BE u64 offset)
                                       else:
                              ◄─────── 1-byte ACK (0x01 - Accept from 0)
  stream remaining bytes ───────────► (from offset to file_size)
                                       write to <BLAKE3_HASH>.part
                                       verify blake3 of entire file
                              ◄─────── 1-byte DONE (0x02 for Success, 0x00 for HashMismatch)
```

### Key technical decisions

- **BLAKE3**: 3-5× faster than SHA-256, parallel, computed concurrently with
  the send so verification adds zero transfer latency
- **sendfile(2) / splice(2)** on Linux/macOS: zero-copy, file never enters userspace
- **2MB read buffers**: matches huge page size, 256× fewer syscalls than 8KB
- **TCP_NODELAY**: eliminates 200ms Nagle batching on small files
- **4MB socket buffers**: saturates bandwidth-delay product on 1 Gbps links
- **io_uring** (v2+, Linux only, feature flag): zero syscalls per I/O op
- **Application-Layer Resumption (Atomic Offset Resume)**: receiver writes to temporary `.part` files named with the expected BLAKE3 hash. If connection drops, the receiver doesn't delete the `.part` file, enabling the sender to resume by querying the receiver's current offset during handshake.

### QUIC decision gate (v0.2) - Resolved (Stay on TCP)

An experimental branch was created to evaluate QUIC and custom UDP (WUDP) options against TCP.
Benchmarks showed:
- Raw throughput for TCP Zero-Copy significantly outpaced QUIC/UDP on local networks (440+ MB/s on loopback vs < 150 MB/s for QUIC/UDP implementations).
- Connection establishment latency was comparable for hot connections.
- Decision: Stay on TCP to keep complexity minimal, and implement robust connection-recovery (resumable file transfers) at the application layer to achieve robustness.

---

## Project structure

```
waft/
  src/
    main.rs         # entry point, subcommand dispatch only
    daemon.rs       # tokio runtime, wires all modules together
    identity.rs     # Ed25519 keygen, load/save
    discovery.rs    # UDP multicast announce + passive listen
    trust.rs        # trust.toml read/write, tier logic
    transfer.rs     # TCP server + client, 64-byte header protocol
    send.rs         # sendfile / zero-copy path
    clipboard.rs    # OS hook, rolling hash, push/receive
    cli.rs          # thin Unix socket client, no logic
    error.rs        # single WaftError enum, all errors live here
  tests/
    test_transfer.rs
    test_discovery.rs
    test_trust.rs
    test_clipboard.rs
    test_perf.rs    # perf regression, runs in CI
  benches/
    bench_transfer.rs   # criterion benches, not run in CI
  .github/
    workflows/
      ci.yml
      release.yml
    ISSUE_TEMPLATE/
      bug.yml
      feature.yml
    pull_request_template.md
  docs/
    protocol.md     # wire format spec
    architecture.md # module map, data flow
  CONTRIBUTING.md
  CHANGELOG.md
  Cargo.toml
  plan.md           # this file
```

---

## Code practices

### General

- **One thing per file.** Each module owns exactly one concept. `transfer.rs`
  does not touch discovery. `trust.rs` does not touch sockets. If you're
  importing across more than two modules for one feature, the abstraction is wrong.
- **Errors are explicit, never panicked.** Every `unwrap()` is a bug waiting to
  ship. Use `?`, return `Result`, and add context with `.context("what failed")`.
  The only allowed `unwrap()` is in tests and in truly unreachable branches
  (document why with a comment).
- **No `unwrap()` in library code.** Clippy lint `clippy::unwrap_used` is enabled.
- **No magic numbers.** Every constant has a name and lives in the module it
  belongs to. `const CHUNK_SIZE: usize = 2 * 1024 * 1024; // 2MB, matches huge page`
- **No clever code.** If you have to think for more than 3 seconds to understand
  a line, rewrite it. Waft is infrastructure — clarity beats cleverness every time.

### Rust specifics

```toml
# Cargo.toml — enforced lints
[lints.rust]
unsafe_code = "forbid"       # no unsafe outside send.rs (sendfile FFI)
unused_imports = "warn"
dead_code = "warn"

[lints.clippy]
unwrap_used = "warn"
expect_used = "warn"         # use .context() instead
panic = "warn"
pedantic = "warn"
```

- `unsafe` is allowed only in `send.rs` for `sendfile`/`splice` FFI, wrapped
  in a safe abstraction. Every `unsafe` block has a `// SAFETY:` comment.
- Prefer `thiserror` for library errors, `anyhow` for binary/CLI errors.
- All public types and functions have doc comments. `///` not `//`.
- No `clone()` in hot paths. If you're cloning inside a transfer loop, stop and
  think — you probably want an `Arc` or a reference.
- `tokio::spawn` tasks are named: `tokio::task::Builder::new().name("discovery-listener")`.
  Makes `tokio-console` and crash logs readable.

### Module boundaries

Each module exposes a minimal public API. Internal helpers are `pub(crate)` at
most. The pattern:

```rust
// transfer.rs — public API surface
pub async fn serve(config: &Config, trust: Arc<TrustStore>) -> Result<()>
pub async fn send(peer: &Peer, path: &Path) -> Result<TransferResult>

// everything else is private
```

Nobody outside `transfer.rs` knows how the header is serialized. If you need to
change the wire format, you change one file.

### Error handling

Single error enum in `error.rs`:

```rust
#[derive(thiserror::Error, Debug)]
pub enum WaftError {
    #[error("peer not found: {0}")]
    PeerNotFound(String),

    #[error("transfer rejected by peer")]
    Rejected,

    #[error("connection lost mid-transfer after {bytes_sent} bytes")]
    Interrupted { bytes_sent: u64 },

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
```

Callers match on variants. No string matching on error messages. No `.unwrap()`
to "handle" an error.

### Logging

Use `tracing`, not `println!`. Structured fields, not string formatting:

```rust
// good
tracing::info!(peer = %peer.name, bytes = file_size, "transfer started");

// bad
println!("Starting transfer to {} ({} bytes)", peer.name, file_size);
```

Log levels:
- `error`: something failed and the user needs to know
- `warn`: degraded behavior (fell back to buffered copy, peer timed out)
- `info`: significant lifecycle events (daemon started, peer connected, transfer done)
- `debug`: useful for debugging but too noisy for normal use
- `trace`: wire-level detail, only for protocol debugging

Default log level in release build: `info`. Set `WAFT_LOG=debug` to increase.

---

## Architecture practices

### The daemon is the only stateful thing

All state lives in the daemon process. The CLI is a thin client — it sends a
command over the Unix socket and prints the response. No state in the CLI. This
means:

- `waft send` opens `~/.waft/daemon.sock`, sends a JSON command, streams the
  progress response, exits.
- The daemon handles peer lists, trust state, open connections, clipboard watcher.
- If the daemon isn't running, the CLI starts it automatically and retries.

### No shared mutable state without a clear owner

Each subsystem owns its state behind an `Arc<Mutex<T>>` or `Arc<RwLock<T>>`,
never a global. Pass dependencies explicitly:

```rust
// good — explicit ownership
pub async fn serve(trust: Arc<TrustStore>, peers: Arc<PeerMap>) -> Result<()>

// bad — implicit global
static TRUST: Lazy<Mutex<TrustStore>> = ...;
```

### Feature flags for platform-specific code

```toml
# Cargo.toml
[features]
io-uring = ["tokio-uring"]     # Linux only, opt-in
# quic = ["quinn"]             # evaluated and removed in v0.2
```

Platform-specific code uses `#[cfg(target_os = "linux")]` blocks, not runtime
detection. The binary for each platform is built with only what that platform needs.

---

## GitHub practices

### Branch model

```
main          production-ready, always green CI
dev           integration branch, PRs merge here first
feat/<name>   feature branches off dev
fix/<name>    bugfix branches
bench/<name>  benchmark experiment branches
```

Rules:
- `main` is protected. No direct push. Requires PR + CI green + one review.
- `dev` merges into `main` when a version milestone is complete.
- Feature branches are short-lived. If a branch is open for more than 2 weeks,
  it needs to be broken up or abandoned.

### Commits

Follow Conventional Commits. Format: `type(scope): short description`

```
feat(transfer): add sendfile zero-copy path for Linux
fix(discovery): remove self from peer list on announce
perf(send): increase chunk size to 2MB
test(transfer): add interruption recovery test
docs(protocol): document 64-byte header wire format
chore(deps): update blake3 to 1.5.1
```

Types: `feat`, `fix`, `perf`, `test`, `docs`, `chore`, `refactor`

Rules:
- Subject line ≤ 72 chars, imperative mood ("add" not "added")
- Body explains *why*, not *what* (the diff shows what)
- Breaking changes: add `!` after type: `feat(protocol)!: bump wire version to 2`
- One logical change per commit. Don't mix a feature and a refactor.

### Pull requests

Every PR needs:
- Title in Conventional Commits format
- Description: what changed, why, how to test it
- Tests added or updated
- `bench_results.md` updated if it touches the transfer path

PR template (`.github/pull_request_template.md`):

```markdown
## What
<!-- one paragraph, what this PR does -->

## Why
<!-- why this change is needed -->

## How to test
<!-- exact commands to verify it works -->

## Benchmarks affected?
<!-- if yes, paste before/after numbers -->

## Checklist
- [ ] tests added / updated
- [ ] clippy passes (`cargo clippy -- -D warnings`)
- [ ] no new unwrap() in library code
- [ ] CHANGELOG.md updated
```

### Issues

Two templates:

**Bug** (`bug.yml`): version, OS, reproduction steps, expected vs actual behavior.

**Feature** (`feature.yml`): what problem it solves, proposed behavior, what it
explicitly does not do (scope control).

Label taxonomy: `bug`, `perf`, `feature`, `protocol`, `platform/linux`,
`platform/macos`, `platform/windows`, `good first issue`, `blocked`.

`good first issue` is reserved for tasks with:
- Clear acceptance criteria
- No deep domain knowledge required
- Estimated < 4 hours
- A module file already exists to put the code in

### Releases

Releases are tagged from `main`: `v0.1.0`, `v0.2.0`, etc.

`release.yml` workflow on tag push:
- Runs full test suite
- Builds release binaries for all three platforms
- Attaches binaries to GitHub Release
- Generates CHANGELOG section from Conventional Commits since last tag

CHANGELOG format: keep-a-changelog style, hand-curated summary at top,
auto-generated commit list below.

---

## Versions

---

### v0.1 — core transfer (2 weeks)

**Goal:** send a file between two machines on the same LAN, correctly and fast.
Nothing else. No clipboard, no tray, no relay.

**Tasks:**

- [x] `identity.rs` — Ed25519 keypair, generate on first run, save to `~/.waft/identity`
- [x] `discovery.rs` — UDP multicast announce every 2s + passive listener forever
- [x] `transfer.rs` — TCP server: read header, check trust, write file
- [x] `send.rs` — TCP client: connect, write header, stream with 2MB buffers
- [x] `trust.rs` — `trust.toml`, tier 1 default, promote after first accept
- [x] `error.rs` — `WaftError` enum, all error variants for v0.1
- [x] `daemon.rs` — tokio runtime, bind 7777, run discovery + transfer tasks
- [x] `cli.rs` — Unix socket client, `waft send`, `waft list`, `waft trust`
- [x] `main.rs` — subcommand dispatch, auto-start daemon if not running

**Definition of done:**
```bash
# terminal 1 (receiver)
waft daemon

# terminal 2 (sender, different machine or loopback)
waft send rohit-laptop video.mp4
# → "sending video.mp4 to rohit-laptop (2.3 GB)... done in 9.1s"
# file appears at ~/Downloads/waft/video.mp4
```

**Tests (v0.1):**

```rust
// tests/test_transfer.rs
async fn test_small_file_roundtrip()        // 1KB, verify blake3
async fn test_large_file_roundtrip()        // 100MB, verify hash, no corruption
async fn test_header_parse_fuzzing()        // random bytes → must not panic
async fn test_connection_refused_graceful() // peer offline → clean error
async fn test_transfer_interrupted()        // kill mid-transfer → partial file cleaned up
async fn test_concurrent_transfers()        // two senders simultaneously → both complete

// tests/test_discovery.rs
async fn test_peer_appears_on_announce()    // B sees A within 3s
async fn test_peer_disappears_on_timeout()  // A stops → B removes after 10s
async fn test_no_self_discovery()           // daemon never adds itself

// tests/test_trust.rs
async fn test_unknown_peer_is_tier1()
async fn test_promote_to_tier2_after_accept()
async fn test_blocked_peer_rejected_silently()
async fn test_trust_persists_across_restart()
async fn test_tier3_auto_opens_file()
```

---

### v0.2 — performance + benchmark suite (1 week)

**Goal:** prove the numbers. Zero-copy path. QUIC decision gate. Profiling pass.

**Tasks:**

- [x] `send.rs` — `sendfile(2)` on Linux, `sendfile` on macOS, buffered fallback on Windows
- [x] Socket tuning — `TCP_NODELAY` + 4MB buffers on both sides of every connection
- [x] BLAKE3 hashing concurrent with send (separate thread, shared mmap buffer)
- [x] `benches/bench_transfer.rs` — criterion harness scaffolded
- [x] Run benchmark matrix, commit results to `bench_results.md`
- [x] QUIC branch experiment — add `quinn`, run same matrix, compare p50/p95/p99
- [x] Decision: document outcome in `bench_results.md`, merge winning transport (TCP Zero-Copy with resumable transfers)
- [x] `cargo flamegraph` pass — no hot allocation in transfer loop
- [x] Application-layer resumable file transfers (Atomic Offset Resumption) over TCP Zero-Copy

**Benchmark harness:**

```rust
// benches/bench_transfer.rs — run with: cargo bench

const SIZES: &[usize] = &[
    1_024,           //   1 KB
    102_400,         // 100 KB
    10_485_760,      //  10 MB
    104_857_600,     // 100 MB
    1_073_741_824,   //   1 GB
];

// metrics per run: connection_setup_ms, first_byte_ms, total_ms, throughput_mbps
// iterations: 100 per size
// report: p50 / p95 / p99

// conditions:
//   clean        no impairment
//   lossy_1pct   sudo tc qdisc add dev lo root netem loss 1%
//   lossy_5pct   sudo tc qdisc add dev lo root netem loss 5%
```

**Perf regression tests (run in CI, loopback only, no tc needed):**

```rust
// tests/test_perf.rs
async fn perf_1mb_under_100ms()           // total_ms < 100
async fn perf_100mb_throughput_floor()    // throughput > 800 MB/s on loopback
async fn perf_small_file_latency()        // 1KB known peer, first_byte_ms < 10
async fn perf_no_regression_vs_baseline() // compare to recorded baseline in fixtures/
async fn test_tcp_resume_transfer()        // partial transfer interrupted and resumed from offset
```

**How to run the full benchmark comparison vs LocalSend:**

```bash
# generate test files once
dd if=/dev/urandom of=/tmp/test_1kb.bin  bs=1K   count=1
dd if=/dev/urandom of=/tmp/test_1mb.bin  bs=1M   count=1
dd if=/dev/urandom of=/tmp/test_100mb.bin bs=1M  count=100

# compare with hyperfine (install: cargo install hyperfine)
hyperfine --warmup 5 --runs 50 \
  'waft send rohit-phone /tmp/test_100mb.bin' \
  'localsend_cli send /tmp/test_100mb.bin'

hyperfine --warmup 10 --runs 100 \
  'waft send rohit-phone /tmp/test_1kb.bin' \
  'localsend_cli send /tmp/test_1kb.bin'
```

Record results in `bench_results.md`. If waft does not beat LocalSend p95
latency by ≥ 30% on small files, fix the connection setup path before v0.3.

---

### v0.3 — clipboard sync (1 week)

**Goal:** copy on one device, paste on another. Under 100ms on LAN.

**Tasks:**

- [ ] `clipboard.rs` — arboard for OS clipboard access, cross-platform
- [ ] Rolling hash: poll every 200ms, `u64` hash of content, push on change
- [ ] Push to all tier-2+ peers on open persistent connection (reuse transfer socket)
- [ ] Receiver updates clipboard only if incoming hash ≠ current hash
- [ ] `clipboard_sync = true/false` in `~/.waft/config.toml`, default false
- [ ] Text only in v0.3

**Tests (v0.3):**

```rust
// tests/test_clipboard.rs
async fn test_push_on_change()              // change on A, B receives within 200ms
async fn test_no_push_if_unchanged()        // same content twice → one push only
async fn test_only_pushes_to_trusted()      // tier-1 peer never receives clipboard
async fn test_unicode_roundtrip()           // exact bytes preserved
async fn test_disabled_by_config()          // clipboard_sync=false → no pushes
async fn test_large_clipboard_capped()      // content > 1MB is not pushed
```

---

### v0.4 — multi-platform + polish (2 weeks)

**Goal:** works on Linux, macOS, Windows. Daemon autostart. Tray icon. Share sheet.

**Tasks:**

- [ ] Linux: `systemd` user service (`~/.config/systemd/user/waft.service`)
- [ ] macOS: `launchd` plist (`~/Library/LaunchAgents/dev.waft.plist`)
- [ ] Windows: startup registry key or Windows Service
- [ ] Tray icon (tray-icon crate): peer list, right-click → send file
- [ ] macOS Share Sheet: thin Swift CLI wrapper, calls `waft send`
- [ ] macOS sendfile correct signature (`off_t*` differs from Linux)
- [ ] Windows zero-copy via `TransmitFile` Win32 API
- [ ] Clipboard images: PNG bytes, cap 10MB
- [ ] `waft receive --watch`: print received file paths to stdout (scriptable)
- [ ] Shell completions: bash, zsh, fish via clap_complete
- [ ] Man page: `waft.1` generated from clap

**Tests (v0.4):**

```rust
fn test_systemd_unit_written_correctly()
fn test_launchd_plist_written_correctly()
async fn test_cross_platform_header_compat()  // Linux → macOS header parse
async fn test_image_clipboard_roundtrip()
async fn test_receive_watch_stdout()
```

---

### v0.5 — relay + cross-network (future, conditional)

**Gate:** only build if v0.4 users explicitly request cross-network support.

**Approach:**
- Minimal relay: WebSocket hole-punch broker, ~60 lines of Rust
- Relay signals the connection — file bytes remain direct P2P
- Self-hostable, one-click deploy to Fly.io / Railway
- No state in relay, no file bytes ever touch it

---

### v0.6 — local AI agent skills (developer tooling)

**Goal:** Run instant, zero-cost AI-assisted code review and performance audits locally via native slash commands.

**Tasks:**
- [x] Configure workspace unified slash command `/review` under `.agents/skills/review/SKILL.md`.
- [x] Integrate compiler checks (`clippy`), security pattern checks (`semgrep`), and interactive fix applications.
- [x] Enable automatic dependency checking and installation in the skill workflow.
- [x] Configure workspace unified slash command `/optimize` under `.agents/skills/optimize/SKILL.md`.
- [x] Integrate performance lints (`clippy --perf`), binary size analysis (`cargo-bloat`, `cargo-llvm-lines`), assembly inspection (`cargo-show-asm`), syscall tracing (`strace`/`fs_usage`), and CPU flamegraph generation (`cargo-flamegraph`).
- [x] Both skills follow the 2026 Agent Skills open standard (YAML frontmatter with `name`, `description`, `metadata`). Compatible with Claude Code CLI, OpenCode CLI, Codex CLI, and Antigravity IDE.

---

## What is explicitly not in scope (ever)

- No cloud relay operated by waft project
- No accounts, sign-in, or phone number
- No file history or recent transfers list
- No web UI
- No mobile app in v1 — desktop first
- No caching of any kind
- No chunked parallel transfer of a single file

---

## Crate decisions

| Purpose            | Crate          | Notes                              |
|--------------------|----------------|------------------------------------|
| Async runtime      | tokio          | full feature set                   |
| Identity           | ed25519-dalek  | keypair gen + sign                 |
| Hashing            | blake3         | parallel, concurrent with send     |
| Zero-copy Linux    | nix            | sendfile(2), splice(2)             |
| Socket tuning      | socket2        | TCP_NODELAY, buffer sizes          |
| Clipboard          | arboard        | cross-platform                     |
| Tray icon          | tray-icon      | v0.4 only                          |
| CLI                | clap           | derive macro                       |
| Config / trust     | toml + serde   | trust.toml, config.toml            |
| Error handling     | thiserror      | library errors                     |
| Binary errors      | anyhow         | CLI / daemon top-level             |
| Logging            | tracing        | structured, levels                 |
| Benchmarks         | criterion      | benches/ only, not in CI           |
| io_uring           | tokio-uring    | v0.2+, Linux only, feature flag    |
| QUIC               | quinn          | Evaluated and removed in v0.2      |

---

## CI pipeline & Code Quality Checks

We use a robust CI pipeline on every push and pull request to ensure high quality, type checking, security auditing, and license compliance across all target operating systems.

```yaml
# .github/workflows/ci.yml
name: CI

on:
  push:
    branches: [ main, dev ]
  pull_request:
    branches: [ main, dev ]

permissions:
  contents: read

env:
  CARGO_TERM_COLOR: always

jobs:
  test:
    name: Test and Lint
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    steps:
      - name: Checkout Code
        uses: actions/checkout@v4

      - name: Setup Rust Toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - name: Cache Cargo Dependencies
        uses: Swatinem/rust-cache@v2

      - name: Check Format
        run: cargo fmt --all --check

      - name: Run Clippy Lints
        run: cargo clippy --all-targets --all-features -- -D warnings

      - name: Run Tests
        run: cargo test --all-features

  audit:
    name: Security Audit
    runs-on: ubuntu-latest
    steps:
      - name: Checkout Code
        uses: actions/checkout@v4

      - name: Setup Rust Toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Install cargo-audit
        run: cargo install cargo-audit --locked

      - name: Security Vulnerability Scan
        run: cargo audit
```

### Compliance & Quality Tools
- **`rustfmt.toml`**: Standardizes code formatting (e.g. max width, grouping imports, and edition compliance).
- **`deny.toml`**: Managed via `cargo-deny`, enforcing MIT/Apache-2.0 licenses, avoiding duplicate dependency versions, and validating crate sources.
- **`build.rs`**: Automatically configures the local Git repository's `core.hooksPath` to point to `.githooks/` whenever any Cargo commands (`build`, `check`, `test`) are executed, ensuring git pre-commit checks run transparently for all developers without manual configuration steps.

- **`codeql.yml`**: Triggers weekly and on push/PR, compiling the codebase and performing static analysis (SAST) to detect security vulnerabilities (e.g. data leakages, unsafe FFI, path injection).
- **`dependabot.yml`**: Scans weekly to find outdated or vulnerable Cargo dependencies and GitHub Actions, automatically creating pull requests to keep dependencies safe.

---

## CONTRIBUTING.md outline

```markdown
# Contributing to waft

## Philosophy
waft values correctness over cleverness, clarity over abstraction,
and minimal surface area over features.

Before opening a PR, check the "not in scope" list in plan.md.
If your idea is on that list, open a discussion issue first.

## Setup
  cargo build
  cargo test
  cargo clippy -- -D warnings

## Good first issues
Look for the `good first issue` label. Each one has exact acceptance
criteria and points to the file to edit.

## Where things live
  src/transfer.rs    wire protocol
  src/discovery.rs   peer discovery
  src/trust.rs       trust tier logic
  src/clipboard.rs   clipboard sync
  src/cli.rs         CLI commands
  tests/             integration tests
  benches/           criterion benchmarks
  docs/protocol.md   wire format spec

## Running benchmarks
  cargo bench
  # compare with LocalSend: see plan.md § "How to measure success"

## Commit format
  feat(scope): description
  fix(scope): description
  perf(scope): description
  (see plan.md § Commits for full guide)
```

---

*last updated: june 2026*