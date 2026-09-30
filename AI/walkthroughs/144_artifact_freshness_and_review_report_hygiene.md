# Walkthrough 144 — Artifact Freshness and Review-Report Hygiene

- **Date**: 2026-09-30
- **Issues**: GitHub #244 (TCI-13, stale host-built `libpam_soos.so` and sandbox toolchain),
  #267 (TCI-14, stale and inconsistently named AI review reports), #281 (TCI-FU follow-ups:
  remaining Docker and CI evidence)
- **Branch**: `fix/p2-quality-followups`
- **Matrix criteria**: QFX1–QFX5 (new)
- **ADR**: "Release Harnesses Always Rebuild; Candid Fingerprint Ignores Both Review Reports"
  (2026-09-30)

---

## 1. Starting Point on `origin/main` (`abd6016`)

| Item | State before this branch |
|---|---|
| #244 pinned toolchain in the images | Already done: `rust-toolchain.toml` pins `1.98.1` and every sandbox Dockerfile installs `--default-toolchain 1.98.1` (TCP1–TCP3); `tests/docker/test_suite.sh` syncs the toolchain and always rebuilds the PAM module (DDS5, DDS6) |
| #244 stale artifact reuse | Still present in `tests/docker/test_packages.sh` and `tests/distro/{debian_ubuntu,fedora_rhel,arch_linux}_test.sh`: `if [[ ! -f "target/release/..." ]]; then cargo build ...; fi` packaged whatever release binaries the bind-mounted checkout already held |
| #267 orphan plan report | The Issue #3 plan report *(`AI/plan_evaluation_report.md`, now deleted)* was still committed next to `AI/plan_evaluator_report.md` |
| #267 fingerprint | `scripts/candid_subagent.sh` excluded only `AI/candid_review_report.md`, so rewriting the plan report changed the reviewed diff |
| #281 checklist | Items 1, 3, 4 and the comment item were fixed in walkthrough 131; item 2 (`rpm -U`) and item 5 (CI URLs) lacked Docker/CI evidence for QFU3, DDS6, PK7 and DV3 |

## 2. Changes

- **Release harnesses** (#244): `tests/docker/test_packages.sh` always runs
  `cargo build --locked --release --workspace`; the three distribution harnesses do so whenever
  `--skip-build` is not given. The `--skip-build` help text now says it packages the existing
  binaries. Cargo makes the build a no-op when the artifacts are fresh, so the CI order
  (distribution harness, then package harness, same container and target directory) costs nothing.
- **Orphan report** (#267): deleted.
- **Candid fingerprint** (#267): `REVIEW_EXCLUDES` holds both singletons
  (`AI/candid_review_report.md`, `AI/plan_evaluator_report.md`) and is used by the fingerprint
  diff, the `--prepare` file list and the zero-change check. The header and `--help` range were
  updated. Both reports remain overwritten singletons; on `main` they describe the last merged
  pull request (documented in `Docs/CI_CD_AND_SECURITY.md` §5 and the candid-reviewer skill).
- **Evidence** (#281): the green `main` CI run of `abd6016`
  (https://github.com/Mysticaly622/soos/actions/runs/36777041130) executed the pending Docker
  cases; QFX5 records the job URLs and the log lines (toolchain sync `rustc 1.98.1`, `rpm -U`
  from the `%ghost`-key fixture kept the exact master key, first green Arch `distro-deploy`).
  Existing rows DDS6, QFU3, PK7 and DV3 were not rewritten (append-only rule for this batch).

## 3. Red → Green

New module `tests/invariants/src/artifact_freshness_contract.rs` (7 tests).

| Test | Before | After |
|---|---|---|
| `test_harnesses_never_skip_the_release_build_because_an_artifact_exists` | FAILED (4 guarded builds) | ok |
| `test_release_harnesses_always_run_the_locked_workspace_build` | ok (regression guard) | ok |
| `test_orphan_plan_evaluation_report_is_removed` | FAILED | ok |
| `test_candid_fingerprint_ignores_both_review_report_singletons` | FAILED (plan report changed the fingerprint) | ok |
| `test_candid_prepare_lists_neither_review_report` | FAILED | ok |
| `test_candid_gate_treats_a_report_only_change_as_zero_changes` | FAILED | ok |
| `test_candid_report_exclusions_are_documented` | FAILED | ok |

The three candid tests run the real script in a scratch git repository under `target/`.

## 4. Verification

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean.
- `cargo test --locked --workspace --all-targets --all-features --no-fail-fast`: all green except
  `soos-pam` `strict_decode_tests::test_direct_authenticate_reports_trailing_bytes_codec_error`
  once under shared-machine load; it passed three times when rerun alone (crate not touched).
- `./scripts/candid_review.sh`: passed.
- `bash -n` on every touched shell script.

## 5. Follow-ups

- A branch whose diff already changes `AI/plan_evaluator_report.md` gets a new fingerprint once
  rebased onto this change and must refresh its report.
- Moving the review reports to per-issue files under `AI/reviews/` was considered and not done
  (ADR point 3).
