# Walkthrough 151 — Diagnostics Parity and the Shared `daemon.toml` Reader

- **Date**: 2026-10-01
- **Issue**: GitHub #289 ("PAM / diagnostics" items, the `daemon.toml` reader item of "Camera"
  and the `daemon_config.rs` TOCTOU finding of the #287 candid review); no backlog id
- **Branch**: `fix/p3fu2-diagnostics`
- **Matrix criteria**: DGP1–DGP9 (new component `diagnostics-parity-and-config-reader`)
- **ADR**: 2026-10-01 "One Shared `daemon.toml` Camera Reader; Diagnostics Parity With
  `pam_soos.so`"

---

## 1. Context & Objectives

| #289 item | Outcome | Rows |
|---|---|---|
| `soos-admin test-pam` does not compare the response `request_id` with its nonce | Done: `Response::matches_request` before the freshness check; a mismatch is `PAM_IGNORE (response request_id does not match the request nonce; ...)` | DGP1 |
| `soos-gui` does not apply `Response::check_freshness` | Done for the only `Response` the GUI consumes (the `PreviewFrame` refusal); a stale refusal is `IpcPreviewError::Protocol` | DGP2, DGP3 |
| `pcx_wire_routing_tests` uses `SYS_setresuid` on 32-bit x86 | Done (setup only): `SYS_setresuid32` on `x86` and `arm` | DGP8 |
| One shared `daemon.toml` reader for `soos-admin`, `soos-enroll`, `soos-gui` | Done: `soos_camera_v4l::daemon_config`; per-key fallback for enroll / GUI; **admin keeps `Malformed` for a mistyped key** (existing CVF3 assertion, owner decision pending) | DGP4–DGP6, DGP9 |
| TOCTOU: `metadata()` then blocking `File::open()` (FIFO hang) | Done: open with `O_NONBLOCK \| O_NOFOLLOW \| O_CLOEXEC`, `fstat` the handle, bounded read | DGP7 |

## 2. Architect Design (Phase 1)

**Blast radius.** `soos-camera-v4l` (new `daemon_config.rs`, `lib.rs`, `Cargo.toml` + `toml`),
`soos-admin-cli` (`daemon_config.rs` now a re-export layer, `test_pam.rs`, `Cargo.toml` − `toml`),
`soos-enrollment-cli` (`service.rs`, `lib.rs`, `main.rs`, `Cargo.toml` − `toml`), `soos-gui`
(`ipc_camera.rs`, `camera_source.rs`, `Cargo.toml` nix `time` feature), one daemon test file,
invariants. `Cargo.lock`: three dependency-list lines move, no new package.

**Shared reader** (`crates/camera-v4l/src/daemon_config.rs`):

```rust
pub const DEFAULT_DAEMON_CONFIG_PATH: &str = "/etc/soos/daemon.toml";
pub const MAX_DAEMON_CONFIG_BYTES: u64 = 1024 * 1024;
pub struct DaemonCameraSettings { camera_device, sensor_preference, sensor_preference_unrecognized } // unchanged fields
pub enum DaemonConfigKey { CameraDevice, SensorPreference }            // name(): "camera_device", ...
pub enum DaemonConfigError { NotFound, NotARegularFile, SymbolicLink /* new */, Unreadable(ErrorKind), TooLarge { limit }, Malformed }
pub struct DaemonCameraConfig { settings: DaemonCameraSettings, mistyped_keys: Vec<DaemonConfigKey> }
impl DaemonCameraConfig { pub fn warnings(&self) -> Vec<String> }  // key names only
pub fn read_daemon_camera_config(path: &Path) -> Result<DaemonCameraConfig, DaemonConfigError>;
```

Parsing uses `toml::Table` and inspects each key's type, so one wrongly typed key cannot void
the other. `[pipeline]` that is not a table stays `Malformed` (a section, not a key).

