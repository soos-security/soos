# soos — Zero-Trust Local Facial Verification PAM Module for Linux

**soos** is an enterprise-grade Linux PAM (Pluggable Authentication Module) and privileged daemon designed for secure, local facial verification. It eliminates brittle camera access inside sensitive PAM processes by enforcing strict privilege separation, bounded binary IPC, and multi-stage anti-spoofing (PAD).

---

## Installation from Source

Supported hosts: Debian 12, Ubuntu 24.04, Fedora 40 / RHEL 9 and Arch Linux (x86_64, aarch64), with systemd.

```bash
# 1. Build dependencies (per-distribution lists: Docs/PACKAGING_AND_PROVISIONING.md §3.1)
./scripts/check_build_deps.sh --print-packages build     # e.g. then: sudo apt-get install ...
./scripts/check_build_deps.sh                            # read-only preflight
#    Rust toolchain: https://rustup.rs (rust-toolchain.toml selects the channel)

# 2. Build release artifacts and install (fails closed, rolls back on any error)
sudo ./scripts/install.sh --build                         # runs the preflight and the release build first
#    or, with artifacts already built by 'cargo build --release --locked --workspace':
sudo ./scripts/install.sh
```

`scripts/install.sh --dry-run` runs the same read-only preflight without touching the system. The installer
refuses debug-profile artifacts, exits non-zero when an artifact is missing, deploys and verifies the attested
models (`scripts/download_models.sh`, needs `curl`, no Python) before enabling `soos-daemon.service`, and undoes
every change it made if any step fails. PAM activation stays a separate, explicit step
(see [`Docs/DISTRIBUTION_DEPLOYMENT.md`](Docs/DISTRIBUTION_DEPLOYMENT.md)).

---

## Technical Documentation

Detailed guides and specifications are maintained in the [`Docs/`](Docs/) directory:

| Document | Description |
|---|---|
| [**IPC Protocol Specification**](Docs/IPC_PROTOCOL.md) | Bounded binary framing (Postcard over Unix domain socket), type definitions, and security bounds. |
| [**CI/CD & Security Auditing**](Docs/CI_CD_AND_SECURITY.md) | Quality gates, `cargo-deny` dependency audits, and Docker PAM sandbox testing. |
| [**Development Workflow & Branching**](Docs/DEVELOPMENT_WORKFLOW.md) | Topic branch rules, multi-agent TDD cycle, PR loop, and automated review. |
| [**Conventional Commits Specification**](Docs/COMMIT_CONVENTION.md) | Standardized commit message format and git hook enforcement rules. |

---

## Architecture & Security Reference

- [`AI/ARCHITECTURE.md`](AI/ARCHITECTURE.md): Threat model, latency budgets, privilege boundaries, and security invariants.
- [`AI/DECISIONS.md`](AI/DECISIONS.md): Architectural Decision Records (ADRs).
- [`AI/MOCK_STRATEGY.md`](AI/MOCK_STRATEGY.md): Hardware mocking and camera-free testing strategy.
- [`AI/ROLES_AND_WORKFLOW.md`](AI/ROLES_AND_WORKFLOW.md): Multi-agent operational rules and quality assurance.
- [`AI/VERIFICATION_MATRIX.md`](AI/VERIFICATION_MATRIX.md): Traceable component acceptance matrix.
- [`AI/walkthroughs/`](AI/walkthroughs/): Step-by-step verifiable implementation walkthroughs.
