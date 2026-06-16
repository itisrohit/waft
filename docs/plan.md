# waft — project plan

> Cross-platform file transfer and clipboard sync daemon.
> Faster than LocalSend. Zero friction. Native Rust binary. No server required.

---

## Goal

AirDrop feel on every OS. Run it once at boot, forget it exists, and files move instantly between devices. No open-app-on-both-sides. No IP addresses. No pairing codes. Just `waft send r1-mac video.mp4` and it's done.

---

## Hard constraints

- Zero external services required for core transfer (relay is optional, not default)
- Single Rust binary per platform, no runtime dependencies
- No caching layer anywhere
- No HTTP, no multipart, no JSON bodies in the hot path

---

## Target benchmarks

| Scenario | LocalSend baseline | waft target |
|---|---|---|
| Known peer, same LAN latency | ~800ms | < 80ms |
| Unknown peer, same LAN | ~1200ms | < 300ms |
| Clipboard push | not supported | < 100ms |
| 1 MB transfer total | ~900ms | < 80ms |
| 100 MB transfer total | ~2.1s | < 1.2s |
| Binary size | ~50MB (Flutter) | < 5MB |
| Idle RAM | ~120MB | < 15MB |

---

## Architecture

```
~/.waft/
  identity        # Ed25519 keypair, generated once
  trust.toml      # fingerprint → trust tier
  config.toml     # user preferences (clipboard_sync, etc.)
  daemon.sock     # Unix socket, CLI talks here

Ports:
  7777/TCP        # file transfer
  7777/UDP        # multicast peer discovery
```

### Trust tiers

```
0 — blocked    reject silently, no notification
1 — ask        notify, user accepts (default for new peers)
2 — trusted    auto-accept, save to ~/Downloads/waft/
3 — own        auto-accept + auto-open after receive
```

Tier promotion: first manual accept auto-promotes the peer to tier 2.

### Wire protocol

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
  write name:   [u8; name_len] ──────►
  write pubkey: [u8; 32] ────────────►
  write sig:    [u8; 64] ────────────►
                                       check trust tier & file status
                                       if complete file + hash match:
                              ◄─────── 0x02 (skip)
                                       if <BLAKE3>.part exists:
                              ◄─────── 0x03 + 8-byte BE u64 offset (resume)
                                       else:
                              ◄─────── 0x01 (accept from 0)
  stream bytes from offset ──────────►
                                       write to <BLAKE3_HASH>.part
                                       verify blake3 of completed file
                              ◄─────── 0x02 (done) or 0x00 (hash mismatch)
```

### Key technical decisions

- **BLAKE3**: parallel hashing, 3–5× faster than SHA-256
- **sendfile(2)** on Linux/macOS: zero-copy kernel path, file never enters user-space
- **2MB buffers**: matches huge page size, 256× fewer syscalls than 8KB reads
- **TCP_NODELAY + 4MB socket buffers**: eliminates Nagle delay, saturates 1 Gbps
- **Atomic Offset Resumption**: receiver writes to `<BLAKE3>.part`; on reconnect, sender resumes from the receiver's current file size — no restart required
- **Ed25519 signatures**: every transfer header is signed; unsigned or tampered connections are rejected before any data is written

---

## Project structure

```
waft/
  src/
    main.rs         # subcommand dispatch only
    daemon.rs       # tokio runtime, wires all modules
    identity.rs     # Ed25519 keypair, load/save, signing
    discovery.rs    # UDP multicast announce + passive listen
    trust.rs        # trust.toml, tier logic
    transfer.rs     # TCP receiver, header protocol, resumption
    send.rs         # TCP sender, zero-copy, offset seeking
    clipboard.rs    # OS clipboard hook, rolling hash, push/receive
    cli.rs          # thin IPC client, no logic
    error.rs        # WaftError enum
    lib.rs
  tests/
    test_transfer.rs
    test_discovery.rs
    test_trust.rs
    test_identity.rs
    test_clipboard.rs
    test_perf.rs          # perf regression, runs in CI
    test_cli_daemon.rs
  benches/
    bench_transfer.rs     # criterion, not run in CI
  docs/
    plan.md
    bench_results.md
    experiments/
      transport_evaluation.md   # TCP vs QUIC vs WUDP evaluation
  .agents/
    skills/
      review/SKILL.md
      optimize/SKILL.md
  CHANGELOG.md
  CONTRIBUTING.md
  Cargo.toml
  build.rs
