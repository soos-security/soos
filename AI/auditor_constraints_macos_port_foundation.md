# Audit Constraints — GitHub #367 (first PR of #360): macOS Port Foundation

- **Date**: 2026-10-13
- **Branch**: `chore/macos-port-foundation`
- **Inputs**: spec `AI/architect_spec_macos_port_foundation.md` (Round 3, `AI/plan_evaluator_report.md` APPROVED with
  R3-1 to R3-4 binding), tester contract `AI/tester_contract_macos_port_foundation.md`, current code.

## Checklist results (greps on the affected crates)

| # | Item | Result |
|---|---|---|
| 1 | Panic paths: `grep -rnE '\.unwrap\(|\.expect\(|panic!|todo!|unimplemented!|unreachable!' crates/camera-v4l/src/{frame,sensor}.rs` | none; the moved code uses `checked_div(..).unwrap_or(0)` and `saturating_*` only |
| 2 | Unsafe | moved code has none; `crates/frame` is a business crate (forbid); the only new `unsafe` is the test-only allocator probe in `crates/camera-v4l/tests/frame_drop_wipe_tests.rs` (adapter crate, permitted by `scripts/candid_review.sh`) |
| 3 | Output isolation | no PAM change; `soos-frame` has no print or tracing |
| 4 | Bounded I/O | `soos-frame` has none; the gate helper and the pending loop are bounded by `CI_WATCH_TIMEOUT_SECS` from `WATCH_START` |
| 5 | Arithmetic | `expected_buffer_size` saturates; `age_ms` saturates — unchanged |
| 6 | Filesystem | invariant tests write only under `std::env::temp_dir()` with a per-process name and remove it |
| 7 | Secrets & privacy | `Frame` keeps its `Zeroize` impl and the wiping `Drop`; its derived `Debug` prints the buffer (pre-existing, unchanged, drift item 1). `grep -rnE '\{:\?\}' crates/*/src` finds no log site formatting a whole `Frame` |
| 8 | Fail-closed | no PAM or daemon path changed; the release loop gains a gate that blocks on every unknown state |
| 9 | Supply chain (crates) | no new external crate; `soos-frame` is private (`publish.workspace = true`), licence inherited; `Cargo.lock` gains only the path package `soos-frame` and loses `soos-camera-v4l` from `soos-vision`/`soos-test-fixtures` edges; `cargo deny --locked check` must stay green |
| 10 | CI/workflow (new job `portable-macos`) | see C-10 to C-14 |

### Supply-chain review of the new workflow job

- Actions: `actions/checkout@3d3c42e5…` (v7.0.1), `Swatinem/rust-cache@6323deb1…` (v2.9.2),
  `actions/cache/restore@55cc8345…` and `actions/cache/save@55cc8345…` (v6.1.0) — the exact SHAs already used by the
  Linux jobs; no new third-party action, no `taiki-e`/`dtolnay` toolchain action (rustup preinstalled on the runner).
- Token: workflow default `permissions: contents: read`; no job-level `permissions:`; `persist-credentials: false`.
- No `secrets.*`, no `github.event.*` interpolation in `run:`, no `pull_request_target`; fork PRs run with the read-only
  token.
- Caches: only `main` saves (`save-if` / `if: github.ref == 'refs/heads/main'`); the ORT cache key includes
  `runner.os` (`macOS`), so a macOS archive never lands in the Linux cache key space and vice versa; `ort-sys` verifies
  the SHA-256 of the ONNX Runtime archive it downloads.
- Runner image `macos-latest` floats (arm64 today); accepted for a non-required job (ADR); an x86 image would fail in
  `prefetch_onnxruntime.sh` (no `x86_64-apple-darwin` prebuilt in `ort-sys` rc.13).