**Placement.** `soos-camera-v4l`: all three clients depend on it already, it owns the vocabulary,
it has no Tokio / ORT; `toml` is already locked by `soos-daemon`. Rejected: `soos-protocol` (wire
crate, no file I/O), the daemon loader (binary crate with Tokio / ORT).

**Clients.** `soos_admin_cli::daemon_config::read_daemon_camera_settings` keeps its signature and
maps a non-empty `mistyped_keys` to `Malformed` (see §3). `soos-enroll`:
`resolve_camera_device_from_config_reported(cli, cfg, enumerator) -> CameraDeviceChoice { path,
notes }`; the existing `resolve_camera_device_from_config[_with]` return the path only (the
library may not print: workspace lint `print_stderr = "deny"`); `camera_config_notes(path)` is
printed by `main.rs` as `[WARN] ...` before each `build_full_service`, so the notes appear even
when the camera then fails to open. `soos-gui` direct mode calls the `_reported` form and logs the
notes with `tracing::warn!`.

**`test-pam`.** `bound_to_request = resp.matches_request(&request_id)` evaluated before
`check_freshness`; the report keeps its fields (JSON shape unchanged).

**GUI.** The GUI never sends `RequestKind::Auth`; the only `Response` it decodes is the refusal of
`PreviewFrame` (`ipc_camera.rs::request_preview`). After the nonce match:
`check_freshness(monotonic_now_ns(), MAX_RESPONSE_FUTURE_SKEW_NS)`; failure → `Protocol`. The
clock helper uses `nix::time::clock_gettime(CLOCK_MONOTONIC)` (safe; the crate forbids unsafe) and
returns 0 on failure, which `check_freshness` rejects (`ClockUnavailable`).

**Latency budget.** No PAM path touched. One `clock_gettime` per preview refusal in the GUI.

## 3. Plan Evaluation

Condensed in this single-agent run (no separate plan-evaluator report). One conflict with an
existing test was found and **not** resolved by changing the test:

- `crates/admin-cli/tests/camera_config_tests.rs::test_cvf_missing_or_invalid_daemon_config_falls_back_with_note`
  asserts `read_daemon_camera_settings("[pipeline]\ncamera_device = 5") == Err(Malformed)`
  ("A key of the wrong type makes the daemon refuse the file: report it as malformed"). The
  orchestrator rule (a wrongly typed key falls back alone) would break that assertion, so, as
  instructed, `soos-admin camera list` keeps the old strict rule on top of the shared reader and
  the question goes to the owner (§9). `soos-enroll` and `soos-gui` get the new rule; their old
  reader already ignored a mistyped key silently, so no existing enrollment or GUI expectation
  changes.

## 4. Tester Contract (Phase 2)

| Test (path::name) | Row | Red evidence (stub API) |
|---|---|---|
| `crates/admin-cli/tests/test_pam_request_id_tests.rs` (3 tests) | DGP1 | mismatch and zero-id tests failed (`PAM_SUCCESS (Authentication authorized)`); the matching control passed |
| `crates/gui/tests/ipc_preview_freshness_tests.rs` (4 tests) | DGP2, DGP3 | expired and unstamped / future-dated refusals failed (`left: Err(Unauthorized) right: Err(Protocol)`); the stale-`Allow` and fresh-refusal controls passed (an `Allow` refusal already mapped to `Protocol`) |
| `crates/camera-v4l/tests/daemon_config_reader_tests.rs` (8 tests) | DGP4–DGP7 | all 8 failed on the stub reader (`Ok(default)` for every input) |
| `crates/admin-cli/tests/camera_config_shared_reader_tests.rs` (3 tests) | DGP4, DGP7 | did not compile: `mismatched types` (admin types were not the shared types) |
| `crates/enrollment-cli/tests/daemon_config_shared_reader_tests.rs` (6 tests) | DGP4–DGP7 | 4 of 5 failed (no notes, oversized file applied); the valid-config control passed; `test_dgp_enroll_camera_config_notes_match_the_resolver` was added with `camera_config_notes` in Phase 4 |
| `tests/invariants/src/diagnostics_parity_contract.rs` (6 tests) | DGP1, DGP2, DGP4, DGP7–DGP9 | 5 of 6 failed; DGP8 passed because the setup fix of item 3 had been applied first |

