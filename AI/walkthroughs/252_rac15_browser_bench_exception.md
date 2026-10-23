# Walkthrough 252 — Narrow RAC15 Exception for the Playwright Bench

- **Date**: 2026-10-23
- **Issue**: follow-up of GitHub #464 (PR 2 of #355); open question of walkthrough 249 §6
  **Branch**: `chore/rac15-bench-exception`
- **Decision**: owner decision of 2026-10-23 — "add `scripts/remote_browser_tests.sh` to the RAC15
  named exceptions (additive migration approved)". ADR 2026-10-13 "README Assets Are Generated From
  Synthetic Data, Never Hand-Edited" and its Playwright amendment are otherwise unchanged.
- **Matrix criteria**: RAC26 (new, verified). RAC15 keeps its row; its test gains one exception
  branch and no assertion changes.

## 1. Context & Objectives

RAC15 (`readme_assets_contract::test_rac_capture_tooling_is_dev_only`) forbids any file under
`scripts/`, `.github/`, `packaging/`, `tests/docker` and `tests/distro` other than the two README
capture scripts from naming `readme-capture` or `readme_web_fixture_server`. The Playwright bench
(#464) needs the same loopback fixture server, so it avoided the name: it built every example of
`soos-test-fixtures` and took the single executable from cargo's JSON report. The candid review of
#465 judged this honest but noted that RAC15's scanner no longer saw this consumer. The owner chose
to make the bench a named exception instead.

## 2. Architect (Phase 1)

- **Narrow exception, not a blanket skip**: for `scripts/remote_browser_tests.sh` the RAC15 loop
  still flags `readme-capture`; only `readme_web_fixture_server` is allowed. The two capture scripts
  keep their full exception.
- **Explicit build**: the bench builds `--example readme_web_fixture_server` (still `--locked`,
  still from cargo's JSON build report so `CARGO_TARGET_DIR` is honoured, still exactly one
  executable or exit 2).
- **New contract RAC26** pins the narrowness: the build line carries `--locked`,
  `-p soos-test-fixtures` and the named example, has no `--examples`, `--features` or `-F`, and the
  script never names `readme-capture`.

## 3. Tester (Phase 2)

`test_rac_browser_bench_exception_is_narrow` was added first and failed against the previous
script (`--examples` instead of the named example). The RAC15 test body is unchanged except for the
new exception branch (9 added lines, 0 removed).

## 4. Auditor (Phase 3)

No shipped program, packaging file or CI workflow changes. The exception cannot enable the capture
feature: RAC15 still flags the feature name in the bench, and RAC26 refuses any feature flag on the
build line. The fixture server stays synthetic and loopback-only.

## 5. Developer (Phase 4) and Verification

- `cargo test --locked -p soos-invariants`: 879 passed, 0 failed (RAC15 and RAC26 included).
- `cargo fmt --all -- --check`, ShellCheck and `bash -n` on the script: clean.
- `scripts/remote_browser_tests.sh -- --project=chromium`: 18 passed (the named build works).
- A first draft of the script comment quoted the feature name; RAC15 and RAC26 both caught it, and
  the comment was reworded.

## 6. Not Verified

The Browser workflow on GitHub runners (it runs on this PR because the script changed).
