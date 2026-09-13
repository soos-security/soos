---
name: dev-workflow
description: >
  Master Multi-Agent Orchestrator for the soos project.
  Activate this skill with an issue ID from AI/BACKLOG.md to autonomously
  execute the full end-to-end development lifecycle through its specialized
  sub-agents: Architect, Tester, Auditor, Developer, Candid Reviewer,
  and Traceability Sub-Agents, concluding with automated merge to main.
---

# Master Multi-Agent Development Workflow — soos

## Scope & Purpose

`dev-workflow` is the **Master Orchestration Skill** for all code development in the `soos` workspace.
When invoked with a target issue (e.g. `Issue #1: policy Crate`), this skill acts as the conductor that sequentially and autonomously coordinates the 6 specialized sub-agents:

```
                  ┌─────────────────────────────────────────────────────────────┐
                  │                    USER REQUEST                             │
                  │   "Use dev-workflow for Issue #N: <component>"              │
                  └──────────────────────────────┬──────────────────────────────┘
                                                 │
                                                 ▼
                  ┌─────────────────────────────────────────────────────────────┐
                  │ Phase 0: Topic Branch Isolation                             │
                  │ (git checkout -b <type>/<name> from AI/BACKLOG.md)          │
                  └──────────────────────────────┬──────────────────────────────┘
                                                 │
                                                 ▼
                   ┌─────────────────────────────────────────────────────────────┐
                   │ Phase 1: Architect Sub-Agent (.agents/skills/architect-agent)│
                   │ Scaffolds crate, specifies bounded types, traits, errors    │
                   └──────────────────────────────┬──────────────────────────────┘
                                                  │
                                                  ▼
                   ┌─────────────────────────────────────────────────────────────┐
                   │ Phase 1.5: Plan Evaluator Sub-Agent (plan-evaluator)        │
                   │ Audits plan vs AI/ARCHITECTURE.md across 6 pillars          │
                   │ Self-validates plan (VALIDATION_VERDICT: APPROVED)          │
                   └──────────────────────────────┬──────────────────────────────┘
                                                  │
                                                  ▼
                  ┌─────────────────────────────────────────────────────────────┐
                  │ Phase 2: Tester Sub-Agent (.agents/skills/tester-agent)     │
                  │ Authors unit/property tests, asserts PAM fallback (RED)     │
                  │ *Strict Test Integrity: tests are an immutable contract*    │
                  └──────────────────────────────┬──────────────────────────────┘
                                                 │
                                                 ▼
                  ┌─────────────────────────────────────────────────────────────┐
                  │ Phase 3: Auditor Sub-Agent (.agents/skills/auditor-agent)   │
                  │ Audits panics, unsafe, stdout pollution, secret leakage     │
                  └──────────────────────────────┬──────────────────────────────┘
                                                 │
                                                 ▼
                  ┌─────────────────────────────────────────────────────────────┐
                  │ Phase 4: Developer Sub-Agent (.agents/skills/developer-agent)│
                  │ Implements minimal production code until tests pass (GREEN) │
                  │ *Zero Test Weakening: strictly fixes production code only*  │
                  └──────────────────────────────┬──────────────────────────────┘
                                                 │
                                                 ▼
                  ┌─────────────────────────────────────────────────────────────┐
                  │ Phase 5: Candid Reviewer Sub-Agent (candid-reviewer)        │
                  │ Impartial cold diff audit on 5 pillars                      │
                  │ Generates AI/candid_review_report.md (VERDICT: APPROVED)    │
                  └──────────────────────────────┬──────────────────────────────┘
                                                 │
                                                 ▼
                  ┌─────────────────────────────────────────────────────────────┐
                  │ Phase 6: Traceability Sub-Agent (traceability-agent)        │
                  │ Updates AI/VERIFICATION_MATRIX.md, Docs/, Walkthrough NN    │
                  └──────────────────────────────┬──────────────────────────────┘
                                                 │
                                                 ▼
                  ┌─────────────────────────────────────────────────────────────┐
                  │ Phase 7: Autonomous Merge Loop (./save.sh --auto-merge)     │
                  │ Quality gates, candid check, push, PR, CI wait, auto-merge  │
                  └─────────────────────────────────────────────────────────────┘
```

---

## 1. Single Invocation Model

The user **only needs to invoke `dev-workflow` with an Issue ID**:
> Example: *"Use the `dev-workflow` skill to implement Issue #1: policy Crate — Authorization Logic"*

The master workflow automatically handles the rest through its sub-agent ensemble.

---

## 2. Mandatory Context Ingestion

Before starting Phase 1, the orchestrator verifies full ingestion of:
- **`AI/`**: `ARCHITECTURE.md`, `DECISIONS.md`, `BACKLOG.md`, `VERIFICATION_MATRIX.md`, `MOCK_STRATEGY.md`, `ROLES_AND_WORKFLOW.md`
- **`Docs/`**: `SECURITY_AND_QUALITY_GUIDELINES.md`, `DEVELOPMENT_WORKFLOW.md`, `COMMIT_CONVENTION.md`, `IPC_PROTOCOL.md`, `CI_CD_AND_SECURITY.md`

---

## 3. Sub-Agent Execution Pipeline

