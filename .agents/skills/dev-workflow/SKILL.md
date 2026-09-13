---
name: dev-workflow
description: >
  Multi-agent TDD development workflow for the soos project.
  Activate this skill when requested to implement a feature,
  an issue from AI/BACKLOG.md, fix a bug, or add a component.
  Enforces full AI/ and Docs/ context ingestion, strict test integrity
  (zero test weakening), 4-phase TDD cycle (Architect -> Tester ->
  Auditor -> Developer), English-only deliverables, and autonomous merge.
---

# Multi-Agent Development Workflow — soos

## Scope

This workflow applies to **all Rust codebase modifications**:
- Implementing issues and sub-issues from `AI/BACKLOG.md`
- Creating new crates and components
- Bug and vulnerability fixes
- Refactoring and architectural adjustments

It does NOT apply to pure documentation fixes, shell script adjustments, or minor configuration tweaks.

---

## 1. Mandatory Context Ingestion (AI/ and Docs/ Mandate)

Before writing any code or architecture specifications, the agent **MUST** ingest and align with the complete project context documented in `AI/` and `Docs/`:

### Core Architectural Context (`AI/`)
1. `AI/ARCHITECTURE.md` — Master system architecture, threat model, security invariants, latency budget
2. `AI/DECISIONS.md` — Architectural Decision Records (ADRs) preventing hallucinations
3. `AI/BACKLOG.md` — Comprehensive backlog specifying issues, sub-issues, acceptance criteria, and branches
4. `AI/VERIFICATION_MATRIX.md` — Acceptance criteria and component verification matrix
5. `AI/MOCK_STRATEGY.md` — Hardware-free simulation and virtual testing strategy
6. `AI/ROLES_AND_WORKFLOW.md` — Multi-agent roles and quality committee guidelines

### Technical & Security Guidelines (`Docs/`)
1. `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` — Master security guidelines, compiler profile hardening, workspace clippy restrictions, and PAM execution constraints
2. `Docs/DEVELOPMENT_WORKFLOW.md` — Detailed development lifecycle, branch policies, and testing guidelines
3. `Docs/COMMIT_CONVENTION.md` — Conventional Commits 1.0.0 specification
4. `Docs/IPC_PROTOCOL.md` — Binary framing layout and IPC communication constraints
5. `Docs/CI_CD_AND_SECURITY.md` — Continuous integration pipeline and supply-chain auditing

---

## 2. Absolute Invariant: Test Integrity & Anti-Weakening Rule

> [!CAUTION]
> **STRICT TEST INTEGRITY (ZERO WEAKENING)**:
> Under **NO circumstances** is an AI agent permitted to modify, weaken, delete, or bypass an existing test to make it pass with broken, incomplete, or flawed production code.
> 
> - Tests authored in Phase 2 represent the immutable contractual specification derived from `AI/BACKLOG.md` and `AI/VERIFICATION_MATRIX.md`.
> - If a test fails or the AI encounters an implementation block: **The AI MUST PERSEVERE, debug, and fix the production implementation** until it cleanly satisfies the test suite.
> - Artificially modifying test expectations, deleting assertions, or silencing failures is considered an architectural invariant violation and a fatal defect.

---

## 3. Strict Language Policy: English Only

Even if the user writes requests, prompts, or questions in French (or any other language), the AI agent MUST author ALL code, docstrings, inline comments, commit messages, PR titles/bodies, technical documentation in `Docs/`, and walkthroughs in `AI/walkthroughs/` strictly in English.

---

## 4. Phase 0 — Dedicated Topic Branch

**Objective**: Isolate all work on a dedicated topic branch; NEVER commit directly to `main`.

1. Locate the targeted issue in `AI/BACKLOG.md` to identify the designated branch name.
2. Verify current branch status (`git status`).
3. Create and switch to topic branch:
   - `git checkout -b feat/<name>` (e.g. `feat/policy-crate`, `feat/daemon-skeleton`)
   - `git checkout -b fix/<name>` (e.g. `fix/pam-timeout`)
   - `git checkout -b test/<name>` (e.g. `test/fuzz-codec`)
   - `git checkout -b chore/<name>` (e.g. `chore/guardrails`)

---

## 5. Phase 1 — Architect Agent

**Objective**: Specify and design before writing implementation code.

1. Locate target crate in the workspace structure (`crates/<name>/`).
2. Specify structs, enums, traits, and error types.
3. Validate strict alignment with `AI/ARCHITECTURE.md` and `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`:
   - Are security invariants respected?
   - Is privilege separation preserved?
   - Are dependencies strictly unidirectional?
   - Are fields and allocations strictly bounded?
4. Document the technical design in the implementation plan.

**Architect Checklist**:
- [ ] Target crate identified in monorepo
- [ ] Bounded types and error enums specified
- [ ] Traits and interfaces declared
- [ ] Security invariants and latency budget verified
- [ ] Zero unapproved third-party dependencies

---

## 6. Phase 2 — Tester Agent (TDD Red Phase)

**Objective**: Write tests BEFORE production code.