```

---

## Code standards

- **One concept per file.** `transfer.rs` does not touch discovery. `trust.rs` does not touch sockets.
- **No `unwrap()` in library code.** Use `?` and `.context()`. Clippy `unwrap_used` is enabled.
- **No magic numbers.** Every constant has a name and a comment explaining why.
- **`unsafe` only in `send.rs`** for `sendfile`/`splice` FFI. Every block has a `// SAFETY:` comment.
- **`thiserror` for library errors, `anyhow` for CLI/daemon top-level.**
- **`tracing` not `println!`.** Structured fields, log levels: `error`, `warn`, `info`, `debug`, `trace`.
- **No `clone()` in hot paths.** Use `Arc` or references.
- **Named `tokio::spawn` tasks** for readability in crash logs and `tokio-console`.
- **Module public API is minimal.** Internal helpers are `pub(crate)` at most.
- **State only in the daemon.** The CLI is a thin IPC client. It sends a command and prints the response. No shared state.

---

## Milestones

### v0.1 — core transfer ✅

- [x] `identity.rs` — Ed25519 keypair, generate on first run
- [x] `discovery.rs` — UDP multicast announce every 2s + passive listener
- [x] `transfer.rs` — TCP receiver: header, trust check, write file
- [x] `send.rs` — TCP sender: connect, header, stream 2MB chunks
- [x] `trust.rs` — `trust.toml`, tier logic, promote on first accept
- [x] `error.rs` — `WaftError` enum
- [x] `daemon.rs` — tokio runtime, bind 7777, run all tasks
- [x] `cli.rs` — IPC client: `waft send`, `waft list`, `waft trust`
- [x] `main.rs` — subcommand dispatch, auto-start daemon

---

### v0.2 — performance + transport evaluation ✅

- [x] `send.rs` — `sendfile(2)` on Linux/macOS, memory-mapped fallback on Windows
- [x] Socket tuning — `TCP_NODELAY` + 4MB buffers
- [x] BLAKE3 hashing concurrent with send
- [x] `benches/bench_transfer.rs` — Criterion harness
- [x] Benchmark matrix committed to `docs/bench_results.md`
- [x] QUIC and custom UDP (WUDP) evaluated, TCP retained (see `docs/experiments/transport_evaluation.md`)
- [x] Application-layer resumable transfers (Atomic Offset Resumption)
- [x] `test_tcp_resume_transfer` integration test

**Achieved:** 446.67 MB/s loopback, 1.56 ms per 1MB under Criterion. Resumption adds < 5ms overhead for 100MB.

---

### v0.3 — clipboard sync

**Goal:** copy on one device, paste on another in under 100ms on LAN.

- [ ] `clipboard.rs` — `arboard` for OS clipboard access
- [ ] Poll every 200ms, push `u64` rolling hash of content on change
- [ ] Push to all tier-2+ peers via persistent connection per peer
- [ ] Receiver applies update only if incoming hash ≠ current clipboard hash
- [ ] `clipboard_sync = true/false` in `~/.waft/config.toml`, default `false`
- [ ] Text-only in v0.3

**Tests:**
- `test_push_on_change` — change on A, B receives within 200ms
- `test_no_push_if_unchanged` — same content twice → one push only
- `test_only_pushes_to_trusted` — tier-1 peer never receives clipboard
- `test_unicode_roundtrip` — exact bytes preserved
- `test_disabled_by_config` — `clipboard_sync=false` → no pushes
- `test_large_clipboard_capped` — content > 1MB is not pushed

---

### v0.4 — multi-platform + polish

**Goal:** works on Linux, macOS, Windows with daemon autostart and OS integration.

- [ ] Linux: `systemd` user service (`~/.config/systemd/user/waft.service`)
- [ ] macOS: `launchd` plist (`~/Library/LaunchAgents/dev.waft.plist`)
- [ ] Windows: startup registry entry or Windows Service
- [ ] Tray icon (`tray-icon` crate): peer list, right-click → send file
- [ ] macOS Share Sheet: thin Swift wrapper calling `waft send`
- [ ] Windows zero-copy via `TransmitFile` Win32 API
- [ ] Clipboard images: PNG bytes, cap 10MB
- [ ] `waft receive --watch`: print received file paths to stdout
- [ ] Shell completions: bash, zsh, fish via `clap_complete`
- [ ] Man page: `waft.1` generated from clap

---

### v0.5 — desktop UI (Tauri 2)

**Goal:** an AirDrop-style desktop companion app for macOS, Linux, and Windows. The existing daemon and CLI stay untouched — the UI is a thin layer on top, talking to the daemon over the existing Unix socket IPC.

**Approach: Tauri 2**
Tauri 2 is production-ready as of 2026. It pairs a Rust backend directly with a native-WebView frontend (no bundled Chromium). App sizes stay in the low MBs. The frontend can be any web stack — we'll use plain HTML/CSS/JS or a lightweight framework like Svelte to keep bundle size minimal.

Architecture:
```
Tauri app
  frontend (WebView / Svelte)
      ↕  Tauri IPC commands
  Tauri Rust layer
      ↕  Unix socket
  waft daemon (existing binary)
```

