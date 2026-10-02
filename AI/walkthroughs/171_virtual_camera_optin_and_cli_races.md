# Walkthrough 171 — Virtual Camera Opt-In and CLI Races

- **Date**: 2026-10-02
- **Issue**: GitHub #318 ([FU-1002] follow-ups of the 2026-10-02 review batch), items "Daemon
  `allow_virtual_camera`" and "`gdm restore` / `logs --file`". GitHub-only follow-up: the
  branch is not registered in `BRANCH_TO_ISSUE`; the commits carry `Refs #318` (the other
  items of the issue stay open).
- **Branch**: `fix/virtual-camera-optin-and-cli-races`
- **Base commit**: `ac47d3e`
- **Matrix criteria**: VCO1–VCO5 (new section `virtual-camera-optin-and-cli-races` right after
  the GCV section); GCV6 annotation updated
- **ADR**: 2026-10-02 "Virtual V4L2 Nodes Are Never Biometric Cameras" (amended)

---

## 1. Context and Objectives

The 2026-10-02 review batch (PR #317) left these non-blocking follow-ups:

- `[pipeline] allow_virtual_camera` was read by the shared `daemon.toml` reader but honoured
  only by `soos-gui` direct mode; `soos-daemon` and `soos-enroll` always refused virtual nodes
  (fail closed, but the documented opt-in did nothing for them).
- The daemon loader accepted a `camera_device` holding a NUL byte; only the guarded open turned
  it into a recoverable error later.
- `soos-admin gdm restore` compared the PAM file with the backup, then wrote, without any check
  in between: an edit landing in that window was silently discarded.
- `soos-admin logs --file` read the whole file forward to keep the last lines: bounded memory,
  but O(file size) time.

## 2. Architect Design

| Item | Design |
|---|---|
| Daemon opt-in | `PipelineConfigFile::allow_virtual_camera: Option<bool>`; `from_toml_str` copies it to `config.pipeline.camera.allow_virtual_device` and, when `true`, pushes a message to `DaemonConfig::warnings` (logged at `warn` by `main.rs` after logging starts, GitHub #315 mechanism). A non-boolean value fails the typed parse (startup error), like every daemon key. |
| Daemon NUL | `from_toml_str` refuses a `camera_device` whose bytes contain `0` with `DaemonError::Config("[pipeline] camera_device must not contain a NUL byte")` (value never echoed). |
| Enroll opt-in | `CameraDeviceChoice::allow_virtual_camera: bool` filled from the same shared-reader call as the device (`CameraSettingsRead`, private); `pub fn enrollment_camera_config(device_path, allow_virtual_device) -> CameraConfig`; `build_full_service_with_notes` opens the camera with `enrollment_camera_config(device_path, choice.allow_virtual_camera)`. |
| `gdm restore` | `PamFileSnapshot { dev, ino, bytes }` read through one `O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC` descriptor (bounded by `MAX_PAM_FILE_BYTES`). The stale-backup check compares that snapshot; `write_atomic_checked` runs `ensure_pam_file_unchanged` after the temporary file is written and synced, right before `rename`: a different snapshot (bytes, inode, or a file that appeared) removes the temporary file and fails with "changed concurrently ... re-run". Private seam `restore_gdm_pam_file_with(pam_file, force, before_rename)`. |
| `logs --file` | Private `trait PositionalRead` (implemented by `File` through `FileExt::read_at`); `tail_lines(source, len, capacity)` scans backwards from the `fstat` size in `TAIL_CHUNK_BYTES` (64 KiB) chunks, recording only `[start, end)` ranges until `capacity` lines are found, then reads each line's first `MAX_LOG_LINE_BYTES` bytes and decodes it lossily. |

Invariants touched: fail closed by default (the opt-in is `false` unless the file says `true`),
no sensitive value logged (warnings and errors name keys only), every read stays bounded.

## 3. Plan Evaluation

Condensed plan (orchestrator-scoped batch with the design given in the task). Decisions made
here: (a) the daemon treats a mistyped `allow_virtual_camera` as a startup error (its typed
loader does so for every key), while the shared client reader keeps its documented "mistyped
key falls back with a note" rule; (b) the `gdm restore` concurrency check also applies with
`--force` (`--force` only skips the stale-backup comparison; a concurrent edit is never
intended); (c) the enrollment warning about the opt-in is left to the capture supervisor, which
already logs one when it opens a virtual node with the opt-in. Out of scope as instructed:
`packaging/`, `scripts/`, `.github/`, `tests/docker`, `tests/distro`, `crates/gui`.

## 4. Tester Contract

| Test (path::name) | Matrix | Red evidence |
|---|---|---|
| `crates/daemon/tests/virtual_camera_optin_tests.rs::test_vco_daemon_allow_virtual_camera_true_allows_virtual_devices` | VCO1 | panicked at line 42: `[pipeline] allow_virtual_camera = true must set CameraConfig::allow_virtual_device` |
| `crates/daemon/tests/virtual_camera_optin_tests.rs::test_vco_daemon_allow_virtual_camera_defaults_false` | VCO1 | passed (regression guard of the fail-closed default) |
| `crates/daemon/tests/virtual_camera_optin_tests.rs::test_vco_daemon_allow_virtual_camera_mistyped_is_refused` | VCO1 | panicked: `` `[pipeline] allow_virtual_camera = "yes"` must be refused, but was accepted `` |
| `crates/daemon/tests/virtual_camera_optin_tests.rs::test_vco_daemon_rejects_nul_byte_camera_device` | VCO2 | panicked: the NUL `camera_device` "must be refused, but was accepted" |
| `crates/enrollment-cli/tests/virtual_camera_optin_tests.rs::test_vco_enroll_honours_allow_virtual_camera` | VCO3 | panicked at line 53: `the opt-in must be read` (stub field always `false`) |
| `crates/enrollment-cli/tests/virtual_camera_optin_tests.rs::test_vco_enroll_allow_virtual_camera_defaults_false` | VCO3 | passed (regression guard) |
| `crates/enrollment-cli/tests/virtual_camera_optin_tests.rs::test_vco_enroll_full_service_uses_the_choice_opt_in` | VCO3 | passed against the stub call site (source guard against a second read or a hard-coded `false`) |
| `gdm::restore_race_tests::test_vco_gdm_restore_aborts_when_file_changes_before_rename` | VCO4 | panicked: `a concurrent change must abort the restore: ()` |
| `gdm::restore_race_tests::test_vco_gdm_restore_force_still_aborts_on_concurrent_change` | VCO4 | same |
| `gdm::restore_race_tests::test_vco_gdm_restore_aborts_when_file_is_replaced_by_same_bytes` | VCO4 | same |
| `gdm::restore_race_tests::test_vco_gdm_restore_aborts_when_missing_file_appears` | VCO4 | same |
| `gdm::restore_race_tests::test_vco_gdm_restore_without_concurrent_change_restores` | VCO4 | passed (nominal path) |
| `logs::tail_tests::test_vco_logs_tail_reads_only_the_tail_region` | VCO5 | panicked: `the default tail must not read the whole file: 67108800 bytes read` |
| `logs::tail_tests::test_vco_logs_backward_tail_matches_the_forward_reader` | VCO5 | passed against the forward stub (oracle equivalence guard) |

The red runs used stubs with the final signatures (`CameraDeviceChoice::allow_virtual_camera`
always `false`, `enrollment_camera_config` ignoring the flag, the restore hook without any
verification, `tail_lines` reading forward through the seam).

Migrated existing tests: none. No existing test file was modified.

## 5. Auditor Constraints

1. No `unwrap`/`expect`/indexing in production code: the new code uses `get_mut`,
   `saturating_*` and `try_from` (clippy `-D warnings` with the workspace lints is green).
2. Bounded I/O: the PAM snapshot is read through one descriptor with `take(MAX_PAM_FILE_BYTES +
   1)`; the log tail keeps one 64 KiB chunk, at most `capacity` ranges and at most
   `MAX_LOG_TAIL_LINES` lines of `MAX_LOG_LINE_BYTES` bytes (the previous memory bound).
3. No new path-based check-then-use in `gdm restore`: the snapshot is taken on an `O_NOFOLLOW`
   descriptor and identified by device and inode; the remaining window between the final
   re-read and `rename(2)` is a few system calls (see §9).
4. No value is logged or echoed: the NUL error and the opt-in warning name the key only.
5. Fail closed: the opt-in defaults to `false` everywhere; a non-boolean value is refused by the
   daemon and ignored (with a note) by the enrollment CLI.
6. `#![forbid(unsafe_code)]` crates unchanged in that respect (no `unsafe` added anywhere).

## 6. Implementation

- `crates/daemon/src/config.rs`: `allow_virtual_camera` key and warning; NUL `camera_device`
  refusal.
- `crates/enrollment-cli/src/service.rs`, `lib.rs`: `CameraDeviceChoice::allow_virtual_camera`,
  `CameraSettingsRead`, `enrollment_camera_config`, used by `build_full_service_with_notes`.
- `crates/admin-cli/src/gdm.rs`: `PamFileSnapshot`, `read_pam_file_snapshot`,
  `ensure_pam_file_unchanged`, `write_atomic_checked` / `WriteFailure`, unit tests.
- `crates/admin-cli/src/logs.rs`: `PositionalRead`, `read_exact_at`, backward `tail_lines`; the
  forward `read_bounded_line` is kept test-only as the oracle.
- Behaviour notes: `gdm restore --force` now refuses a current PAM file larger than
  `MAX_PAM_FILE_BYTES` (it must be read to be compared at rename time); the not-UTF-8 refusal
  without `--force` keeps its message. A log file that shrinks while the tail is read is an
  `UnexpectedEof` error instead of a shorter output.
- Docs: `Docs/DAEMON.md` (keys table, warnings), `Docs/CAMERA_V4L_CRATE.md` (opt-in honoured by
  the daemon and the enrollment CLI, NUL refusal), `Docs/ENROLLMENT_CLI.md`,
  `Docs/DISTRIBUTION_DEPLOYMENT.md` (`gdm restore` concurrency check), `AI/DECISIONS.md` (ADR
  amendment), `AI/VERIFICATION_MATRIX.md` (VCO1–VCO5, GCV6 annotation).

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) was run on this branch (§8). The layer 2 sub-agent review
is run by the orchestrator on the merged batch, not in this walkthrough's scope.

## 8. Verification Results

Results: fmt clean; clippy (workspace, all targets, all features, `-D warnings`) clean; the
five-package test run passed 1343 tests, 0 failed, 5 ignored (161 test binaries, real models
available through `SOOS_MODELS_DIR`); `soos-invariants` re-run after the documentation edits:
370 passed; i686 `cargo check` of the three touched binaries' crates clean; candid layer 1
PASSED. Commands:

```bash
export CARGO_BUILD_JOBS=6
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --all-features -p soos-daemon -p soos-camera-v4l -p soos-enrollment-cli \
  -p soos-admin-cli -p soos-invariants
ORT_SKIP_DOWNLOAD=1 cargo check --locked --target i686-unknown-linux-gnu --all-features \
  -p soos-daemon -p soos-enrollment-cli -p soos-admin-cli
./scripts/candid_review.sh
```

## 9. Known Limitations / Follow-ups

- `gdm restore` narrows, but cannot close, the window between the final re-read and
  `rename(2)` without a lock that every editor of `/etc/pam.d` honours (none exists); an edit
  landing in those few system calls is still replaced.
- The backward tail still scans a single huge final line to its start (its cost follows the
  tail region, which then is that line).
- The other items of GitHub #318 (Arch face login and faillock, `/usr` program paths, CI supply
  chain, i686 coverage note) are not part of this branch.
