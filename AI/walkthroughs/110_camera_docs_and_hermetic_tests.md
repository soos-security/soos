# Walkthrough 110 — Camera Documentation Drift and Hermetic Camera Tests

- **Issues**: GitHub #197 (review finding CAM-15, doc drift) and #198 (CAM-16, test gap)
- **Branch**: `test/p2-camera-docs-hermetic-tests`
- **Matrix rows**: CHT1–CHT8 (component `camera-hermetic-tests`)
- **ADR**: 2026-09-30 "Hermetic V4L2 Enumeration and `/dev/videoN` Auto-Selection"

## 1. Starting Point

The review (base `fbb99c4`) reported that the camera documentation cited wrong defaults and
missing tests, and that nothing covered enumeration, selection or the GUI failure paths without
host hardware. Since then, several items had already been fixed on `main`:

| Review item | State on `main` before this change |
|---|---|
| Matrix rows CAM1 / ASG3 / GARP1 cite non-existent tests | Fixed by TCI-04 (#187); enforced by `matrix_citations` |
| `default` sentinel only in walkthrough 73 | Fixed: `is_auto_camera_device` accepts `""`, `auto`, `default` (#152) |
| No injectable enumerator for resolution | Partially fixed: `CameraEnumerator` trait + `camera_resolver_parity_tests` (#152) |
| `auto_format` documented `false`, code `true` | **Still wrong** |
| `idle_timeout` documented 60 s, code 10 s | **Still wrong** |
| C5 "drops 15–30 frames" while `daemon.toml` defaults to 0 | **Still undocumented** |
| `Docs/ENROLLMENT_CLI.md` claims by-id resolution unconditionally | **Still wrong** (no `/dev/videoN` fallback) |
| `test_enumerate_filters_empty_format_nodes` (BACKLOG) | **Missing**; the sysfs scan could not be tested hermetically |
| `test_resolve_camera_device_default_resolution` (walkthrough 75) | **Missing** |
| `device_resolution_tests` host-dependent | Still host-dependent (asserts only `starts_with("/dev")`) |
| GUI oversize / unavailable / `EACCES` / direct `EBUSY` paths | **Untested** |
| Hardware smoke tests | **Missing** |

## 2. Specification

- `crates/camera-v4l/src/sensor.rs`:
  - `V4lNodeCapabilities { card_name, video_capture, supported_formats }`;
  - `trait V4lNodeProbe { fn probe(&self, dev_path: &Path) -> Option<V4lNodeCapabilities> }`;
  - `SystemV4lNodeProbe` (real `VIDIOC_QUERYCAP` / `VIDIOC_ENUM_FMT`);
  - `enumerate_capture_devices_with(sysfs_dir, dev_dir, &dyn V4lNodeProbe)`;
  - constants `SYSFS_VIDEO4LINUX_DIR`, `MAX_SYSFS_ENTRIES` (256), `MAX_VIDEO_NODES` (64);
  - `enumerate_capture_devices()` becomes a thin wrapper (same public signature).
- Behaviour: only `video<decimal>` entries (strict ASCII digits, so `video+1` is rejected even
  though `u32::from_str` would accept it), numeric order, bounded scan and probe count, listed
  only with `VIDEO_CAPTURE` and at least one decodable format, missing sysfs → empty list.
- Decision (ADR): the resolver returns `/dev/videoN` when the selected node has no by-id alias;
  documents must not claim a by-id path unconditionally. The daemon `daemon.toml` warmup default
  of 0 (matrix CLP2, deliberate) is documented, not changed.

## 3. Tests First (Red Evidence)

| Test file | Red result on the unmodified code |
|---|---|
| `tests/invariants/src/camera_docs_contract.rs` | 3 of 5 failed: `auto_format` `false` vs `true`, `idle_timeout` `60s` vs `10s`, `format` / `sensor_preference` undocumented; C5 row silent about `daemon.toml`; `Docs/ENROLLMENT_CLI.md` missing `/dev/videoN` |
| `crates/camera-v4l/tests/enumeration_tests.rs` (9 tests) | Compile failure: `enumerate_capture_devices_with`, `V4lNodeCapabilities`, `V4lNodeProbe`, `MAX_VIDEO_NODES` unresolved |
| `crates/enrollment-cli/tests/device_resolution_hermetic_tests.rs` (3 tests) | Passed immediately: characterization tests pinning the existing resolver (coverage gap, not a behaviour bug) |
| `crates/gui/tests/ipc_camera_failure_tests.rs` (7 tests) | Passed immediately: characterization tests of existing, previously untested failure paths |
| `crates/camera-v4l/tests/hardware_smoke_tests.rs` (2 tests) | `#[ignore]` + `SOOS_HW_TESTS=1`; not part of the red/green contract |

No pre-existing test was modified.

## 4. Audit

- No `unwrap`/`expect` in production code; the new scan uses `let ... else` and `Option` chains.
- Bounded I/O: `read_dir(...).take(MAX_SYSFS_ENTRIES)` and `.take(MAX_VIDEO_NODES)` probes.
- No new `unsafe`; `soos-camera-v4l` stays an adapter crate, business crates untouched.
- Nothing logged by the new code; the smoke tests assert only dimensions and counts, never write
  or log frames.
- The GUI tests use private temporary socket directories and never widen socket permissions.

## 5. Implementation

`enumerate_capture_devices_with` in `crates/camera-v4l/src/sensor.rs`, re-exported from
`lib.rs`. Documentation changes: `Docs/CAMERA_V4L_CRATE.md` (defaults, `daemon.toml` note, C5 row,
hermetic enumeration and smoke-test sections, CHT mapping row), `Docs/ENROLLMENT_CLI.md`
(`/dev/videoN` and sentinel fallback), `Docs/GUI_APPLICATION.md` (failure-path tests).

## 6. Verification

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features --no-fail-fast
./scripts/candid_review.sh
SOOS_HW_TESTS=1 cargo test -p soos-camera-v4l --test hardware_smoke_tests -- --ignored \
  test_hw_enumeration_lists_a_capture_node_with_formats   # passed on a laptop with an RGB camera
```

The streaming smoke test (`test_hw_resolved_camera_streams_a_frame`) has not been run yet; CHT8
stays `⬜ Pending`.

## 7. Follow-ups

- Run `test_hw_resolved_camera_streams_a_frame` on an RGB+IR laptop and on an RGB-only webcam,
  then promote CHT8.
- The daemon supervisor's open/stream path (`V4lCameraManager` MMAP loop) still has no hermetic
  fake below the `CameraManager` trait; a `CaptureBackend` seam would be needed for that.
- `test_probe_preview_reports_io_when_socket_permission_denied` returns early when run as root.
