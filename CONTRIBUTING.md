# Contributing to waft

Thank you for your interest in contributing to `waft`!

## Philosophy
`waft` values correctness over cleverness, clarity over abstraction, and minimal surface area over features. Before opening a pull request, please check the "not in scope" list in `docs/plan.md`. If your proposed change is on that list, open a discussion issue first.

## Quick Start
1. Clone the repository.
2. Run `cargo check` or `cargo build` to build the binary and automatically configure the local Git hooks path to `.githooks`.
3. Run tests:
   ```bash
   cargo test --all-features
   ```
   For the multicast discovery smoke test on a machine that supports multicast:
   ```bash
   cargo test --all-features --test test_discovery_multicast -- --ignored
   ```
4. Run Clippy:
   ```bash
   cargo clippy --all-targets --all-features -- -D warnings
   ```
5. Run formatting checks:
   ```bash
   cargo fmt --all --check
   ```

## Git Hook Automation
To maintain high quality, we enforce pre-commit checks:
- The `build.rs` script automatically runs `git config --local core.hooksPath .githooks` on build.
- This ensures that `cargo fmt` and `cargo clippy` run before every commit locally.

## AI Agent Skills via Slash Commands
We ship two built-in agent skills for contributors. If you use a compatible AI agent (e.g., Claude Code CLI, OpenCode CLI, Codex CLI, or Antigravity IDE), you can run these commands directly in your editor:

**`/review`** — Security & Code Quality Audit
```text
/review
```
Runs `cargo clippy`, checks/installs `semgrep` dynamically, scans the workspace for vulnerabilities or lints on your branch diff, and gives you interactive patch suggestions to apply immediately. **Run this before opening any pull request.**

**`/optimize`** — Performance Audit
```text
/optimize
```
Runs static performance lints (`clippy --perf`), binary size analysis, assembly inspection, syscall tracing, and CPU flamegraph generation. Auto-installs all required tools. Run this when you suspect a throughput regression or want to verify your change does not add allocations to the hot transfer path.

## Pull Requests
All pull requests must:
1. Pass all checks in CI (Linux, macOS, Windows).
2. Follow Conventional Commits format (e.g. `feat(transfer): add zero-copy path for Linux`).
3. Include tests verifying the change.

## Branch Flow
Do not work directly on `dev` or `main`.

Use this flow:
1. Branch from `dev` into a short-lived branch such as `feature/*`, `fix/*`, `chore/*`, or `dependabot/*`.
2. Open pull requests from the short-lived branch into `dev`.
3. Promote tested changes by opening a pull request from `dev` into `main`.

Repository enforcement:
- Direct pushes to `dev` and `main` are blocked by GitHub branch protection.
- Local Git hooks also block direct pushes to `origin/dev` and `origin/main`.
- `main` only accepts pull requests whose source branch is `dev`.
- If someone accidentally opens a feature or chore pull request against `main`, GitHub automatically retargets it to `dev`.

## Default Branch
The repository default branch is `main` for standard GitHub navigation and clearer release semantics.

Even with `main` as the default branch:
- day-to-day work still goes into `dev` first
- Dependabot still targets `dev`
- only `dev` should be promoted into `main`

## Checks And Merge Speed
Quality gates are intentionally front-loaded onto `dev`.

- Pull requests into `dev` run the full CI and security checks.
- Pull requests from `dev` into `main` use a lighter promotion gate.
- Administrators can bypass unfinished checks when fast promotion is necessary.

This means `dev` is the main integration-quality branch, while `main` is the release branch.