Each FIFO test runs the reader on a helper thread and fails after 2 s
(`recv_timeout`) instead of hanging the suite. A mistake in the new invariant (a forbidden
pattern `.is_file() {` that also matched the correct handle check) was corrected to
`path.is_file()` / `path.exists()` before Phase 4 finished; no pre-existing test was touched for
that.

### Existing test files touched — setup only (no assertion changed)

| File | Change |
|---|---|
| `crates/daemon/tests/pcx_wire_routing_tests.rs` | `SETRESUID_SYSCALL` = `libc::SYS_setresuid32` under `#[cfg(any(target_arch = "x86", target_arch = "arm"))]`, `libc::SYS_setresuid` otherwise; `set_thread_uids` uses it; SAFETY comment updated |
| `crates/gui/tests/ipc_camera_tests.rs` | `#[path = "common/stamps.rs"] mod stamps;`; `denial()` stamps `1 / 2` → `stamps::fresh_stamps()` |
| `crates/gui/tests/ipc_camera_failure_tests.rs` | same include; `unavailable()` stamps `1 / 2` → `stamps::fresh_stamps()` |

Without the GUI migration, `test_ipc_camera_manager_stops_on_unauthorized_response`,
`test_ipc_camera_manager_keeps_polling_after_rate_limit`, `test_probe_preview_reports_unauthorized`
and `test_ipc_camera_manager_reports_unavailable_and_keeps_polling` would see their (expired)
`1 / 2` refusals turned into `Protocol`. New helper: `crates/gui/tests/common/stamps.rs`
(CLOCK_MONOTONIC through `nix`, `expires = issued + 2 s`, like `crates/pam/tests/common/stamps.rs`).

## 5. Auditor Constraints (Phase 3)

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| 1 | No `unwrap` / `expect` / `panic!` / indexing in new production code | `daemon_config.rs`, `service.rs`, `test_pam.rs`, `ipc_camera.rs` | clippy `-D warnings` (workspace lints) |
| 2 | Open first, `fstat` the handle, bounded read (fstat size and bytes read); no `metadata()` / `is_file()` on the path | shared reader | DGP7 tests and invariant |
| 3 | A symbolic link is never followed (`O_NOFOLLOW` → `ELOOP` → `SymbolicLink`) | shared reader | `test_dgp_symbolic_link_is_not_followed` |
| 4 | Warnings and notes name the key and the file, never a value; no nonce in `test-pam` output | `warnings()`, enrollment notes, `pam_result` | DGP1, DGP5 tests |
| 5 | Fail closed: a nonce mismatch is never `PAM_SUCCESS`; a stale GUI `Response` is never a verdict or an authorized preview; a clock failure is stale | `test_pam.rs`, `ipc_camera.rs` | DGP1–DGP3 |
| 6 | Libraries do not print (`print_stderr` lint); only `main.rs` prints notes | enrollment-cli | clippy |
| 7 | `soos-gui` stays `#![forbid(unsafe_code)]` (safe `nix` clock) | `ipc_camera.rs` | build |
| 8 | No new crate in `Cargo.lock`; no Tokio / ORT pulled into `soos-admin-cli` | `Cargo.toml`s | `Cargo.lock` diff (dependency lists only) |
| 9 | The x86 change is setup only and does not alter the per-thread semantics | `pcx_wire_routing_tests.rs` | DGP8 invariant; test green on `x86_64` |

Pre-existing violations found: the enrollment reader followed symlinks, blocked on a FIFO
(`is_file()` then `read_to_string`) and read without a bound — all removed by the shared reader.
Clearance: CLEARED.

