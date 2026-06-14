# Local AI Code Review Protocol & Architecture

This document specifies the design, UX flow, and architecture of the local AI code review system (`/review` command). It outlines a **hybrid model** that combines fast, deterministic static analysis tools (SAST) with local/free LLM-guided context analysis and interactive patch generation.

---

## 1. Goal & Philosophy

Relying entirely on a raw Large Language Model (LLM) to perform security scanning is unreliable; LLMs suffer from high false-positive rates, miss subtle structural data flows, and are expensive to scale on full codebases.

**Our Approach:**
1. **Deterministic Scan First**: Use high-fidelity local static security analysis and linter engines (`semgrep`, `cargo-audit`, `clippy`) to find exact security flaws, patterns, and warnings.
2. **Generative Synthesis Second**: Feed the tool diagnostics and AST-sliced surrounding code to the LLM. The LLM's job is to explain the vulnerability, weed out false positives, and output a clean, ready-to-apply diff patch.
3. **Local & Free**: Run entirely on a developer's machine using local inference models (e.g., Ollama running `qwen2.5-coder:7b`) or free cloud-tier APIs (e.g., Gemini 2.0 Flash) with prompt caching.

---

## 2. Developer UX Flow

### Command Invocation
The user executes the review locally:
* **As a Slash Command** inside an AI agent workspace (e.g. Claude Code / Antigravity):
  ```text
  /review [--staged | --branch <name> | --all]
  ```
* **As a CLI command**:
  ```bash
  waft review --staged
  ```

### Interactive Output
Instead of dumping raw JSON or lengthy logs, the command produces a structured, interactive list of issues grouped by severity:

```text
🤖 Waft AI Code Review — 2 files analyzed (300ms)

[SEC-01] Cryptographic Vulnerability in src/identity.rs (line 42)
Severity: 🔴 Critical (Verified by Semgrep rule: rust.security.weak-hash-md5)
Explanation: MD5 is used to compute identity fingerprints. It is vulnerable to collision attacks.

👉 Suggested Fix:
--------------------------------------------------------------------------------
-    let mut hasher = Md5::new();
+    let mut hasher = Sha256::new();
--------------------------------------------------------------------------------
[A] Apply Fix   [D] Discuss with AI   [I] Ignore Issue

[SEC-02] Insecure Multicast Bind in src/discovery.rs (line 120)
Severity: 🟡 Warning (Verified by Clippy / local check)
Explanation: Binding multicast socket directly to 0.0.0.0 on loopback might allow external traffic interference.

👉 Suggested Fix:
--------------------------------------------------------------------------------
-    let socket = UdpSocket::bind("0.0.0.0:7777")?;
+    let socket = UdpSocket::bind("127.0.0.1:7777")?;
--------------------------------------------------------------------------------
[A] Apply Fix   [D] Discuss with AI   [I] Ignore Issue
```

### Git hook integration
The engine can be run as a Git `pre-push` hook. If any `🔴 Critical` security warnings are outputted, the push is safely blocked until resolving them or adding an explicit ignore inline comment (`// waft-ignore: SEC-01`).

---

## 3. High-Speed Architecture

To maintain a sub-second response time before invoking the LLM, the scan operates in three stages:

### Stage A: Slicing the Git Diff
1. Find all files changed: `git diff --name-only` (or `--cached` for staged).
2. Filter out non-code assets (`.gitignore`, docs, assets).
3. If no code files changed, exit immediately.

### Stage B: Incremental & Parallel Scans
Rather than running scanning tools on the entire project, run them in parallel on the sliced files only:

```text
               ┌──► CLI / Agent / Hook
               │
      [Parallel Execution Thread Pool]
      ├──► Clippy: `cargo clippy -- <changed_files>`
      ├──► Semgrep: `semgrep scan <changed_files>`
      └──► Cargo Audit (Conditional): `cargo audit` (only run if Cargo.lock changed)
```

#### Cargo-Audit Caching
Checking all third-party dependencies takes time. Since dependencies only change when `Cargo.lock` changes:
1. Compute the SHA-256 hash of `Cargo.lock`.
2. Check if the hash matches the cached value in `~/.waft/audit_cache.txt`.
3. If it matches, bypass `cargo-audit` entirely.

### Stage C: AST Context Slicing (Semantic Slicing)
If a tool flags a line (e.g., `src/trust.rs:85`), we do not send the whole file to the LLM. 
1. Use `tree-sitter` or `ast-grep` to identify the boundary of the function or block containing line 85.
2. Slices only the function body + the signatures of adjacent structures.
3. This creates a tiny, highly-dense prompt (typically under 2KB), resulting in extremely fast LLM inference times.

---

## 4. LLM Prompt Compiler Specification

The prompt compiler combines the AST slice and the scanner JSON report into a unified structure.

### System Prompt Guidelines
```text
You are a senior security engineer and compiler expert.
Your job is to examine the provided compiler/security diagnostics and code context.
Determine:
1. If the warning is a false positive under the current implementation context.
2. How to fix it safely without breaking existing APIs or logic.

Output structure:
- Verdict: [Real Issue / False Positive]
- Explanation: [Short, clear explanation of why it matters]
- Unified Diff Patch: [The exact git diff patch to apply]
```

### Local vs. Cloud LLM Configuration
* **Ollama (Default Local)**: Runs `qwen2.5-coder:7b` (Q4_K_M quantization). Excellent local performance, zero cost, and code security since files never leave the machine.
* **Gemini Flash (Default Cloud)**: Runs via the official Google Developer API. Leverages prompt caching on repeated system/context inputs, providing ~500ms response times.

---

## 5. Security & Isolation

For security plugins or custom user lint scripts running locally:
* All execution of custom, unverified scripts (like dynamic rules) must occur inside restricted processes.
* Standard scans (`semgrep`, `cargo-audit`, `clippy`) run under standard user permissions directly in the local repository directory.