1. Write unit, property, and invariant tests for public types and functions.
2. **Tests MUST fail initially** (compilation error or assertion failure).
3. Cover nominal paths, error paths, and edge cases (timeouts, malformed payloads, spoofed UIDs).
4. For PAM components: always include a test asserting fail-closed `PAM_IGNORE` fallback.
5. For protocol components: include round-trip serialization and oversized payload rejection tests.

**Tester Checklist**:
- [ ] Unit and property tests authored before implementation
- [ ] Tests initially fail (Red Phase verified)
- [ ] Error and boundary conditions covered
- [ ] PAM fallback asserted where applicable
- [ ] Acceptance criteria from `AI/VERIFICATION_MATRIX.md` implemented

---

## 7. Phase 3 — Auditor Agent (Security & Panic Safety)

**Objective**: Static security and safety review BEFORE production code is written.

1. Ensure zero `unwrap()` or `expect()` in PAM and library production code.
2. Verify `#![forbid(unsafe_code)]` in business crates (`protocol`, `policy`, `vision`).
3. For adapter crates (`pam`, `camera-v4l`): verify `unsafe` is minimal, isolated, and documented with `// SAFETY:` comments (`clippy::undocumented_unsafe_blocks`).
4. Ensure zero sensitive data (passwords, embeddings, raw frames) is logged or exposed in IPC schemas.
5. Verify `catch_unwind` wraps FFI boundaries.
6. Verify no stdout/stderr prints (`println!`, `eprintln!`, `dbg!`) exist in library or PAM code.

**Auditor Checklist**:
- [ ] Zero unwrap/expect in production pathways
- [ ] Bounded allocations and memory bounds verified
- [ ] `#![forbid(unsafe_code)]` active in business crates
- [ ] `// SAFETY:` rationale on all unsafe blocks
- [ ] Zero sensitive credentials in logs or schemas
- [ ] Output isolation verified (no println!/dbg! in PAM)

---

## 8. Phase 4 — Developer Agent (TDD Green Phase)

**Objective**: Implement production code satisfying test suite.

1. Write the minimal production code satisfying the Architect specification and Auditor constraints.
2. **Verify all tests pass (TDD Green Phase) WITHOUT modifying or weakening any test.**
3. If tests fail: analyze root cause, debug, and fix the production code. Persevere until all tests pass cleanly.
4. Execute workspace quality gates:
   - `cargo fmt --check`
   - `cargo clippy --all-targets --all-features -- -D warnings`
   - `cargo test --all-targets`
   - `cargo deny check`
5. Update `AI/VERIFICATION_MATRIX.md` criteria status.

**Developer Checklist**:
- [ ] Minimal production code implemented
- [ ] All pre-written tests pass cleanly without modifications
- [ ] `cargo fmt` clean
- [ ] `cargo clippy` clean (zero warnings with `-D warnings`)
- [ ] `cargo test` clean (all unit and invariant suites pass)
- [ ] `cargo deny check` clean
- [ ] `AI/VERIFICATION_MATRIX.md` updated

---

## 9. Phase 5 — Candid Reviewer Sub-Agent (Fresh Reasoning Audit)

**Objective**: Impartial, adversarial code review of the raw diff with fresh, unpolluted context.

1. Activate the `candid-reviewer` skill (`.agents/skills/candid-reviewer/SKILL.md`).
2. Inspect the raw diff (`git diff origin/main...HEAD` or `target/candid_diff.patch`).
3. Evaluate against the 5 critical pillars:
   - **Logic & Architecture**: State transitions, edge cases, bounds.
   - **PAM Concurrency**: Zero Tokio in PAM, synchronous 200–250ms deadline, zero stdout/stderr prints.
   - **Panic Safety**: All FFI wrapped in `catch_unwind`, zero unwraps/panics in prod, fail-closed `PAM_IGNORE`.
   - **Test Integrity**: Zero test weakening; pre-existing test contracts preserved.
   - **Memory & Secrets**: Bounded allocations (4096 bytes), zeroization, zero passwords/frames in schemas.
4. Author the formal review report in `AI/candid_review_report.md` with an explicit `VERDICT: APPROVED`.
5. If findings require changes (`VERDICT: CHANGES_REQUESTED`), return to Phase 4 (Developer Agent) to resolve them before proceeding.

---

## 10. Autonomous PR, Review & Auto-Merge Loop

Once development and Candid Sub-Agent review are complete:
1. Author or update technical documentation in `Docs/` in professional English.
2. Author sequential walkthrough in `AI/walkthroughs/NN_<name>.md` in professional English.
3. Execute the autonomous loop:
   ```bash
   ./save.sh --auto-merge
   ```
   This autonomous pipeline:
   - Formats code (`cargo fmt`).
   - Runs workspace Clippy (`cargo clippy --all-targets --all-features -- -D warnings`).
   - Executes all unit and architectural invariant test suites (`cargo test --all-targets`).
   - Audits supply chain dependencies and licenses (`cargo deny check`).
   - Executes the dual-layer **Candid Pre-Push Code Review** (`./scripts/candid_subagent.sh`) combining deterministic invariant checks and the AI Sub-Agent reasoning report.
   - Pushes topic branch to GitHub and opens a Pull Request.
   - Monitors GitHub Actions CI checks (Quality, Security, PAM Integration).
   - Auto-merges into `main` via squash merge upon green CI, and synchronizes local `main`.
