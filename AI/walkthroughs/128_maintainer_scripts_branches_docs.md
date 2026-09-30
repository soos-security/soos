# Walkthrough 128 — Maintainer Scripts, Branch Prefixes and Documentation Drift

- **Date**: 2026-09-30
- **Issues**: GitHub #238 (TCI-07), #239 (TCI-08), #240 (TCI-09)
- **Branch**: `chore/p2-maintainer-scripts-branches-docs`
- **Matrix criteria**: MSB1–MSB8 (new)
- **ADR**: 2026-09-30 "Branch Prefixes and Root-Level Scratch Files"
- **Scope**: repository hygiene, `scripts/sync_issue.py --check`, Docker image package lists,
  `run_tests.sh` header and constants, documentation. No Rust production code changes.

---

## 1. Findings

| Issue | Drift |
|---|---|
| #238 | `create_issues.py`, `fix_main.py`, `patch_sync.py` and `temp_body.md` were tracked at the repository root. `create_issues.py` hard-coded `/home/<user>/...` paths; `fix_main.py` rewrote `crates/daemon/src/main.rs` with symbols removed in Issue #38. The `sync_issue.py` fallback cited by the review had already been replaced by `shutil.which("gh")`. |
| #239 | `BRANCH_TO_ISSUE` registered `refactor/*` and `docs/*` branches that `AGENTS.md` does not allow; the workspace trees of `AGENTS.md` and `AI/ARCHITECTURE.md` omitted `crates/gui` and listed a non-existent `tests/integration-pam`; `AI/ARCHITECTURE.md` still listed a `nokhwa` "verified version", two "Planned Target" paragraphs and a three-crate `forbid(unsafe_code)` list. |
| #240 | The Docker images installed `pamtester` although the Docker matrix uses `tests/docker/pam_test_runner.c`; `run_tests.sh` quoted T1–T12 (the suite runs T1–T15) and declared two unused `readonly` constants; `Docs/README.md` indexed 6 of 18 pages; `AI/MOCK_STRATEGY.md` described JPEG / tensor fixtures that do not exist; `Docs/MEMORY_PROTECTION_AND_SWAP.md` said "128D/512D". The class-0 ADR had already been marked superseded (2026-09-29). |

## 2. Decision

Branch prefixes stay `feat/`, `fix/`, `test/`, `chore/` (see the ADR). The five merged historical
branches are kept in `BRANCH_TO_ISSUE` as the frozen `LEGACY_BRANCHES` set, because `--auto` maps
a branch name to its backlog issue and those names exist in the history. `pamtester` is removed
only from the Docker PAM images: `tests/docker/authselect_profile_test.sh` and
`tests/docker/pam_rollback_test.sh` install it on demand in stock images, and
`tests/physical/pam_integration_test.sh` keeps it as a fallback runner.

## 3. Tests first (red evidence)

New module `tests/invariants/src/maintainer_hygiene_contract.rs` (15 tests). Before the change:

```
cargo test --locked -p soos-invariants --all-features maintainer_hygiene
test result: FAILED. 3 passed; 12 failed
```

The three tests that passed before the change guard facts that were already true:
`test_sync_issue_resolves_gh_through_path_only` (the gh fallback was already removed),
`test_sync_issue_check_accepts_frozen_legacy_branch` (the legacy branches were accepted) and
`test_personal_home_path_detector_matches_literal_home_dirs` (the detector's self-test).

## 4. Changes

- Deleted the four root-level one-off files; `.gitignore` now ignores `/*.py` and `/temp_*.md`.
- `scripts/sync_issue.py`: `ALLOWED_BRANCH_PREFIXES`, frozen `LEGACY_BRANCHES`, and a
  `check_mappings` error "branch '<name>' uses a non-approved prefix".
- `AGENTS.md`, `Docs/DEVELOPMENT_WORKFLOW.md`, `project-facts.md`: the prefix rule, the legacy
  branch list, the full workspace tree (with `gui/` and the real `tests/` directories), and the
  `forbid(unsafe_code)` list pointing to `test_business_crates_forbid_unsafe_code`.
- `AI/ARCHITECTURE.md`: crate versions without `nokhwa` (now "banned by `deny.toml`"), the PAM ABI
  and memory-hygiene paragraphs marked implemented (matrix PA11, H1, VZF1–VZF3), the corrected
  tree, the 9-crate forbid list, and the Docker PAM matrix phase without `pamtester`.
- `Dockerfile`, `tests/docker/Dockerfile.ubuntu`: `pamtester` removed from the package list.
- `run_tests.sh`: header states T1–T15 through `pam_test_runner`; the unused
  `PAM_MODULE_NAME` / `PAM_MODULES_DIR` constants are removed.
- `AI/ROLES_AND_WORKFLOW.md`, `Docs/PAM_DOCKER_TEST_MATRIX.md` (T10 note), `Docs/README.md`
  (index of all 18 pages), `AI/MOCK_STRATEGY.md` (in-code fixtures), and
  `Docs/MEMORY_PROTECTION_AND_SWAP.md` (512D).

## 5. Verification

- `cargo test --locked -p soos-invariants --all-features`: 129 passed, 0 failed.
- `python3 scripts/sync_issue.py --check`: OK (55 branches).
- `bash -n run_tests.sh`.
- Full gate: see §6.

## 6. Gate

`cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`,
`cargo test --locked --workspace --all-targets --all-features --no-fail-fast` and
`./scripts/candid_review.sh` were run on the final tree before the commit: all green (the
workspace test run reported no failure; `soos-invariants` 129 passed).

## 7. Follow-ups

- Rebuild the Docker PAM images (`./run_tests.sh`, `./tests/docker/run_matrix.sh all`) in CI to
  confirm that removing `pamtester` changes nothing (no case of `test_suite.sh` invokes it).
- `tests/fixtures/mod.rs` still documents its synthetic embeddings as "128D"; they are logic
  fixtures, not model outputs, and were left untouched by this change.
