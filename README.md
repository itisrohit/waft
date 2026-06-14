# Terminal Commands Guide

### Setup
```bash
# Build the project (automatically configures local Git hooks)
cargo build

# Run compiler checks
cargo check
```

### Testing
```bash
# Run the integration and unit tests
cargo test
```

### Code Quality & Lints
```bash
# Run Clippy checks (treats all warnings as errors)
cargo clippy --all-targets --all-features -- -D warnings

# Format all files in check mode
cargo fmt --all -- --check

# Format all files in-place
cargo fmt --all
```

### Security & Dependency Auditing
```bash
# Security vulnerability advisory scan (requires cargo-audit)
cargo audit

# License, duplicates, and sources check (requires cargo-deny)
cargo deny check
```

### AI Code Review (Agent Skill)
Waft supports the unified agent skill standard for AI-assisted workspace reviews:
* If you are developing inside **Claude Code CLI**, **OpenCode CLI**, **Codex CLI**, or **Antigravity IDE**, you can type:
  ```text
  /review
  ```
  This automatically runs `cargo clippy`, checks for `semgrep` (installing it if missing), identifies modified files, and prompts you with interactive diff patches to apply fixes.