### Phase 0: Topic Branch Isolation
- **Issue Disambiguation**: If an issue number could refer to either a Backlog Issue or a GitHub Issue (e.g. user passes `#12`), consult `scripts/sync_issue.py` (`BACKLOG_TO_GITHUB` / `BRANCH_TO_ISSUE`) to resolve the canonical Backlog issue and topic branch.
- Lookup target branch in `AI/BACKLOG.md` (e.g. `feat/camera-v4l`).
- Create and switch: `git checkout -b <type>/<name>`.
- **Sandbox Execution**:
  - `git checkout -b <type>/<name>` modifies `.git` and must be executed with `BypassSandbox: true` if `.git` is read-only in the sandbox.
  - If adding new external dependencies to `Cargo.toml`, run a single `cargo fetch` with `BypassSandbox: true` to populate the Cargo cache, then resume sandboxed compilation and testing.
  - Release steps (`./save.sh --push-pr`, `./scripts/pr_loop.sh`) require `BypassSandbox: true` to communicate with GitHub.

### Phase 1: Architect Sub-Agent ([architect-agent](file:///home/hadrien/soos/.agents/skills/architect-agent/SKILL.md))
- Scaffolds crate in `crates/<name>/` and registers in root `Cargo.toml`.
- Inherits workspace lints: `[lints] workspace = true`.
- Specifies bounded structs, enums, and `thiserror` error types.
- Asserts `#![forbid(unsafe_code)]` in business crates.

### Phase 1.5: Plan Evaluator Sub-Agent ([plan-evaluator](file:///home/hadrien/soos/.agents/skills/plan-evaluator/SKILL.md))
- Audits implementation plans and technical specifications against `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/BACKLOG.md`, and `AI/VERIFICATION_MATRIX.md`.
- Evaluates across 6 core pillars: Architectural Alignment, PAM Real-Time Deadlines, Panic Safety, Dependency Isolation, Memory/Secret Hygiene, and Test Integrity.
- Validates the implementation plan autonomously (`VALIDATION_VERDICT: APPROVED`) before execution proceeds.

### Phase 2: Tester Sub-Agent ([tester-agent](file:///home/hadrien/soos/.agents/skills/tester-agent/SKILL.md))
- Authors unit, property (`proptest`), and invariant tests.
- Verifies tests **FAIL** initially (TDD Red Phase).
- Systematically writes tests asserting fail-closed `PAM_IGNORE` for PAM pathways.
- **Contractual Invariant**: Tests written in this phase are immutable. They define the non-negotiable contract of acceptance.

### Phase 3: Auditor Sub-Agent ([auditor-agent](file:///home/hadrien/soos/.agents/skills/auditor-agent/SKILL.md))
- Audits interfaces for panic safety (zero unwrap/expect in production code).
- Enforces output isolation (zero `println!` or `dbg!` in PAM).
- Verifies `catch_unwind` on all FFI entry points.
- Validates memory bounds and zeroization of sensitive buffers.

### Phase 4: Developer Sub-Agent ([developer-agent](file:///home/hadrien/soos/.agents/skills/developer-agent/SKILL.md))
- Implements minimal production code satisfying pre-written tests.
- Iterates until 100% of tests pass cleanly (TDD Green Phase).
- **Strict Anti-Weakening Rule**: Under NO circumstances may the developer modify, weaken, or delete a test. The developer must persevere and fix production code only.
- Formats code (`cargo fmt`) and ensures zero Clippy warnings (`cargo clippy --all-targets --all-features -- -D warnings`).

### Phase 5: Candid Reviewer Sub-Agent ([candid-reviewer](file:///home/hadrien/soos/.agents/skills/candid-reviewer/SKILL.md))
- Executes independent, cold diff review against `origin/main` on 5 pillars:
  1. Logic & Architecture
  2. PAM Concurrency & Real-Time Deadlines
  3. Panic Safety & Fallback
  4. Test Integrity & Anti-Weakening
  5. Memory & Secret Bounds
- Authors formal report in `AI/candid_review_report.md` with `VERDICT: APPROVED`.

### Phase 6: Traceability Sub-Agent ([traceability-agent](file:///home/hadrien/soos/.agents/skills/traceability-agent/SKILL.md))
- Synchronizes issues and sub-issues automatically via `python3 scripts/sync_issue.py`:
  - Checks off completed sub-issues (`- [x] **#X.Y**`) in `AI/BACKLOG.md`.
  - Checks off sub-issues directly in the corresponding GitHub Issue body on `github.com`.
  - Posts automated progress comments on the GitHub Issue.
- Updates `AI/VERIFICATION_MATRIX.md` with verified status and test evidence.
- Updates technical documentation in `Docs/` in professional English.
- Authors sequential walkthrough `AI/walkthroughs/NN_<name>.md`.

### Phase 7: Autonomous Release Loop
- Executes:
  ```bash
  ./save.sh --auto-merge
  ```
- Runs local quality gates + dual-layer candid review (`scripts/candid_subagent.sh`).
- Pushes topic branch and opens GitHub Pull Request.
- Monitors GitHub Actions CI checks until 100% green.
- Auto-merges into `main` via squash merge and synchronizes local `main`.
