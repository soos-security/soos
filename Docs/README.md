# Technical Documentation — soos Project

Welcome to the technical documentation for **soos** (local zero-trust facial verification PAM module for Linux).

---

## Documentation Index

| Document | Description | Target Audience |
|---|---|---|
| [**IPC Protocol Specification**](IPC_PROTOCOL.md) | Specification of binary protocol v1 between `pam_soos.so` and `soos-daemon` (types, bounds, codec, invariants). | PAM Developers, Daemon Developers |
| [**PAM Module Specification**](PAM_MODULE.md) | PAM module `pam_soos.so`, `pam-bindings` 0.3.0 architecture, latency budgets, and panic safety. | PAM Developers, Contributors |
| [**Daemon Reference**](DAEMON.md) | `soos-daemon` configuration (`daemon.toml` keys and defaults), runtime behaviour and status/preview requests. | Administrators, Daemon Developers |
| [**CI/CD & Security Auditing**](CI_CD_AND_SECURITY.md) | Multi-level quality gates, `cargo-deny` dependency audits, and Docker PAM sandbox tests. | All Contributors, DevOps |
| [**Development Workflow & Branching**](DEVELOPMENT_WORKFLOW.md) | Branch strategy, multi-agent TDD cycle, PR loop, and automated Copilot review. | Contributors, AI Agents |
| [**Conventional Commits Specification**](COMMIT_CONVENTION.md) | Standardized commit message format, allowed types and scopes, and hook validation. | Contributors, AI Agents |
| [**GUI Application**](GUI_APPLICATION.md) | `soos-gui` threading model (background daemon polling, off-UI-thread `pkexec`), camera error banners, stderr logging and the brand visual design system (palette, header, page layouts). | GUI Developers, Support |
| [**Security & Code Quality Guidelines**](SECURITY_AND_QUALITY_GUIDELINES.md) | Compiler profiles, workspace lints, panic safety and secure coding rules. | All Contributors, AI Agents |
| [**PAM Docker Test Matrix**](PAM_DOCKER_TEST_MATRIX.md) | Dockerized PAM matrix T1–T15 (`tests/docker/test_suite.sh`, `pam_test_runner`) and its expected outcomes. | PAM Developers, DevOps |
| [**Policy Crate**](POLICY_CRATE.md) | `soos-policy` zero-I/O authorization, rate limiting and PAD consensus logic. | Daemon Developers |
| [**Camera V4L2 Crate**](CAMERA_V4L_CRATE.md) | `soos-camera-v4l` MMAP capture manager, format negotiation and `mock-camera` feature. | Daemon Developers |
| [**Vision Crate**](VISION_CRATE.md) | `soos-vision` preprocessing, alignment, PAD wiring and cosine matching. | Vision Developers |
| [**Inference ORT Crate**](INFERENCE_ORT_CRATE.md) | `soos-inference-ort` isolated ONNX Runtime CPU sessions and manifest shape validation. | Vision Developers |
| [**Biometric Store Crate**](BIOMETRIC_STORE_CRATE.md) | `soos-biometric-store` encrypted embeddings at rest. | Daemon Developers, Security Reviewers |
| [**Evidence Store Crate**](EVIDENCE_STORE_CRATE.md) | `soos-evidence-store` opt-in encrypted intrusion snapshots and retention. | Daemon Developers, Security Reviewers |
| [**Enrollment CLI**](ENROLLMENT_CLI.md) | `soos-enroll` root enrollment and diagnostics tool. | Operators, Contributors |
| [**Memory Protection & Swap Hardening**](MEMORY_PROTECTION_AND_SWAP.md) | `mlock` page pinning and zeroization of keys, embeddings and frames. | Security Reviewers |
| [**Packaging & Provisioning**](PACKAGING_AND_PROVISIONING.md) | Build dependencies, installer, packages, runtime paths and modes. | Packagers, Operators |
| [**Distribution Deployment Guide**](DISTRIBUTION_DEPLOYMENT.md) | Per-distribution PAM activation, validation and rollback. | Operators, Packagers |
| [**Remote Companion**](REMOTE_COMPANION.md) | `soos-remote` user-level lock status page, remote lock and opt-in remote unlock over Tailscale Serve: trust model, accepted unlock risk, `LockedHint` requirements, per-user install, out-of-scope list. | Operators, Security Reviewers |

---

## Architectural Reference Documents

For core security requirements, design decisions, and system constraints:
- [`AI/ARCHITECTURE.md`](../AI/ARCHITECTURE.md): Threat model, latency budgets, privilege separation, and non-negotiable security invariants.
- [`AI/DECISIONS.md`](../AI/DECISIONS.md): Architectural Decision Records (ADR) log.
- [`AI/MOCK_STRATEGY.md`](../AI/MOCK_STRATEGY.md): Hardware simulation and mock testing strategy.
- [`AI/ROLES_AND_WORKFLOW.md`](../AI/ROLES_AND_WORKFLOW.md): Multi-agent operational guidelines and responsibilities.
- [`AI/VERIFICATION_MATRIX.md`](../AI/VERIFICATION_MATRIX.md): Acceptance criteria and test coverage matrix per component.
- [`AI/walkthroughs/`](../AI/walkthroughs/): Sequential changelog and technical audit walkthroughs for every step.