## 6. Implementation (Phase 4)

- `crates/camera-v4l/src/daemon_config.rs` (new), `lib.rs` (`pub mod daemon_config;`),
  `Cargo.toml` (`toml`).
- `crates/admin-cli/src/daemon_config.rs`: re-exports the shared constants and types;
  `read_daemon_camera_settings` = shared reader + strict mistyped-key rule. `Cargo.toml`: `toml`
  removed. `camera.rs` unchanged (the new `SymbolicLink` error renders through the existing note).
- `crates/admin-cli/src/test_pam.rs`: nonce binding before the freshness check.
- `crates/enrollment-cli/src/service.rs`: private loose reader removed; `CameraDeviceChoice`,
  `resolve_camera_device_from_config_reported`, `camera_config_notes`; `build_full_service` uses
  `DEFAULT_DAEMON_CONFIG_PATH`. `main.rs`: `warn_camera_config_notes()` before each
  `build_full_service`. `lib.rs`: re-exports. `Cargo.toml`: `toml` removed.
- `crates/gui/src/ipc_camera.rs`: freshness check on refusals, `monotonic_now_ns()`;
  `camera_source.rs`: `_reported` + `tracing::warn!`; `Cargo.toml`: nix `time` feature.
- Docs: `Docs/CAMERA_V4L_CRATE.md` (new "Shared `daemon.toml` Camera Reader" section, matrix
  mapping), `Docs/IPC_PROTOCOL.md` (Response Freshness and §9 client contract),
  `Docs/GUI_APPLICATION.md`, `Docs/ENROLLMENT_CLI.md`; `AI/DECISIONS.md` (ADR inserted after
  "PAM Client Enforces Response Expiry"; it replaces that ADR's "`soos-gui` does not check the
  stamps" sentence, which was left untouched to limit merge conflicts); `AI/VERIFICATION_MATRIX.md`
  (section inserted after `pam-response-expiry-protocol-followups`).

## 7. Candid Review

Layer 2 (fresh-context reviewer) not run in this batch: the orchestrator runs it on the
integrated branch. Layer 1 (`./scripts/candid_review.sh`) passed (§8).

## 8. Verification Results

2026-10-01, `CARGO_BUILD_JOBS=5`:

- `cargo test --locked --all-features --no-fail-fast -p soos-camera-v4l -p soos-admin-cli -p soos-enrollment-cli -p soos-gui -p soos-daemon -p soos-protocol -p soos-invariants`:
  161 test binaries, 1317 tests passed, 0 failed.
- `cargo clippy --locked --all-targets --all-features` on the same packages `-- -D warnings`: clean.
- `cargo fmt --all -- --check`: clean.
- `./scripts/candid_review.sh` (layer 1): PASSED.
- No `i686` / `armv7` toolchain on the host: the 32-bit branch of `SETRESUID_SYSCALL` was not
  compiled (the constants exist in `libc` 0.2.186 for gnu and musl on both architectures).

## 9. Known Limitations / Open Questions

- **Owner decision needed**: should `soos-admin camera list` adopt the per-key fallback too? It
  requires changing the `Err(Malformed)` assertion for `camera_device = 5` in
  `camera_config_tests::test_cvf_missing_or_invalid_daemon_config_falls_back_with_note` (CVF3).
  Today the admin rule matches what `soos-daemon` does (refuses to start); the enroll / GUI rule
  follows the orchestrator decision.
- A symlinked `/etc/soos/daemon.toml` (config management) is now refused by the three clients
  with a note, while `soos-daemon` follows it. If symlinked configurations must be supported, the
  reader would need a root-owned-target check instead of `O_NOFOLLOW`.
- The GUI notes path display is not sanitized for control characters (the admin note is); the
  path is operator-supplied (`/etc/soos/daemon.toml` in production).
