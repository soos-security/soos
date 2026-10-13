# Walkthrough 198 — macOS Port Foundation: soos-frame, Portable-Crate Guard and Advisory macOS CI

- **Date**: 2026-10-13
- **Issue**: GitHub #367, first PR of the epic #360 (GitHub-only: no `AI/BACKLOG.md` entry, not registered in
  `scripts/sync_issue.py`; the commit carries `Closes #367`). **Branch**: `chore/macos-port-foundation`.
- **ADR**: "[2026-10-13] macOS Port in a Separate Repository" (`AI/DECISIONS.md`).
- **Documents**: spec `AI/architect_spec_macos_port_foundation.md` (Round 3, APPROVED with R3-1 to R3-4 folded in);
  plan evaluation `AI/plan_evaluator_report.md`; tester contract `AI/tester_contract_macos_port_foundation.md`;
  auditor constraints `AI/auditor_constraints_macos_port_foundation.md` (C-1 to C-16, CLEARED);
  `Docs/MACOS_PORT.md`, `Docs/FRAME_CRATE.md`.
- **Matrix criteria**: MAC1–MAC9 (MAC8 and MAC9 pending).
- **Number**: 198. Numbers 196 and 197 are reserved by two in-flight parallel branches (`feat/network-lock` and one
  other), so the next free unique number is used; no file numbered 198 exists.

## 1. Context & Objectives

The owner asked (2026-10-12) to start macOS development of soos in a separate repository and decided on 2026-10-12
and 2026-10-13: no Mac yet (shared-crate preparation only here), private `soos-macos`, maintainer identity
`soos-security`, first target `sudo` through `sudo_local`, RGB-only convenience factor, ad-hoc signing without
notarization, and a non-required macOS CI job. `soos-vision` could not build on macOS because it imported `Frame`,
`PixelFormat` and `SensorType` from `soos-camera-v4l`, which depends on `v4l` unconditionally. This PR delivers the
planned split items 1–3 of #360: ADR + bootstrap specification, the `soos-frame` extraction, and the portable-crate
guard + macOS job.

## 2. Architect Design

- New pure crate `crates/frame` (`soos-frame`): the three types moved byte-for-byte (same derives, methods,
  `Zeroize` impl and wiping `Drop`); single dependency `zeroize`; `#![forbid(unsafe_code)]`; no feature (IR-only types
  later go behind a non-default `ir` feature).
- `soos-camera-v4l` re-exports them (`src/frame.rs`, `src/sensor.rs`), so every existing path stays valid; `soos-vision`
  and `soos-test-fixtures` depend on `soos-frame` instead.
- Portable set: `soos-frame`, `soos-protocol`, `soos-policy`, `soos-push-protocol`, `soos-inference-ort`,
  `soos-vision`, `soos-biometric-store` (POSIX-only calls; compiled on macOS, tests not yet run there).
- Guard: `cargo tree --locked -p <crate> -e normal,build --target all --all-features`, root removed, intersected with a
  31-name forbidden set (V4L2, Linux-PAM, D-Bus/logind, udev/systemd/inotify, Tokio, desktop stacks, Linux-only
  workspace crates; nine names are defensive, absent from `Cargo.lock`).
- CI job `portable-macos` on `macos-latest` (arm64), outside `CI Success`; `scripts/pr_loop.sh` gains the helper
  `scripts/pr_checks_gate.sh` so the macOS job is the single advisory check while every other check stays fail-closed.

## 3. Plan Evaluation

Three rounds. Round 1: REVISION_REQUIRED — X-1 (the guard was permanently red for `soos-test-fixtures` because
`cargo tree` prints the root package), X-2 (`main` has no branch protection; `pr_loop.sh`'s `--fail-fast` watch over
all checks made the "non-required" job a de facto merge gate), and M-3 to M-9 (docs index, probe mechanics, hermetic
broken manifest and per-group witnesses, the `ort` macOS fact — `ort-sys` rc.13 ships only an `aarch64-apple-darwin`
`coreml` build, drift items, walkthrough number, AM enumeration). Round 2: REVISION_REQUIRED — R2-1 (the first
advisory design could merge while a check of another workflow, such as the PR title check, was still pending, because
`CI Success` does not cover it), R2-2 (the probe twin would have read uninitialised bytes), R2-3 (text-only pin).
Round 3: APPROVED with R3-1 to R3-4 binding (records without a trailing newline and a count cross-check, literal and
unique advisory name, explicit status capture under `set -e`, a final gate right before the merge).

## 4. Tester Contract

