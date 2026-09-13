# Walkthrough — English Standardization & Conventional Commits Specification

> Date: 2026-09-13  
> Phase: Professionalization — English-Only Policy & Commit Convention  

---

## Summary

Elevated the repository to enterprise-grade open-source standards by transitioning all assets to professional English, instituting the Conventional Commits 1.0.0 specification with automated git hook enforcement, and establishing a strict AI language decoupling policy.

---

## Deliverables & Key Changes

### 1. English-Only Language Policy & AI Decoupling
- **Decoupled User Communication**: Mandated in `AGENTS.md` and `.agents/skills/dev-workflow/SKILL.md` that all repository deliverables (source code, comments, docstrings, docs, commits, PRs, walkthroughs, logs) MUST be strictly in English, even when user prompts are in French.
- **Renamed Documents**:
  - `Docs/CI_CD_ET_SECURITE.md` -> `Docs/CI_CD_AND_SECURITY.md`
  - `Docs/CYCLE_DE_DEVELOPPEMENT.md` -> `Docs/DEVELOPMENT_WORKFLOW.md`
  - `Docs/PROTOCOLE_IPC.md` -> `Docs/IPC_PROTOCOL.md`
  - `AI/ROLES_ET_WORKFLOW.md` -> `AI/ROLES_AND_WORKFLOW.md`
- **Translated Core Documentation**:
  - `README.md`, `Docs/README.md`
  - `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/MOCK_STRATEGY.md`, `AI/VERIFICATION_MATRIX.md`
  - All historical walkthroughs (`01_...` through `07_...`)
  - Source code comments and docstrings across `crates/protocol`, `crates/pam`, and `tests/invariants`

### 2. Conventional Commits 1.0.0 & Automated Validation
- **Specification**: Authored [`Docs/COMMIT_CONVENTION.md`](file:///home/hadrien/soos/Docs/COMMIT_CONVENTION.md) detailing allowed types, canonical scopes, imperative mood formatting, and body/footer conventions.
- **Git Hook (`.githooks/commit-msg`)**: Created automated hook validating commit headers against conventional regex, enforcing subject length (<= 80 chars), and blocking non-compliant commit attempts with actionable guidance.
- **Pre-Commit Hook (`.githooks/pre-commit`)**: Converted to English, preventing commits directly on `main` and filtering against private key/token leaks.
- **Quality Script Integration**: Updated `save.sh` to generate English Conventional Commit messages by default and enforce commit-msg validation.
- **Autonomous PR Loop**: Updated `scripts/pr_loop.sh` logs, messages, and PR descriptions to English.

---

## Verification Results

- `cargo test --all-targets`: 25 passed.
- `cargo fmt --check`: Clean format.
- `cargo clippy --all-targets -- -D warnings`: 0 warnings.
- `cargo deny check`: 0 advisories, all licenses approved.
- `.githooks/commit-msg`: Successfully rejects invalid commit headers and validates compliant conventional commit formats.
