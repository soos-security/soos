# Quality Pipeline, CI/CD, and Security Auditing

> Project: `soos`  
> Status: Operational  

---

## 1. Quality Gates Overview

The project enforces compliance across three concentric verification levels:

```
┌─────────────────────────────────────────────────────────────┐
│  Level 1: save.sh (Local — executed before every commit)    │
│  - cargo fmt --check                                        │
│  - cargo clippy --all-targets -- -D warnings                │
│  - cargo test --all-targets (unit & invariant tests)        │
│  - cargo deny check (licenses, advisories, bans)            │
│  - git hooks (Conventional Commits & secret scanner)        │
└──────────────────────────────┬──────────────────────────────┘
                               │ git push / Pull Request
                               ▼
┌─────────────────────────────────────────────────────────────┐
│  Level 2: GitHub Actions CI (.github/workflows/ci.yml)      │
│  - Job 1: Quality (fmt + clippy + test)                     │
│  - Job 2: Security (cargo-deny audit)                       │
│  - Job 3: PAM Integration (Docker sandbox)                  │
└──────────────────────────────┬──────────────────────────────┘
                               │ PAM sandbox validation
                               ▼
┌─────────────────────────────────────────────────────────────┐
│  Level 3: Docker PAM Integration Tests (run_tests.sh)       │
│  - Clean build of pam_soos.so in Ubuntu 24.04 container     │
│  - pamtester suite: T1 (nominal), T2 (reject), T3 (fallback)│
└─────────────────────────────────────────────────────────────┘
```

---

## 2. Local Automation Script: `save.sh`

The [`save.sh`](../save.sh) script is the primary tool for both human contributors and AI agents to validate, stage, and commit changes cleanly:
- **Sequential Execution**: Strict fail-fast pipeline (`set -euo pipefail`). Any failure immediately halts execution.
- **Conventional Commits Generation**: Generates standard Conventional Commit headers automatically in English based on the staged files (`feat(...)`, `test:`, `docs:`, `chore:`).
- **Push & PR Automation**: By default, `save.sh` performs local formatting, checking, testing, and commits. When invoked with `--auto-merge` (or via `scripts/pr_loop.sh`), it automates push, PR opening, Copilot review handling, and merge into `main`.

Usage:
```bash
# Local verification and commit with auto-generated message:
./save.sh

# Local verification and commit with custom message:
./save.sh "feat(policy): implement per-uid rate limiting"

# Full autonomous loop (push, PR creation, CI wait, Copilot review, auto-merge):
./save.sh --auto-merge
```

---

## 3. Dependency & Supply Chain Auditing: `cargo-deny`

The [`deny.toml`](../deny.toml) configuration enforces strict third-party dependency rules in alignment with our threat model and licensing requirements:

### Enforced Rules:
- **Advisories**: Any known unpatched RustSec advisory immediately fails the build.
- **Licenses**: Only permissive open-source licenses are permitted for dependencies (MIT, Apache-2.0, BSD-2/3, ISC). Workspace internal crates under AGPL-3.0 are isolated with `publish = false` and `[licenses.private] ignore = true`.
- **Sources**: Only the official `crates.io` index is permitted (no unverified git repositories or alternate registries).
- **Banned Crates**: Explicit prohibitions against forbidden libraries (notably `opencv`, strictly banned per [`AI/ARCHITECTURE.md`](../AI/ARCHITECTURE.md)).

To execute the security audit locally:
```bash
cargo deny check
```

---

## 4. Dockerized PAM Integration Testing: `run_tests.sh`

To guarantee that experimental PAM modules never compromise the host operating system, all PAM integration tests run inside an isolated, ephemeral Ubuntu 24.04 Docker container:
```bash
./run_tests.sh
```

This test harness executes:
1. **T1 (ABI & Nominal Path)**: Compiles and loads `pam_soos.so`, returns `PAM_IGNORE`, verifies authentication proceeds to `pam_unix`.
2. **T2 (Rejection Path)**: Verifies invalid credentials are consistently rejected.
3. **T3 (Fault Tolerance & Resilience)**: Verifies that an absent or failing `.so` module falls back gracefully without locking out system logins.

---

## 5. Pre-Commit Guardrails and Secret Scanning

Local Git hooks in `.githooks/` protect the repository:
- **Branch Protection**: Physically prevents direct commits to `main`.
- **Secret Scanning**: Prevents staging RSA/EC/SSH private keys, certificates, or GitHub personal access tokens.
- **Commit Format Validation**: Enforces the Conventional Commits 1.0.0 specification via `.githooks/commit-msg`.
