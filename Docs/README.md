# Technical Documentation — soos Project

Welcome to the technical documentation for **soos** (local zero-trust facial verification PAM module for Linux).

---

## Documentation Index

| Document | Description | Target Audience |
|---|---|---|
| [**IPC Protocol Specification**](IPC_PROTOCOL.md) | Specification of binary protocol v1 between `pam_soos.so` and `soos-daemon` (types, bounds, codec, invariants). | PAM Developers, Daemon Developers |
| [**PAM Module Specification**](PAM_MODULE.md) | PAM module `pam_soos.so`, `pam-bindings` 0.3.0 architecture, latency budgets, and panic safety. | PAM Developers, Contributors |
| [**CI/CD & Security Auditing**](CI_CD_AND_SECURITY.md) | Multi-level quality gates, `cargo-deny` dependency audits, and Docker PAM sandbox tests. | All Contributors, DevOps |
| [**Development Workflow & Branching**](DEVELOPMENT_WORKFLOW.md) | Branch strategy, multi-agent TDD cycle, PR loop, and automated Copilot review. | Contributors, AI Agents |
| [**Conventional Commits Specification**](COMMIT_CONVENTION.md) | Standardized commit message format, allowed types and scopes, and hook validation. | Contributors, AI Agents |

---

## Architectural Reference Documents

For core security requirements, design decisions, and system constraints:
- [`AI/ARCHITECTURE.md`](../AI/ARCHITECTURE.md): Threat model, latency budgets, privilege separation, and non-negotiable security invariants.
- [`AI/DECISIONS.md`](../AI/DECISIONS.md): Architectural Decision Records (ADR) log.
- [`AI/MOCK_STRATEGY.md`](../AI/MOCK_STRATEGY.md): Hardware simulation and mock testing strategy.
- [`AI/ROLES_AND_WORKFLOW.md`](../AI/ROLES_AND_WORKFLOW.md): Multi-agent operational guidelines and responsibilities.
- [`AI/VERIFICATION_MATRIX.md`](../AI/VERIFICATION_MATRIX.md): Acceptance criteria and test coverage matrix per component.
- [`AI/walkthroughs/`](../AI/walkthroughs/): Sequential changelog and technical audit walkthroughs for every step.
