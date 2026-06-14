# Security Policy

## Reporting a Vulnerability

We take the security of `waft` seriously. If you find a security vulnerability, please do not report it in a public issue. Instead, please report it directly to the maintainer via email.

Please send security reports to the email listed in the repository Git configurations. We will investigate and respond to all reports as quickly as possible.

## Active Auditing

We employ local and cloud-based static application security testing (SAST) tools to catch vulnerabilities early:
* **Semgrep**: Scans for weak crypto, insecure socket binds, and standard logical vulnerabilities.
* **Cargo Audit**: Audits cargo dependency versions against the Rust Sec Advisory Database.
* **CodeQL**: Runs weekly in GitHub Actions to check compile paths.

We encourage contributors to run on-demand local security and performance audits before opening pull requests:
* **`/review`** — runs `cargo clippy`, `semgrep`, and interactive fix patches in your AI agent.
* **`/optimize`** — runs performance lints, binary size analysis, syscall tracing, and flamegraph profiling.

Both commands are available in compatible AI agents (Claude Code CLI, OpenCode CLI, Codex CLI, or Antigravity IDE).
