---
name: optimize
description: "Run a structured performance optimization audit on the waft codebase. Covers static lints, syscall tracing, CPU profiling via flamegraph, assembly inspection, and binary size analysis. Only uses tools that can be automated by an agent. Trigger when the user asks to optimize, profile, benchmark, or check performance of waft."
allowed-tools:
  - run_command
  - view_file
  - replace_file_content
  - write_to_file
metadata:
  version: "1.0.0"
  author: "waft maintainers"
  category: "performance"
  compatibility: "Claude Code, OpenCode, Antigravity IDE"
---

You are a performance engineer and systems debugging expert. When the user runs `/optimize`, perform a structured, layered performance audit of the local `waft` codebase.

Work through the steps below **in order**, auto-installing any missing tools, and present a consolidated findings report at the end.

---

## Step 0: Detect Changed Files and Platform

1. Execute `git diff --name-only` to find modified files in the working directory.
2. If there are no modified files, check for staged changes using `git diff --cached --name-only`.
3. If no code files have changed at all, fall back to analyzing the full `src/` directory.
4. Detect the OS: run `uname -s`. Adapt syscall tracing commands based on the result (Linux uses `strace`; macOS uses `fs_usage`).

---

## Step 1: Static Lint — Fastest Check, Run First

Run clippy with performance-specific lint groups:

```bash
cargo clippy --all-targets --all-features -- \
  -D clippy::perf \
  -D clippy::pedantic \
  -W clippy::nursery \
  -A clippy::missing_errors_doc \
  -A clippy::missing_panics_doc
```

For each warning:
- Identify the file and line number.
- Classify severity: 🔴 allocating on hot path, 🟡 unnecessary clone, 🟢 style issue.
- Prepare a diff patch. Ask the user if they want it applied before moving on.

---

## Step 2: Release Profile Inspection

Read `Cargo.toml` and check for an optimized release profile. If it is missing `lto`, `codegen-units = 1`, or `panic = "abort"`, report it and suggest:

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
strip = "symbols"
```

Only flag missing fields. Do not suggest adding fields that already exist.

---

## Step 3: Binary Size Analysis

Install `cargo-bloat` if missing, then run:

```bash
cargo install cargo-bloat --quiet 2>/dev/null || true
cargo bloat --release --crates -n 15
cargo bloat --release -n 20
```

Report the top 10 functions contributing to binary size. Flag any function > 10 KB as a candidate for inlining or refactoring. Also check for monomorphization bloat:

```bash
cargo install cargo-llvm-lines --quiet 2>/dev/null || true
cargo llvm-lines --release 2>/dev/null | head -25
```

---

## Step 4: Assembly Inspection of Hot Path Functions

Install `cargo-show-asm` if missing, then inspect the two critical hot-path functions:

```bash
cargo install cargo-show-asm --quiet 2>/dev/null || true
cargo asm --release waft::transfer::handle_connection 2>/dev/null | head -80
cargo asm --release waft::send::send_file 2>/dev/null | head -80
```

Analyze the output and report in plain English:
- Presence of `call` inside the body loop → function was **not inlined** (regression risk).
- Presence of `vmovd`, `xmm`, `ymm`, `zmm` registers → SIMD **vectorization succeeded** (good).
- Excessive `push`/`pop` pairs → register pressure, too many local variables on the stack.

---

## Step 5: Syscall Tracing — OS-Level Hot Path

> Build release binary first: `cargo build --release`

**On Linux** (`strace` required):
```bash
which strace 2>/dev/null || echo "Install: sudo apt install strace"
dd if=/dev/urandom of=/tmp/waft_perf_test.bin bs=1M count=10 2>/dev/null
strace -c -e trace=read,write,sendfile,send,recv,mmap,brk \
  ./target/release/waft send 127.0.0.1 /tmp/waft_perf_test.bin 2>&1 | tail -20
```

**On macOS** (`fs_usage`, no SIP change required):
```bash
dd if=/dev/urandom of=/tmp/waft_perf_test.bin bs=1m count=10 2>/dev/null
sudo fs_usage -f filesys -w ./target/release/waft 2>&1 | grep -E "read|write|sendfile" | head -20
```

Analyze output for:
- Many small `write()` calls (< 4096 bytes each) → buffering broken.
- No `sendfile()` calls → data still copying through user-space (zero-copy not active).
- High `mmap()`/`brk()` calls → allocator active on the hot transfer loop (bad).

---

## Step 6: CPU Flamegraph — Visual Hot Path

Install `cargo-flamegraph` if missing, then generate a flamegraph while sending a test file:

```bash
cargo install flamegraph --quiet 2>/dev/null || true
dd if=/dev/urandom of=/tmp/waft_flamegraph_test.bin bs=1M count=10 2>/dev/null
cargo flamegraph --release --bin waft -o flamegraph.svg -- send 127.0.0.1:7777 /tmp/waft_flamegraph_test.bin 2>/dev/null
ls -lh flamegraph.svg 2>/dev/null && echo "flamegraph.svg generated — open in a browser"
```

If the binary requires a running daemon and cannot run standalone, skip and note it.

Interpret results:
- `blake3`, `tokio`, `std::io` dominating → IO-bound (optimal, nothing to do).
- `Vec::clone`, `String::from`, or custom serialization visible → unnecessary hot-path allocations.
- User-defined functions (`waft::*`) appearing wide → investigate and potentially inline.

---

## Step 7: Benchmark Scaffold Check

Check if benchmarks exist:

```bash
ls benches/ 2>/dev/null || echo "No benches/ directory — scaffold recommended"
```

If no `benches/` directory exists, offer to scaffold a minimal Criterion harness:

```bash
mkdir -p benches
cat > benches/bench_transfer.rs << 'EOF'
use criterion::{criterion_group, criterion_main, Criterion};

fn bench_file_transfer(c: &mut Criterion) {
    c.bench_function("transfer_1mb", |b| {
        b.iter(|| {
            // TODO: wire up send_file + start_receiver in loopback
        });
    });
}

criterion_group!(benches, bench_file_transfer);
criterion_main!(benches);
EOF
echo "Scaffold written to benches/bench_transfer.rs — add [[bench]] to Cargo.toml"
```

---

## Step 8: Consolidated Report

Present a final summary using this exact structure:

```markdown
# waft Performance Audit — <date>

## Platform
- OS: <uname output>
- Rust: <rustc --version output>

## ✅ Already Optimized
- (list what is good)

## 🔴 Critical Issues — Hot Path
| File | Line | Issue | Fix |
|---|---|---|---|

## 🟡 Moderate Issues — Worth Investigating
| File | Line | Issue | Estimated Impact |
|---|---|---|---|

## 🟢 Low Priority — Future Work
- (list)

## Recommended Next Actions (ordered by impact)
1. ...
2. ...
3. ...
```

Ask the user which findings they want fixed before ending the session.
