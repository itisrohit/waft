# Terminal Commands Guide

### Setup
```bash
# Build the project (automatically configures local Git hooks)
cargo build

# Run compiler checks
cargo check
```

## Testing

```bash
# Run all integration and unit tests
cargo test
```

## Code Quality & Lints

```bash
# Run Clippy checks (treats all warnings as errors)
cargo clippy --all-targets --all-features -- -D warnings

# Format all files in check mode
cargo fmt --all -- --check

# Format all files in-place
cargo fmt --all
```

## Security & Dependency Auditing

```bash
# Security vulnerability advisory scan (requires cargo-audit)
cargo audit

# License, duplicates, and sources check (requires cargo-deny)
cargo deny check
```

## AI Agent Skills

`waft` ships two built-in agent skills compatible with **Claude Code CLI**, **OpenCode CLI**, **Codex CLI**, and **Antigravity IDE**.

### `/review` — Security & Code Quality Audit
Runs `cargo clippy`, installs and runs `semgrep`, identifies modified files, and offers interactive diff patches to apply fixes.
```text
/review
```
Run this before opening any pull request.

### `/optimize` — Performance Audit
Runs static performance lints (`clippy --perf`), binary size analysis (`cargo-bloat`), assembly inspection (`cargo-show-asm`), syscall tracing (`strace`/`fs_usage`), and CPU flamegraph generation. Auto-installs all required tools.
```text
/optimize
```
Run this when profiling, benchmarking, or investigating throughput regressions.