13 invariant tests in `tests/invariants/src/portable_crates_contract.rs`, 6 tests in
`crates/frame/tests/frame_contract_tests.rs`, 3 tests in `crates/camera-v4l/tests/frame_drop_wipe_tests.rs`. Red
evidence: 10 of 13 invariant tests failed on their assertions (missing crate, manifests still on camera-v4l, job/ADR/
doc/helper absent); the 3 guard self-tests were green by design; the frame and drop-wipe tests failed to compile on
the missing `soos-frame` crate (exactly the specified API).

Migrated existing tests (import paths only, assertions untouched): AM-1 `test_business_crates_forbid_unsafe_code`
(+ `"frame"`), AM-2 the 13 vision test files, AM-3 `tests/fixtures/tests/fixture_generators_tests.rs`;
`git diff -U0` shows exactly 14 `use soos_camera_v4l::` → `use soos_frame::` pairs.

## 5. Auditor Constraints

C-1 to C-16 (CLEARED): verbatim move and single definition, purity of `soos-frame`, both business lists, allocator
probe hygiene (atomics only in the hooks, `// SAFETY:` on every `unsafe`, adapter crate only), consumers unchanged,
fail-closed guard, the gate helper's parsing rules (count cross-check, newline-less final record, literal advisory
match, unknown state blocks), `pr_loop.sh` status capture and final gate, the workflow supply chain (only the SHAs
already used by the Linux jobs, no secrets, read-only token, main-only cache saves, `runner.os` in the ORT cache key),
docs positioning, English only. Pre-existing findings kept unchanged: `Frame`'s derived `Debug` prints the buffer (no
log site formats a whole frame), and the post-`zeroize` byte loop of `frame_zeroize_tests::test_frame_zeroize_trait`
(row H1) is vacuous because `Vec::zeroize` empties the vector — the new allocator probe is the byte-level evidence.

## 6. Implementation

- `Cargo.toml` (member + workspace dependency), `Cargo.lock` (path package `soos-frame`; vision and fixtures edges
  moved), `crates/frame/**`, `crates/camera-v4l/{Cargo.toml,src/frame.rs,src/sensor.rs}`,
  `crates/vision/{Cargo.toml,src/{color,error,pipeline,ir_liveness}.rs}`, `tests/fixtures/{Cargo.toml,mod.rs}`.
- `scripts/candid_review.sh` (`BUSINESS_CRATES` + `frame`), `scripts/pr_checks_gate.sh` (new), `scripts/pr_loop.sh`
  (watch status captured, gate loop on failure bounded by `WATCH_START`, final gate before `gh pr merge`).
- `.github/workflows/ci.yml`: header line and job `portable-macos`; `ci-success` untouched.
- Docs: ADR, `Docs/MACOS_PORT.md`, `Docs/FRAME_CRATE.md`, `Docs/README.md`, `Docs/VISION_CRATE.md`,
  `Docs/CAMERA_V4L_CRATE.md`, `Docs/CI_CD_AND_SECURITY.md`, `AGENTS.md` and `AI/ARCHITECTURE.md` trees (the latter's
  business-crate prose now lists 13 crates), `.agents/skills/dev-workflow/references/project-facts.md`, matrix MAC1–MAC9.

## 7. Candid Review

Not run by this agent (orchestrator instruction): the fresh-context reviewer runs `./scripts/candid_subagent.sh
--prepare` on the final diff. Items for the reviewer: confirm the AM-2/AM-3 diffs are pure `use` swaps, the
verbatim move of `frame.rs`, and the fail-closed paths of `scripts/pr_checks_gate.sh`.

## 8. Verification Results

```
cargo fmt --all -- --check                                                       pass
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings    pass
cargo test   --locked --workspace --all-targets --all-features                   pass (3447 passed, 0 failed, 6 ignored)
cargo deny --locked check                                                        pass (advisories, bans, licenses, sources ok)
./scripts/candid_review.sh                                                       pass (layer 1)
```

`frame_drop_wipe_tests` and `portable_crates_contract` were each run 10 times in a row: all green.

## 9. Known Limitations / Follow-ups

- MAC8: `soos-biometric-store` tests are compiled but not run on macOS (BSD group inheritance, `/private/var` temp
  dirs, directory `flock` unverified). If the store does not compile on macOS, the remediation rule of
  `Docs/MACOS_PORT.md` §5.2 applies.
- MAC9: the macOS job runs for the first time on the pull request; a red run is advisory for `pr_loop.sh` but must be
  inspected.
- `macos-latest` floats; it must stay an arm64 image (no `x86_64-apple-darwin` ONNX Runtime prebuilt).
- Possible follow-up: a redacted `Debug` for `Frame`.
- Owner actions: create `soos-security/soos-macos`, tag `shared-v0.1.0` after merge, README link once the repository
  exists. Deferred: crates.io publication, soos-remote Mac support, reference Mac model, CoreML.
