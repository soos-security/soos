## Audit Constraints — GitHub #337 (soos-enroll camera-busy message)

Branch: `fix/enroll-camera-busy-message`. Inputs: `AI/architect_spec_enroll_camera_busy_message.md`,
`AI/plan_evaluator_report.md` (APPROVED), `AI/tester_contract_enroll_camera_busy_message.md`,
uncommitted diff (`crates/enrollment-cli/src/error.rs` stub + new
`crates/enrollment-cli/tests/enroll_camera_busy_tests.rs`).

Red run re-executed by the auditor:
`cargo test -p soos-enrollment-cli --locked --all-features --test enroll_camera_busy_tests`
→ 8 passed, 6 failed, all six on assertions (`got Err(Camera(Starved))` / `got "Camera is busy"`),
no compile error. Matches the contract.

| # | Constraint | Applies to (file::fn) | Verified by (test / lint / invariant / grep) |
|---|---|---|---|
| 1 | No `unwrap`/`expect`/`panic!`/`todo!`/`unreachable!`/indexing in the new code. The status check is a plain `matches!(camera.status(), CameraStatus::Error { kind: CameraErrorKind::DeviceBusy, .. })`. | `crates/enrollment-cli/src/service.rs::acquire_frame_after` | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`; `git diff -U0 -- crates/enrollment-cli/src \| grep -E '^\+.*(\.unwrap\(\|\.expect\(\|panic!\|todo!\|unreachable!)'` empty |
| 2 | `camera.status()` is read **only on budget expiry**, after the last `latest_frame()` poll failed, exactly once per expiry. A frame that arrives in time is returned whatever the status (no status-first short circuit, no early exit before the budget). | `service.rs::acquire_frame_after` | `test_ecb_frame_arriving_while_status_busy_is_used_for_enroll`, `..._for_verify`; review |
| 3 | Budgets, poll interval and loop shape unchanged: `FIRST_FRAME_TIMEOUT_MS`, `ENROLL_FRESH_FRAME_TIMEOUT_MS`, `FRAME_POLL_INTERVAL_MS` untouched; no new wait, retry or sleep. `status()` is non-blocking (mock: poison-tolerant `RwLock` read; V4L: poison-tolerant `Mutex` copy in `CameraStatusCell::get`), so the bound is preserved. | `service.rs` constants + `acquire_frame_after` | diff review; `enroll_fresh_frames_tests.rs` stays green |
| 4 | Only `CameraErrorKind::DeviceBusy` maps to `CameraBusy`; every other status (`Starting`, `Suspended`, `Stopped`, `Ready`, any other `Error` kind) keeps `EnrollmentCliError::Camera(CameraError::Starved)` byte-for-byte. Do not use `CameraError::is_device_busy()` on some other value or a wildcard `Error { .. }` arm. | `service.rs::acquire_frame_after` | `test_ecb2_*` (6 tests, incl. `CameraErrorKind::ALL` sweep) |
| 5 | Both budget paths (first frame, `previous_sequence == None`, and fresh frame, `Some(_)`) apply the same mapping; implement it at the single expiry return, not in `enroll`/`verify`. | `service.rs::acquire_frame_after` | `test_ecb1_enroll_busy_camera_returns_camera_busy`, `test_ecb1_verify_busy_camera_returns_camera_busy`, `test_ecb1_enroll_camera_goes_busy_mid_enrollment_returns_camera_busy` |
| 6 | `CameraBusy` stays a field-less unit variant. Its `#[error]` text is the exact spec string (§2): contains `another process holds`, `soos-daemon`, `soos-gui`, `sudo systemctl stop soos-daemon`, `sudo systemctl start soos-daemon`, and does not contain `starved`. Remove the "TDD Red stub" doc line; keep the GitHub #337 doc line. | `crates/enrollment-cli/src/error.rs::EnrollmentCliError::CameraBusy` | `test_ecb1_camera_busy_message_guides_operator`, `test_ecb1_camera_busy_from_enroll_displays_guidance` |
| 7 | Message content: static text only — no device path, uid, username, errno string, frame/biometric data, key path or any runtime value interpolated. Commands are accurate: the unit is `packaging/soos-daemon.service` (`systemctl stop/start soos-daemon` valid); `soos-gui` enrolls through the daemon preview IPC (`crates/gui/src/camera_source.rs`), so it is a valid alternative while the daemon runs. English only. | `error.rs` | review; `grep -n 'CameraBusy' -B4 crates/enrollment-cli/src/error.rs` |
| 8 | Exit code (ECB3): `main()` and `run()` are not modified; `CameraBusy` reaches the existing `eprintln!("[ERROR] {err}"); std::process::exit(1)` path like `Camera(_)`. No new exit path, no new `print*` outside `main.rs`. | `crates/enrollment-cli/src/main.rs::main` | `git diff --stat` shows no `main.rs` change; candid review |
| 9 | Fail-closed: `CameraBusy` is an `Err`; no template is stored on a busy camera and no error path becomes success. No PAM, daemon, GUI, camera-crate or protocol change. | `service.rs::enroll`/`verify` | `fx.store.get(..).is_none()` assertions in ECB1 tests; `git diff --name-only` limited to `crates/enrollment-cli/src/{error,service}.rs` + docs |
| 10 | No `unsafe`, no new dependency, `Cargo.lock` unchanged; `#![forbid(unsafe_code)]` status of `soos-enrollment-cli` unchanged. | crate | `git diff --exit-code Cargo.lock crates/enrollment-cli/Cargo.toml`; `test_business_crates_forbid_unsafe_code` |
| 11 | Test integrity: the contract file `enroll_camera_busy_tests.rs` and every existing test (notably `enroll_fresh_frames_tests.rs::test_enroll_frozen_camera_fails_with_bounded_wait`, which pins `Camera(Starved)` on a default-status camera) are not modified, weakened or deleted. | `crates/enrollment-cli/tests/**`, all `#[cfg(test)]` modules | `git diff --name-only HEAD -- 'crates/*/tests' tests/` empty for existing files; contract file hash unchanged after developer phase |
| 12 | Any exhaustive `match` on `EnrollmentCliError` elsewhere (none found today in `gui`, `admin-cli`, `daemon`) must compile without a catch-all that hides `CameraBusy` behind a success-like outcome. | workspace | `cargo build --workspace --all-features --locked` |

### Test strength assessment

- Kills a status-first implementation (constraint 2): ECB-F tests with a stale `DeviceBusy` status and live frames.
- Kills "any `Error` ⇒ busy" (constraint 4): `CameraErrorKind::ALL` sweep minus `DeviceBusy`.
- Kills a first-frame-only implementation (constraint 5): mid-enrollment stub (1 frame, then silent, `DeviceBusy`).
- Kills a placeholder message (constraint 6): substring checks on both `Display` and the end-to-end error.
- Real mock path covered: `MockCameraManager::set_error(Simulated { code: EBUSY })` reports `Error { kind: DeviceBusy, failures: 1 }` (asserted as a fixture precondition).
- Residual gap (accepted): ECB3 is a review check, not a test; covered by constraint 8.

### Pre-existing violations found (not introduced by this change)

None relevant. `acquire_frame_after`'s doc comment (`# Errors`) will need to mention `CameraBusy` — documentation update, not a violation.

### Clearance: CLEARED