## Audit Constraints

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| C-1 | `Frame`, `PixelFormat`, `SensorType` moved verbatim (derives, methods, `Zeroize`, `Drop` wiping `data`), defined only in `soos-frame` | `crates/frame/src/*.rs`, `crates/camera-v4l/src/{frame,sensor}.rs` | `test_mac_frame_crate_is_pure_and_forbids_unsafe`, `test_mac_camera_v4l_reexports_the_frame_types`, `frame_contract_tests::*`, `frame_drop_wipe_tests::*` |
| C-2 | `soos-frame` first non-comment line `#![forbid(unsafe_code)]`; only normal dependency `zeroize`; no `std::{fs,net,process,os,io,thread,env}`, no print/tracing; no `[features]` | `crates/frame` | `test_mac_frame_crate_is_pure_and_forbids_unsafe` |
| C-3 | `frame` added to both business lists (invariant + `scripts/candid_review.sh`) | `tests/invariants/src/lib.rs`, `scripts/candid_review.sh` | `test_business_crates_forbid_unsafe_code`, `test_mac_business_lists_include_frame` |
| C-4 | The allocator probe: atomics only inside `alloc`/`dealloc`, reads only the watched block before `System.dealloc`, every `unsafe` preceded by `// SAFETY:`, never reads uninitialised bytes (twin buffer fully written) | `crates/camera-v4l/tests/frame_drop_wipe_tests.rs` | review + clippy `undocumented_unsafe_blocks` |
| C-5 | Existing tests: only the AM-1 to AM-3 edits; `git diff -U0` of AM-2/AM-3 shows exactly the 14 `use` swaps | vision tests, fixtures test | candid review |
| C-6 | No consumer change needed beyond `soos-vision` and `soos-test-fixtures` (daemon, gui, enrollment-cli, admin-cli compile through the re-exports) | workspace | `cargo clippy --workspace --all-targets --all-features` |
| C-7 | Guard fail-closed: non-zero `cargo tree` exit fails the test; root presence asserted; root removed only by exact name | `portable_crates_contract.rs` | the guard tests |
| C-8 | Gate helper `scripts/pr_checks_gate.sh`: `set -euo pipefail`; argument must be `^[0-9]+$` (else exit 1 without calling `gh`); one `gh pr checks "$pr" --json name,bucket --jq '(length|tostring), (.[] \| [.name, .bucket] \| @tsv)'`; `gh` failure → 1; first line must be a decimal count equal to the parsed records (R3-1); every record read with `IFS= read -r line \|\| [[ -n "$line" ]]` and split on exactly one tab; non-empty name; bucket in `pass\|fail\|pending\|skipping\|cancel`; advisory by quoted literal `[[ "$name" == "$advisory" ]]` (R3-2); zero non-advisory checks → 1; any non-advisory fail/cancel → 1; else pending → 3; else 0 | `scripts/pr_checks_gate.sh` | `test_mac_pr_checks_gate_decisions`, `test_mac_pr_loop_treats_only_the_macos_job_as_advisory`, shellcheck (lint job) |
| C-9 | `pr_loop.sh`: helper called only when the fail-fast watch fails; status captured `gate_rc=0; "$GATE_HELPER" "$PR_NUMBER" \|\| gate_rc=$?` (R3-3); `3` loops with a 15 s sleep only while within `CI_WATCH_TIMEOUT_SECS` of `WATCH_START`; any other non-zero → `exit 1`; the `CI Success` poll unchanged; after a green `CI Success`, the gate runs again and only `0` proceeds to `gh pr merge` (R3-4); comment states that `CI Success` alone is not sufficient and that `gh pr checks --json` exits 0 on failed checks | `scripts/pr_loop.sh` | textual pin + review |
| C-10 | macOS job actions pinned to the full SHAs already used by Linux jobs, with `# vX` comments | `.github/workflows/ci.yml` `portable-macos` | `test_mac_macos_job_is_pinned_secret_free_and_non_required` |
| C-11 | No `secrets.`, no job `permissions:`, no `continue-on-error`, `persist-credentials: false`, `if: github.event_name != 'schedule'`, `timeout-minutes` set | same | same test |
| C-12 | Packages: clippy over exactly the 7 portable crates; `test --no-run` and `test` over exactly the 6 tested crates; never `--workspace` nor a forbidden crate; `--locked --all-targets --all-features` | same | same test |
| C-13 | `ci-success.needs` unchanged (macOS job absent); job name unique across workflows | `ci.yml` | same test + `test_mac_pr_loop_treats_only_the_macos_job_as_advisory` |
| C-14 | ORT cache: restore/prefetch/save with the same pins; save only on `main` | same | review (AFC4 pattern) |
| C-15 | Docs and ADR never claim macOS soos replaces the password; RGB-only convenience factor stated; no sensitive data claims weakened | `AI/DECISIONS.md`, `Docs/MACOS_PORT.md` | `test_mac_adr_records_the_separate_repository_decision`, `test_mac_macos_port_doc_covers_the_bootstrap_spec`, candid review |
| C-16 | English only, repository-relative paths only in every new file | all | `./scripts/candid_review.sh` |

### Pre-existing violations found (not introduced by this change)

- `Frame` derives `Debug`, which prints the raw buffer (no redaction), although #360's risk list assumes redaction. No
  current log site formats a whole `Frame`. Kept unchanged (behaviour-preserving move); recorded as a follow-up.
- `crates/camera-v4l/tests/frame_zeroize_tests.rs::test_frame_zeroize_trait` loops over `frame.data` after
  `zeroize()`, when the vector is already empty: its byte check is vacuous (row H1). Not modified; the new allocator
  probe is the byte-level evidence.

### Clearance: CLEARED
