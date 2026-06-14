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
# Run all integration and unit tests
cargo test --all-features

# Run ignored multicast smoke coverage on a compatible machine
cargo test --all-features --test test_discovery_multicast -- --ignored
```

### Code Quality & Lints
```bash
# Run Clippy checks (treats all warnings as errors)
cargo clippy --all-targets --all-features -- -D warnings

# Format all files in check mode
cargo fmt --all --check

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

