---
name: dev-workflow
description: >
  Multi-agent TDD development workflow for the soos project.
  Activate this skill when requested to implement a feature,
  fix a bug, or add a component. Enforces a strict 4-phase TDD
  cycle (Architect -> Tester -> Auditor -> Developer), English-only
  deliverables, and Conventional Commits.
---

# Multi-Agent Development Workflow — soos

## Scope

This workflow applies to **all Rust codebase modifications**:
- New features and crates
- Bug and vulnerability fixes
- Significant refactorings and behavioral adjustments

It does NOT apply to pure documentation fixes, shell script adjustments, or minor configuration tweaks.

## Prerequisites

Before starting, the agent MUST read:
1. `AI/ARCHITECTURE.md` — Master architecture, threat model, security invariants
2. `AI/DECISIONS.md` — Architectural decision records (ADRs)
3. `AI/VERIFICATION_MATRIX.md` — Acceptance criteria for the targeted component
4. `Docs/COMMIT_CONVENTION.md` — Conventional Commits 1.0.0 specification

## Strict Language Policy: English Only

Even if the user writes requests, prompts, or questions in French (or any other language), the AI agent MUST author ALL code, docstrings, inline comments, commit messages, PR titles/bodies, technical documentation in `Docs/`, and walkthroughs in `AI/walkthroughs/` strictly in English.

---

## Phase 0 — Dedicated Topic Branch

**Objective**: Isolate all work on a dedicated topic branch; NEVER commit directly to `main`.

1. Verify current branch (`git status`).
2. Create and switch to topic branch:
   - `git checkout -b feat/<name>` (e.g. `feat/ipc-client`)
   - `git checkout -b fix/<name>` (e.g. `fix/pam-timeout`)
   - `git checkout -b test/<name>` (e.g. `test/docker-pam`)
   - `git checkout -b chore/<name>` (e.g. `chore/ci-rules`)

---

## Phase 1 — Architect Agent

**Objective**: Specify and design before writing implementation.

1. Identify the target crate in the monorepo structure.
2. Specify structs, enums, traits, and error types.
3. Validate consistency with `AI/ARCHITECTURE.md`:
   - Are security invariants respected?
   - Is privilege separation preserved?
   - Are dependencies strictly unidirectional?
4. Document design choices in the implementation plan.

**Architect Checklist**:
- [ ] Target crate identified in workspace
- [ ] Types defined with bounded fields
- [ ] Traits and interfaces specified
- [ ] Security invariants verified
- [ ] Zero unapproved dependencies

---

## Phase 2 — Tester Agent

**Objective**: Write tests BEFORE production code (TDD Red Phase).

1. Write unit and property tests for public types and functions.
2. Tests MUST fail initially (compilation or assertion failure).
3. Cover both nominal and error paths (timeouts, malformed buffers, spoofed UIDs).
4. For PAM components: always include a test asserting `PAM_IGNORE` fallback.
5. For protocol components: include round-trip serialization and oversized payload rejection tests.

**Tester Checklist**:
- [ ] Unit tests authored for all public functions
- [ ] Error and edge cases covered
- [ ] `PAM_IGNORE` fallback test included for PAM paths
- [ ] Conforms to `AI/VERIFICATION_MATRIX.md`

---

## Phase 3 — Auditor Agent

**Objective**: Static security and safety review BEFORE implementation.

1. Ensure zero `unwrap()` or `expect()` in PAM production code.
2. Audit memory allocations and verify strict length bounds.
3. Enforce `#![forbid(unsafe_code)]` in business crates.
4. Verify `unsafe` is minimal, isolated, and documented in adapter crates.
5. Confirm zero sensitive data (passwords, embeddings, raw frames) is logged or exposed.
6. Verify `catch_unwind` wraps FFI boundaries.

**Auditor Checklist**:
- [ ] Zero `unwrap()` / `expect()` in PAM code
- [ ] Bounded allocations verified
- [ ] `#![forbid(unsafe_code)]` active in business crates
- [ ] Security invariants honored
- [ ] Zero sensitive data in logs or IPC schemas
- [ ] `catch_unwind` on all FFI entry points

---

## Phase 4 — Developer Agent

**Objective**: Implement production code satisfying test suite.

1. Write minimal code satisfying the Architect's specification and Auditor's constraints.
2. Verify all tests pass (TDD Green Phase).
3. Execute quality gates:
   - `cargo fmt --check`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo test --all-targets`
   - `cargo deny check`
4. Update `AI/VERIFICATION_MATRIX.md` criteria.

**Developer Checklist**:
- [ ] Implementation conforms to Architect specification
- [ ] All tests pass
- [ ] `cargo fmt` clean
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `AI/VERIFICATION_MATRIX.md` updated
- [ ] Technical documentation in `Docs/` updated
- [ ] Walkthrough written in `AI/walkthroughs/NN_<name>.md`

---

## Post-Implementation & Autonomous Pull Request Loop

Once development is complete:
1. Update technical documentation in `Docs/` in English.
2. Author walkthrough in `AI/walkthroughs/NN_<name>.md` in English.
3. Run the autonomous PR and auto-merge loop:
   ```bash
   ./save.sh --auto-merge
   ```
   This autonomous script:
   - Runs local quality gates (fmt, clippy, unit + invariant tests, cargo-deny).
   - Executes the independent **Candid Pre-Push Code Review** (`./scripts/candid_review.sh`) inspecting the raw diff without modification context for logic, security invariants, panic-safety, and English policy.
   - Validates the Conventional Commit message and scans for secrets.
   - Pushes branch to GitHub and opens a Pull Request.
   - Polls GitHub Actions CI jobs until completion.
   - Auto-merges to `main` upon green CI without requiring external review, then synchronizes local `main`.
