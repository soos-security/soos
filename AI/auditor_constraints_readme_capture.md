# Audit Constraints — GitHub #381: Reproducible README Screenshot Capture (PR 1 of #356)

- **Date**: 2026-10-13
- **Branch**: `test/readme-capture`
- **Inputs**: `AI/architect_spec_readme_capture.md` (round 2, §10 binding), `AI/plan_evaluator_report.md`,
  tester contract (`crates/gui/tests/readme_snapshots.rs`, `crates/gui/tests/readme_capture/`,
  `tests/fixtures/tests/readme_fixture_tests.rs`, `tests/invariants/src/readme_assets_contract.rs`).

## Checklist results

| # | Item | Result |
|---|---|---|
| 1 | Panic paths | New production code is `crates/gui/src/app/capture.rs` (feature-gated) and `tests/fixtures/mod.rs` (dev-only crate). Both must stay free of `unwrap`/`expect`/`panic!`/indexing (workspace lints, `-D warnings`). PAM untouched. |
| 2 | Unsafe | None anywhere; `soos-gui` keeps `#![forbid(unsafe_code)]` (crate root and `app.rs`); the fixtures crate adds none. |
| 3 | Output isolation | PAM untouched. Only the example `main` prints (`PORT=`), behind a scoped `#[allow(clippy::print_stdout, clippy::print_stderr, reason)]`. |
| 4 | Bounded I/O | Fixture server: request head ≤ 8192 bytes, 2 s read/write timeouts, ≤ 16 live connections, process lifetime clamped to `1000..=300_000` ms, SSE prelude written once then held ≤ `FIXTURE_SSE_HOLD_MS`. Capture: daemon-state wait bounded by `CAPTURE_DAEMON_STATE_TIMEOUT`. |
| 5 | Arithmetic | Frame size check with `checked_mul`; placeholder face clamps sizes to `16..=4096` before allocating (≤ 50 MB). |
| 6 | Filesystem | The server reads only `<assets>/<allow-listed name>` (no path built from the request); the snapshot test writes only `Docs/assets/screenshots/` in update mode, refused under `CI`. |
| 7 | Secrets & privacy | No host identity in scenes (exact state checks + text scan); no store, key, `pkexec`, `systemctl` or camera device touched; fixture bodies carry no login, host, IP, passkey id or endpoint; assets are synthetic and metadata-free (invariant). |
| 8 | Fail-closed | No auth path touched; the capture `RefusingExecutor` refuses every privileged action. |
| 9 | Supply chain | Only `png = "0.18"` (dev-dependency, already locked at 0.18.1, MIT OR Apache-2.0) plus workspace dev-deps; no new crate in the graph; `cargo deny --locked check` must pass. |
| 10 | CI / scripts | No workflow edited. Scripts: `set -euo pipefail`, no network fetch, no privileged command, loopback only, throw-away browser profile (invariant RAC16). |

## Audit Constraints

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| AC-1 | `capture` compiled only with `feature = "readme-capture"`; the feature is empty and not default | `crates/gui/src/app.rs`, `crates/gui/Cargo.toml` | RAC15, `cargo tree -e normal -p soos-gui` |
| AC-2 | `SoosApp::new` and `eframe::App::ui` behaviour unchanged (no edit outside the `mod capture` declaration) | `crates/gui/src/app.rs` | diff review, existing gui tests |
| AC-3 | No process spawn, device open, store open or network in a capture scene: thread-free static camera, fixed daemon probe, refusing executor, `GuiStore::Polkit` without `refresh_profiles` | `capture::capture_app` | RAC3 (state), code review |
| AC-4 | Identity fields overwritten with `alice`/1000 after `EnrollmentGuiState::default()`; profiles only the synthetic one | `capture::capture_app` | RAC3 |
| AC-5 | No `unwrap`/`expect`/indexing/unchecked arithmetic in `capture.rs` and `tests/fixtures/mod.rs` | both | clippy `-D warnings` |
| AC-6 | Server binds `127.0.0.1:0` only; `route` is total (never panics) and never maps a request path to a file path | `tests/fixtures/mod.rs::readme_web`, example | RAC8, RAC15 loopback check |
| AC-7 | Server bounds of item 4 implemented as constants | `readme_web::serve` | RAC8 server test, review |
| AC-8 | `strip_png_metadata` refuses truncation, missing IEND and `acTL`; copies chunks verbatim | `readme_web::strip_png_metadata` | RAC9 strip test |
| AC-9 | PNG writer emits no ancillary chunk; committed assets pass the invariant | test harness, `Docs/assets` | RAC6, RAC13 |
| AC-10 | Update mode refused when `CI` is set | snapshot test | RAC1 |
| AC-11 | `soos-test-fixtures` keeps `soos-frame` as its only normal dependency and stays dev-only | `tests/fixtures/Cargo.toml` | RAC15, `fixtures_contract` |
| AC-12 | Log messages in new code contain no frame/embedding data | `capture.rs` (no logging needed) | review |

### Pre-existing violations found (not introduced by this change)

None relevant. Note: `EnrollmentGuiState::default()` resolves the host login (existing behaviour, GitHub #156 era); the
capture overwrites it (AC-4) instead of changing the production default.

### Clearance: CLEARED
