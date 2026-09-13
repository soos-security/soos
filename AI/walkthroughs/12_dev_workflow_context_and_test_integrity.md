# Walkthrough 12 — Dev-Workflow Context Mandate & Strict Test Integrity

> Date: 2026-09-13  
> Phase: AI Harness & Agent Workflow Hardening  

---

## Summary

In preparation for autonomous feature development driven by AI agents, this update reinforces `.agents/skills/dev-workflow/SKILL.md`, `AGENTS.md`, `AI/ROLES_AND_WORKFLOW.md`, and `Docs/DEVELOPMENT_WORKFLOW.md` with two critical non-negotiable invariants:
1. **Mandatory Full Context Ingestion**: Explicit requirement for agents to ingest all architectural and technical context across `AI/` and `Docs/` before designing or implementing code.
2. **Strict Test Integrity (Zero Test Weakening)**: Absolute prohibition against modifying, weakening, deleting, or bypassing existing tests when encountering an implementation block. The AI must persevere and fix the production code to satisfy the test contract.

---

## 1. Full Context Mandate (`AI/` and `Docs/`)

Before an agent writes any code or creates an implementation plan, it must consult:
- `AI/ARCHITECTURE.md` (System invariants, latency budgets, threat model)
- `AI/DECISIONS.md` (ADR register)
- `AI/BACKLOG.md` (Issue specifications, sub-issues, and acceptance criteria)
- `AI/VERIFICATION_MATRIX.md` (Acceptance criteria status)
- `AI/MOCK_STRATEGY.md` (Hardware-free simulation)
- `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` (Compiler profile hardening, workspace clippy restrictions, PAM constraints)
- `Docs/DEVELOPMENT_WORKFLOW.md` (Multi-agent TDD cycle, branching)
- `Docs/COMMIT_CONVENTION.md` (Conventional Commits 1.0.0)
- `Docs/IPC_PROTOCOL.md` (Framing and bounded serialization)

---

## 2. Strict Test Integrity Invariant

In TDD workflows, an AI agent might be tempted to alter an existing test assertion if its implementation fails. This is now strictly forbidden by policy and enforced across the workflow:
- **Immutable Contract**: Tests written in Phase 2 (Tester Agent) define the non-negotiable contract.
- **Perseverance Required**: If a test fails during Phase 4 (Developer Agent), the agent must debug and fix the production code until it satisfies the test contract.
- Modifying a test to make it artificially green is treated as an architectural invariant violation.

---

## 3. Simplified Invocation Model

With these guardrails merged, invoking the `dev-workflow` skill on any issue from `AI/BACKLOG.md` (e.g. `Issue #1: policy Crate`) triggers the complete autonomous lifecycle:
1. Identifies sub-issues, acceptance criteria, and topic branch from `AI/BACKLOG.md`.
2. Creates the dedicated topic branch (`feat/...`).
3. Executes the 4-phase TDD loop (Architect -> Tester -> Auditor -> Developer).
4. Updates documentation and writes the sequential walkthrough.
5. Executes `./save.sh --auto-merge` (quality gates, candid review, push, PR, CI monitoring, auto-merge to `main`).