The Tauri Rust layer connects to the existing daemon socket. No daemon logic is duplicated inside the app.

- [ ] Scaffold Tauri 2 app in `app/` directory
- [ ] Connect Tauri backend to existing daemon Unix socket
- [ ] Peer list panel: show discovered peers, trust tier badges
- [ ] Drag-and-drop file send: drop a file onto a peer → calls `waft send`
- [ ] Transfer progress: real-time progress bar via daemon progress events
- [ ] Received file notifications: OS notification on transfer complete
- [ ] Clipboard sync toggle: on/off switch wired to `config.toml`
- [ ] System tray integration (replaces `tray-icon` crate approach from v0.4)
- [ ] Autostart on login (replaces manual launchd/systemd config from v0.4)

**Testing:**
- Manual: drag-and-drop a 100MB file between two machines, verify progress UI, verify file integrity
- Manual: kill the network mid-transfer, verify resume on reconnect surfaces correctly in UI
- Manual: tray icon peer list updates within 3s of a new peer appearing on the LAN

---

### v0.6 — mobile app (Tauri 2 iOS + Android)

**Goal:** same UI on iOS and Android. Tauri 2 targets mobile using the platform's native WebView (WKWebView on iOS, Android System WebView). The Rust backend logic runs natively on device — no daemon socket on mobile; the Rust core is compiled directly into the app.

Architecture on mobile:
```
Tauri app (iOS / Android)
  frontend (WebView / Svelte)
      ↕  Tauri IPC commands
  waft-core Rust library (no daemon, no Unix socket)
      direct function calls to transfer, discovery, trust, clipboard
```

This requires extracting the core waft logic (transfer, identity, trust, discovery) into a `waft-core` library crate that can be compiled for mobile targets without the daemon/IPC layer.

- [ ] Extract `waft-core` library crate (transfer, identity, trust, discovery — no daemon)
- [ ] Add mobile targets to Tauri 2 build (`tauri ios dev`, `tauri android dev`)
- [ ] Adaptive UI layout: same Svelte components, responsive for small screens
- [ ] iOS Share Sheet integration: share files from other apps directly into waft
- [ ] Android share intent integration
- [ ] Background service for clipboard sync on mobile (within OS limits)
- [ ] Test on physical iOS and Android devices

**Testing:**
- Send a file from macOS desktop to iOS — verify end-to-end including UI feedback on both sides
- Send from Android to Linux — verify peer discovery works on both platforms
- Interrupt transfer and resume from mobile side

---

### v0.7 — relay + cross-network (conditional)

Only build if users explicitly request cross-network (non-LAN) transfers after v0.6.

- Minimal WebSocket hole-punch broker
- Relay signals the connection only — file bytes remain direct P2P
- Self-hostable, no file bytes touch the relay server

---

### v1.0 — release gate

v1.0 is tagged from `main` only when all of the following are true:

- [ ] v0.3 clipboard sync: tested on macOS ↔ macOS and macOS ↔ Linux
- [ ] v0.4 daemon autostart: verified on all three platforms in CI
- [ ] v0.5 desktop UI: manual testing sign-off on macOS, Linux, Windows
- [ ] v0.6 mobile UI: manual testing sign-off on physical iOS and Android devices
- [ ] All `cargo test --release` passing on Linux, macOS, Windows in CI
- [ ] Binary size < 5MB on all platforms
- [ ] No known P1/P2 open issues

Release artifacts: signed binaries for macOS (arm64 + x86_64), Linux (x86_64 + aarch64), Windows (x86_64), iOS `.ipa`, Android `.apk`.

---

## Not in scope

- No cloud relay operated by the waft project
- No accounts, sign-in, or phone number
- No file transfer history or recent files list
- No chunked parallel transfer of a single file

---

## Crates

| Purpose | Crate | Notes |
|---|---|---|
| Async runtime | tokio | full features |
| Identity | ed25519-dalek | keypair, sign, verify |
| Hashing | blake3 | parallel, concurrent with send |
| Zero-copy (Unix) | libc / nix | sendfile(2), splice(2) |
| Memory mapping | memmap2 | cross-platform fallback |
| Socket tuning | socket2 | TCP_NODELAY, buffer sizes |
| Clipboard | arboard | cross-platform |
| CLI | clap | derive macro |
| Config / trust | toml + serde | trust.toml, config.toml |
| Library errors | thiserror | WaftError enum |
| CLI/daemon errors | anyhow | top-level context |
| Logging | tracing | structured, leveled |
| Benchmarks | criterion | benches/ only, not in CI |
| Desktop + mobile UI | Tauri 2 | v0.5–v0.6; native WebView, no Chromium |
| UI frontend | Svelte | lightweight, minimal bundle |

---

*last updated: june 2026*