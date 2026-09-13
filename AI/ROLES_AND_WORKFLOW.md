# Multi-Agent Development Guidelines and Quality Workflow

This document defines the strict operational rules and committee structure for AI agents and human engineers contributing to the **soos** project.

---

## 1. Multi-Agent Committee Workflow (Strict TDD)

For every code feature or modification, development is executed in 4 sequential phases:

1. **Architect Agent:**
   - Defines structs, enums, traits, and error types.
   - Verifies adherence to `AI/ARCHITECTURE.md` invariants and latency budgets.
   - Ensures unidirectionality of dependencies and absence of unapproved crates.

2. **Tester Agent (TDD Red Phase):**
   - Authors comprehensive unit, property, and integration tests BEFORE any business implementation is written.
   - All tests MUST fail initially (red phase).
   - Any code touching the PAM pathway must include a dedicated test asserting fail-closed `PAM_IGNORE` fallback.

3. **Auditor Agent (Security & Compliance):**
   - Audits code against `unwrap()` and `expect()` in PAM paths.
   - Verifies `#![forbid(unsafe_code)]` in business crates.
   - Audits memory safety, bounded allocations, and zeroization of sensitive buffers.
   - Ensures zero passwords, frames, or embeddings are exposed in logs or IPC schemas.

4. **Developer Agent (TDD Green Phase):**
   - Implements the minimal production code necessary to turn tests green.
   - Ensures strict compliance with formatting (`cargo fmt`), linting (`cargo clippy -- -D warnings`), and dependency audits (`cargo deny check`).

---

## 2. Test-Driven Development (TDD) Invariants

- Never produce business logic without pre-existing automated tests.
- Always verify that critical security invariants in `tests/invariants/` pass continuously.

---

## 3. Automation Tooling & Quality Scripts

- **`save.sh`:** Local quality pipeline and commit automation script (`cargo fmt`, `clippy -D warnings`, `test`, `deny check`, Conventional Commits).
- **`scripts/pr_loop.sh`:** Autonomous loop orchestrating branch push, PR creation, CI monitoring, GitHub Copilot review integration, and auto-merge to `main`.
- **`run_tests.sh`:** Isolated ephemeral Docker container executing `pamtester` validation without risking host lockout.

---

## 4. Compiler & Lint Error Handling

When given compiler or borrow-checker errors (e.g. `cargo check` output), analyze the issue silently and provide the complete, corrected code directly without verbose explanations.
