# Walkthrough 142 — Camera Diagnostics Command, Explained Resolution and Shared By-Id Stems

- **Date**: 2026-10-01
- **Issues**: GitHub #256 (CAM-17, no camera diagnostics command), #195 (CAM-13, IR/RGB
  classification heuristics), #198 (CAM-16, no hermetic camera tests)
- **Branch**: `fix/p3-camera-diagnostics`
- **Matrix rows**: CDX1–CDX11 (component `camera-diagnostics`)
- **ADR**: 2026-10-01 "Metadata-Only Camera Diagnostics, Explained Resolution and Shared By-Id
  Stems" (`AI/DECISIONS.md`)

---

## 1. Starting Point

Walkthroughs 109 and 110 already delivered most of #195 and #198: the ordered sensor scorer
with by-id and frame-size hints, deep-greyscale IR formats, the injectable `V4lNodeProbe` for
hermetic enumeration, fixture-based resolver tests, GUI failure-path tests and `#[ignore]`d
hardware smoke tests. What was still missing:

| Item | State on `main` (`abd6016`) |
|---|---|
| `soos-admin camera list\|probe` (#256) | Missing: `soos-admin` only printed the `camera_ready` boolean of `status` |
| Resolver decision with a reason (#256: "print `selected` + `reason`") | Missing: `resolve_camera_device` returned only the path, source and sensor type |
| `bus_info`, `driver`, `device_caps`, all fourccs, frame sizes per node (#195 recommendation, #256) | Not collected anywhere a user could see |
| Composite module whose USB product string carries `IR` (candid finding 3 of walkthrough 109) | Open: both nodes classified Infrared by the shared by-id stem, `PreferIr` picked the first (often RGB) node |
| Log when the weakest rule (frame-size signature) alone decides (candid finding 6 of walkthrough 109) | Open |
| Probe failures (`EACCES`, `EBUSY`) visible without code reading (#198) | Unprobeable nodes were silently skipped |

## 2. Specification

`soos-camera-v4l`:

- `sensor.rs`: `ClassificationReason` (6 rules, `as_str()` snake_case labels) and
  `explain_sensor_classification(card, formats, hints) -> (SensorType, ClassificationReason)`;
  `classify_sensor_with_hints` now delegates to it. `scan_video_node_names` (crate-private) is
  the bounded sysfs scan shared by the enumeration and the diagnostics (behaviour unchanged).
- `resolver.rs`: `by_id_stem`, `CandidateClassification`, `SelectionReason`,
  `CameraResolutionReport`, `explain_camera_resolution(explicit, preference, enumerator)`.
  `resolve_camera_device` = `explain_camera_resolution(..).resolution` plus the log lines; the
  selection still uses `select_camera_device_with` (single fallback order). A by-id name whose
  stem another capture candidate shares is withheld from the classifier.
- `diagnostics.rs` (new): `V4lNodeDetails`, `ProbeFailure` (`from_io_error`), trait
  `V4lDeviceProbe`, `SystemV4lDeviceProbe` (QUERYCAP / ENUM_FMT / ENUM_FRAMESIZES only),
  `NodeStatus`, `NodeDiagnostics`, `CameraDiagnostics`, `collect_camera_diagnostics`,
  `probe_camera_node`, `device_caps_flag_names`, `fourcc_label`, `sanitize_v4l_text`,
  `sanitize_display_text`, constants `MAX_DIAGNOSTIC_FOURCCS` (64), `MAX_V4L_TEXT_CHARS` (32) and
  the `V4L2_CAP_*` bits.

`soos-admin-cli`:

- `args.rs`: `Commands::Camera(CameraArgs)`, `CameraAction::{List, Probe}`, `CameraListArgs
  { json, sensor_preference, device }` (the preference is parsed with the shared
  `parse_sensor_preference`), `CameraProbeArgs { device, json }`.
- `camera.rs` (new): `CameraEnvironment` (sysfs, dev and by-id directories), `NodeReport`,
  `CameraListReport`, `CameraProbeReport` (`from_*`, `format_table`, `to_json` via `serde_json`,
  `exit_code`), `collect_list_report`, `probe_report`. `main.rs` wires the command; the global
  `--format json` works as well as `--json`.

Why a second probe trait instead of extending `V4lNodeProbe`: `V4lNodeCapabilities` and
`CameraDeviceInfo` are built with struct literals by pre-existing contract tests, so new fields
would require editing those tests (forbidden). The diagnostics hand their probed candidates to the
shared resolver through a private `CameraEnumerator` adapter, so both paths agree by construction.

## 3. Tests First (Red Evidence)

| Test file | Red result before the implementation |
|---|---|
| `crates/camera-v4l/tests/camera_diagnostics_tests.rs` (15 tests) | Compile failure: unresolved `soos_camera_v4l::diagnostics`, `by_id_stem`, `explain_camera_resolution`, `explain_sensor_classification`, `ClassificationReason`, `SelectionReason` |
| `crates/admin-cli/tests/camera_command_tests.rs` (9 tests) | Compile failure: unresolved `soos_admin_cli::camera`, `soos_admin_cli::args::CameraAction`, `soos_camera_v4l::diagnostics` |
| `tests/invariants/src/camera_diagnostics_contract.rs` (3 tests) | 2 failed: `crates/camera-v4l/src/diagnostics.rs` / `crates/admin-cli/src/camera.rs` missing (the detector self-test was added with the invariant) |

Behavioural red for the shared-stem rule: with the implementation in place but the by-id name
passed to the classifier unconditionally, `test_cdx_shared_by_id_stem_ir_token_is_not_decisive`
failed with `left: ".../usb-Acme_HD_IR_Camera_0001-video-index0"`, `right:
".../usb-Acme_HD_IR_Camera_0001-video-index2"` (the pre-change behaviour); restoring the rule
turned it green. No pre-existing test was modified.

## 4. Audit

- No `unwrap`/`expect`/`panic`/indexing in the new production code; string slicing in
  `by_id_stem` uses `get` and `checked_add`. No PAM code touched, no Tokio or OpenCV added, no new
  `unsafe` (`soos-admin-cli` keeps `#![forbid(unsafe_code)]` and gains only a dependency on
  `soos-camera-v4l`, never on `v4l` directly).
- Bounded: sysfs scan (`MAX_SYSFS_ENTRIES` / `MAX_VIDEO_NODES`), by-id scan (the single bounded
  scanner `SystemCameraEnumerator::by_id_aliases`), fourccs (`MAX_DIAGNOSTIC_FOURCCS`), frame sizes
  (`MAX_FRAME_SIZE_HINTS`), strings (`MAX_V4L_TEXT_CHARS`).
- Non-biometric: only metadata ioctls; no format set, no buffer mapped, no frame read; enforced
  by the `camera_diagnostics_contract` invariant. Nothing sensitive is logged; the resolver logs
  device paths and rule labels only.
- Terminal safety: a USB device chooses its own product string, so control characters and
  bidirectional overrides are replaced before printing (card, driver, bus_info and every path);
  JSON is produced by `serde_json`.
- Upstream limitation: `v4l` 0.14 `Capabilities::from` unwraps `str::from_utf8` on the
  card/driver/bus fields, so a device with a non-UTF-8 card name panics inside the `v4l` crate.
  `SystemV4lDeviceProbe::details` runs the ioctls under `catch_unwind` and reports such a node as
  `probe_failed: io_error`. The pre-existing `SystemV4lNodeProbe` (daemon enumeration) has the same
  exposure and is not changed here (follow-up).

## 5. Implementation Notes

- The resolver's classification pass computes all by-id names first, then marks names whose stem
  occurs more than once among the capture candidates. Metadata nodes are not candidates, so the
  usual `-index0` / `-index1` pair of one UVC function never counts as shared.
- `collect_camera_diagnostics` runs `explain_camera_resolution` twice over the in-memory
  inventory: once without the explicit device (to classify every candidate) and once with it
  (the decision). No device is probed twice.
- `camera probe` classifies one node in isolation; the shared-stem rule needs the siblings and
  is applied by `camera list` only (documented).

## 6. Manual Hardware Check (not CI evidence)

On the development laptop (one RGB `Integrated Camera`, `uvcvideo`, `usb-0000:00:14.0-8`):
`soos-admin camera list` listed `/dev/video0` as `candidate` (MJPG, YUYV, 1280x720 down to
320x180, `rgb (colour_formats)`) and `/dev/video1` as `not_video_capture` (`META_CAPTURE`), then
`selected: /dev/v4l/by-id/usb-Azurewave_Integrated_Camera_SN0001-video-index0`, `reason:
fallback_first_candidate` (no IR sensor, `prefer_ir`), exit status 0. `camera probe` of the by-id
link probed `/dev/video0`; `camera probe /dev/video9` printed `probe_failed: not_found` and exited
with status 1. No IR module was available.

## 7. Verification

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features --no-fail-fast
./scripts/candid_review.sh
```

Results are recorded in the branch report.

## 7a. Candid review follow-up: probe only V4L2 nodes (CDX11)

`soos-admin camera probe <DEVICE>` accepts an arbitrary path and often runs as root. The
production `SystemV4lDeviceProbe` now calls `ensure_v4l2_char_device` before `open(2)`: a path
that is not a character device with major 81 is refused with `ENOTTY`, so probing a tape drive,
tty or regular file has no side effect. Proven by
`test_cdx_system_probe_refuses_non_v4l2_nodes_before_open`; `camera probe /dev/video0` still
reports the real laptop camera.

## 8. Follow-ups

- Run `soos-admin camera list` on RGB+IR hardware (Windows-Hello module advertising YUYV,
  Y16/Y8I RealSense-style node, composite module with `IR` in its USB product string) and attach
  the JSON to the matrix rows CDX2/CDX6.
- Let `camera list` read `camera_device` and `sensor_preference` from `/etc/soos/daemon.toml`
  (today `--device` / `--sensor-preference` mirror them) and call it from `scripts/install.sh`.
- Apply the shared-stem rule in the capture supervisor (it classifies only the opened node).
- The daemon supervisor's open/stream path still has no hermetic fake below `CameraManager`
  (a `CaptureBackend` seam), carried over from walkthrough 110; #198 stays partial for it.
- The host-dependent `crates/enrollment-cli/tests/device_resolution_tests.rs` remains alongside
  its hermetic replacement (pre-existing tests are not modified).
- `v4l` 0.14 panics on non-UTF-8 capability strings (see §4): guard the daemon's
  `SystemV4lNodeProbe` and capture path the same way, or patch the crate.
