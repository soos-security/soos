# Walkthrough 70: GUI Biometric Profiles Synchronization with System Store

## Objective
Enable unprivileged users running `soos-gui` on the desktop to inspect and consult their enrolled biometric profiles from `/var/lib/soos/biometrics`, synchronize newly registered guided enrollments, and support secure profile deletion via Polkit.

---

## Root Cause Identified
In the zero-trust Linux Hello architecture, `/var/lib/soos/biometrics` is strictly mode `0700` (`root:soos`) and `/var/lib/soos/master.key` is mode `0600` (`root:soos`) to prevent unauthorized unprivileged reading of biometric templates.
When `soos-gui` is executed by a standard unprivileged user on the desktop:
1. `SoosApp` fell back to an isolated `/tmp/soos-gui-biometrics` directory.
2. While guided enrollment successfully exported the newly registered template into `/var/lib/soos/biometrics/<uid>.bio` via `pkexec soos-enroll import`, the GUI's `refresh_profiles()` method only inspected its internal unprivileged `self.store`, which queried the empty `/tmp` store.
3. As a result, the "Biometric Profiles" tab displayed "No biometric templates currently enrolled", giving the impression that the user's registered face profile was not present.

---

## Architectural Changes & Key Implementations

### 1. `soos-enrollment-cli` (`crates/enrollment-cli/src/service.rs`)
- Derived `serde::Serialize` and `serde::Deserialize` on `EnrolledUserSummary` to facilitate structured cross-process synchronization.
- Added workspace `serde` dependency to `crates/enrollment-cli/Cargo.toml`.

### 2. `soos-gui` (`crates/gui/src/app.rs`)
- Updated `refresh_profiles()`: when running unprivileged (`!self.is_system_store`), queries `pkexec soos-enroll list --format json` to fetch the live list of enrolled system profiles directly from the root-managed store.
- Because Polkit caches authorization when guided enrollment imports a template, `refresh_profiles()` synchronizes immediately without re-prompting the user.
- Updated `render_profiles()`:
  - If no profiles are loaded, displays an explicit action button (`🔑 Load Profiles via System Authorization`) allowing the user to initiate Polkit authorization on demand.
  - Enhanced the deletion flow for unprivileged mode to execute `pkexec soos-enroll delete --uid <uid> --yes` and accurately report system store deletion status.

---

## Verification & Test Results
- Added unit test `test_enrolled_user_summary_json_roundtrip` in `crates/gui/tests/layout_tests.rs`.
- `cargo test -p soos-gui`: 4 passed, 0 failed.
- `cargo test -p soos-enrollment-cli`: 41 passed, 0 failed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed with 0 warnings.
- `cargo fmt --all -- --check`: verified clean formatting.
