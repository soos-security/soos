# Walkthrough 159 — Capture Backend Seam Below the V4L2 Supervisor

- **Date**: 2026-10-01
- **Issue**: GitHub #198 (review finding CAM-16; no backlog id, so the branch is not registered
  in `BRANCH_TO_ISSUE`) — **Branch**: `test/camera-capture-backend`
- **Matrix criteria**: CCB1–CCB15 (component `camera-capture-backend`); CHT8 promoted (RGB run)
- **ADR**: 2026-10-01 "Capture Backend Seam Below the V4L2 Supervisor" (`AI/DECISIONS.md`)

---

## 1. Context & Objectives

#198 recommended three things: an injectable device enumerator with fixture-table resolution
tests, GUI failure-path tests with a fake probe, and `#[ignore]`d hardware smoke tests behind
`SOOS_HW_TESTS=1`. State on `main` (`043d250`) before this change:

| #198 item | State on `main` | Evidence |
|---|---|---|
| Injectable enumerator, fixture-table resolution | Delivered (#152, walkthrough 110) | `CameraEnumerator`, `V4lNodeProbe`, `enumerate_capture_devices_with`; `enumeration_tests.rs`, `crates/enrollment-cli/tests/device_resolution_hermetic_tests.rs`, `camera_resolver_parity_tests.rs`, `camera_diagnostics_tests.rs` (CHT1–CHT4, CDX1–CDX6) |
| GUI failure paths (oversize, zero length, truncation, `EACCES`, `EBUSY`, daemon unavailable) | Delivered (walkthrough 110) | `crates/gui/tests/ipc_camera_failure_tests.rs` (CHT5, CHT6) |
| `SOOS_HW_TESTS=1` hardware smoke tests | Delivered, run on an RGB camera in walkthrough 152 §6, CHT8 still `⬜ Pending` | `crates/camera-v4l/tests/hardware_smoke_tests.rs` |
| Hermetic open / stream path of the daemon supervisor | **Missing** (walkthrough 110 §7, 142 §8) | Only `supervision_tests.rs`, which spawns `V4lCameraManager` on non-existent paths, so only the `ENOENT` open-failure branch ran; the streaming loop had scripted unit tests (`capture::tests`) but nothing drove `supervise` / `open_and_stream` past `open(2)` |

This change delivers the missing item: a capture-backend seam below the supervisor, with
hermetic tests of its whole state machine. The host-dependent
`crates/enrollment-cli/tests/device_resolution_tests.rs` stays next to its hermetic replacement
(existing tests are not modified).

## 2. Architect Design (Phase 1)

**Blast radius**: `crates/camera-v4l/src/v4l_impl.rs` only (plus the new test module, an
invariant file, docs). No public item is added, removed or changed: the seam is private to
`v4l_impl`, `V4lCameraManager::spawn` / `spawn_with_resolver` keep their signatures. Consumers
(`soos-daemon`, `soos-enroll`, `soos-gui`, `soos-admin`) are unaffected (grep: they only call
`spawn*` and the `CameraManager` trait).

**Types** (all private to `v4l_impl`):

```rust
struct NodeCapabilities { card: String, video_capture: bool }

trait CaptureBackend: Send + 'static {
    type Device: CaptureDevice;
    fn open_device(&self, path: &Path) -> io::Result<Self::Device>;
}

trait CaptureDevice {
    type Stream<'a>: CaptureSource where Self: 'a;   // borrows the device
    fn capabilities(&self) -> io::Result<NodeCapabilities>;            // VIDIOC_QUERYCAP
    fn pixel_formats(&self) -> Vec<FourCC>;                           // VIDIOC_ENUM_FMT
    fn frame_sizes(&self, fourccs: &[FourCC]) -> Vec<(u32, u32)>;     // ENUM_FRAMESIZES
    fn apply_format(&self, requested: &v4l::Format) -> io::Result<v4l::Format>; // S_FMT
    fn apply_frame_interval(&self, n: u32, d: u32) -> io::Result<(u32, u32)>;   // S_PARM
    fn start_stream(&self, buffer_count: u32, poll_timeout: Duration)
        -> io::Result<Self::Stream<'_>>;                              // REQBUFS + mmap
}

struct V4lBackend;  // impl CaptureBackend; `impl CaptureDevice for v4l::Device`
```

`spawn_inner`, `run_v4l_supervisor`, `supervise` and `open_and_stream` become generic over
`B: CaptureBackend`. The production implementation forwards each method to the exact call it
replaces (`v4l::Device::with_path`, `query_caps_guarded`, `enum_formats_guarded`,
`device_frame_sizes`, `set_format_guarded`, `Capture::set_params`, `mmap_stream_guarded` +
`set_timeout`). Everything else (error classification, `supervisor_sensor_hints` +
`supervisor_alias_hints`, `plan_capture_with_hints`, wire format selection, format and deep-grey
validation, frame-rate request, poll timeout, `run_capture_loop`, `guarded_v4l_drop(source)`,
`settle_stream_teardown`, backoff, re-resolution, idle suspend) stays in shared code. Teardown is
deliberately **not** a backend method: the fake stream's `Drop` panics like `Drop for
v4l::io::mmap::Stream`, so the real guard is exercised.

Names avoid the substrings `query_caps(`, `enum_formats(`, `set_format(` scanned by CVF4. The
stream type borrows the device (GAT), so the borrow checker keeps "stream torn down before the
device closes". The only reordering is that `dqbuf_poll_timeout` (a pure function of the granted
rate) is computed before the stream is created and passed to `start_stream`, which applies it
before the first DQBUF exactly as before.

**Sentinels / bounds**: unchanged (`buffer_count` stays 4; poll timeout 150–250 ms; stall budget
`MAX_STREAM_STALL`; backoff `min_backoff`…`max_backoff`; `idle_timeout = 0` disables standby).
**Latency budget**: not touched (no change on the authentication path, one generic dispatch
monomorphised to the same calls). **Fail-closed**: unchanged.

## 3. Plan Evaluation

Condensed in this single-agent run (no separate plan-evaluator report). Checked against the
code: invariants CVF4 (no raw `v4l` metadata calls outside the guard, non-recursive scan of
`crates/camera-v4l/src`), CVF5 / CAG1 (`let hints = supervisor_sensor_hints(` then `let hints =
supervisor_alias_hints(`, `DEFAULT_BY_ID_DIR`), CAG4 (`mmap_stream_guarded(`,
`guarded_v4l_drop(source)`, `CameraError::StreamTeardown`, no `Stream::with_buffers(`), RFX7, and
the matrix citation resolver (a `v4l_impl::supervisor_tests::test_*` citation resolves through
the parent directory and file stem). Placing the tests in `src/v4l_impl/supervisor_tests.rs`
keeps them out of the CVF4 scan and lets them call the private `spawn_inner`, so no test-only
public API is needed.

## 4. Tester Contract (Phase 2)

| Test | Criterion | Red evidence |
|---|---|---|
| `v4l_impl::supervisor_tests::test_ccb_open_failure_backs_off_then_recovers` | CCB1 | Compile failure (below) |
| `v4l_impl::supervisor_tests::test_ccb_busy_device_is_retried_on_the_same_path` | CCB2 | idem |
| `v4l_impl::supervisor_tests::test_ccb_frames_flow_stamped_with_the_sensor_type` | CCB3 | idem |
| `v4l_impl::supervisor_tests::test_ccb_stall_escalates_to_starved_then_reopens` | CCB4 | idem |
| `v4l_impl::supervisor_tests::test_ccb_vanished_device_is_reresolved_to_the_new_node` | CCB5 | idem |
| `v4l_impl::supervisor_tests::test_ccb_vanished_device_without_resolver_keeps_its_path` | CCB6 | idem |
| `v4l_impl::supervisor_tests::test_ccb_unsupported_node_is_reresolved` | CCB7 | idem |
| `v4l_impl::supervisor_tests::test_ccb_idle_suspend_releases_the_device_and_resumes_on_activity` | CCB8 | idem |
| `v4l_impl::supervisor_tests::test_ccb_teardown_failure_recovers_instead_of_dying` | CCB9 | idem |
| `v4l_impl::supervisor_tests::test_ccb_shutdown_releases_the_device_within_budget` | CCB10 | idem |
| `capture_backend_contract::test_ccb_supervisor_reaches_the_device_only_through_the_backend_seam` | CCB11 | Assertion failure: `v4l_impl.rs must contain 'trait CaptureBackend'` |
| `capture_backend_contract::test_ccb_decision_recorded` | CCB12 | Assertion failure: `AI/DECISIONS.md must record the capture-backend seam decision` |

Red for the supervisor tests: `cannot find trait 'CaptureBackend'`, `cannot find trait
'CaptureDevice'`, `cannot find type 'NodeCapabilities'` (exactly the specified API).

The fake (`FakeBackend` / `FakeDevice` / `FakeStream`) scripts per node: `open(2)` errnos, `EBUSY`
at `VIDIOC_S_FMT`, card name, `VIDEO_CAPTURE` flag, fourccs, granted stride, a stream script per
start (`Frames`, `VanishAfter` → `ENODEV` on DQBUF and the node removed, `Stall` → every poll waits
its full timeout), and teardown panics. It logs opens (with time), open handles, streams started,
teardowns, `VIDIOC_S_FMT` requests and resolver calls. Paths live under a directory that never
exists, so the alias lookup finds nothing and `/dev` is never touched. The tests observe the
supervisor only through `CameraManager` (`health`, `status`, `latest_frame`,
`current_device_path`). Frames are synthetic fill bytes.

Mutation check (each mutation applied to `v4l_impl.rs`, then restored byte for byte):

| Mutation | Tests that failed |
|---|---|
| backoff no longer doubles; frames always stamped `Rgb`; re-resolution never switches path | CCB1, CCB3, CCB5, CCB7 |
| stream dropped without `guarded_v4l_drop` | CCB9, CCB10 |

Migrated existing tests: **none**. No existing test file or assertion was edited; the only
change to an existing test-support file is the new module declaration in
`tests/invariants/src/lib.rs` (and `#[cfg(test)] mod supervisor_tests;` at the end of
`v4l_impl.rs`).

Flakiness: the 10 supervisor tests were run 10 times in a row (all green, 2.3 s each) and once
with `--test-threads=1` (green, 4.1 s). Timing assertions are lower bounds (backoff gaps,
`MAX_STREAM_STALL`) or the product budget (`C9_SHUTDOWN_BUDGET`, 500 ms), never a local
measurement.

## 5. Auditor Constraints (Phase 3)

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| 1 | No `unwrap` / `expect` / `panic!` / indexing in new production code | `V4lBackend`, `impl CaptureDevice for v4l::Device`, generic supervisor | clippy `-D warnings`, review |
| 2 | No new `unsafe` | `v4l_impl.rs` | diff (the only `unsafe` stays `monotonic_nanos`) |
| 3 | Every panicking `v4l` call stays guarded; teardown stays `guarded_v4l_drop(source)` in shared code | `V4lBackend` methods, `open_and_stream` | CVF4, CAG4, CCB11 invariants |
| 4 | Error classification unchanged (`from_io_error` at open, `QueryCapabilities`, `from_ioctl_error` for `S_FMT` / stream creation and DQBUF) | `open_and_stream` | existing `error_recovery_tests`, CCB1, CCB2, CCB5 |
| 5 | The stream is torn down before its device closes | `CaptureDevice::Stream<'a>` | borrow checker (GAT borrows the device) |
| 6 | Bounded DQBUF wait applied before the first dequeue | `start_stream` | code review, CCB10 shutdown budget |
| 7 | Production can never run the fake: seam private, fake only under `#[cfg(test)]`, both spawn functions pass `V4lBackend` | `v4l_impl.rs` | CCB11 |
| 8 | No frame content, path secrets or panic payload logged or stored by the tests or the seam | fake, seam | review; fake asserts on sizes and synthetic fill bytes only |
| 9 | Fail-closed unchanged: frames withdrawn on every error, teardown failure `Recovering` not `Dead`, supervisor panic still `Dead` | `supervise` | CCB4–CCB10, `supervision_tests::test_supervisor_panic_marks_camera_dead` |
| 10 | No new dependency | `Cargo.toml` | diff |

Pre-existing finding: `device_frame_sizes` called `Capture::enum_framesizes` outside the guard;
in `v4l` 0.14 it converts with `TryFrom` and does not panic, but its loop runs until the driver
returns an error (our collection was bounded by `MAX_FRAME_SIZE_HINTS`, the driver loop was
not). Fixed on this branch at the coordinator's request, see §10. Clearance: CLEARED.

## 6. Implementation (Phase 4)

- `crates/camera-v4l/src/v4l_impl.rs`: `NodeCapabilities`, `CaptureBackend`, `CaptureDevice`,
  `V4lBackend`, `impl CaptureDevice for v4l::Device`; `spawn_inner`, `run_v4l_supervisor`,
  `supervise`, `open_and_stream` generic over the backend; `#[cfg(test)] mod supervisor_tests;`.
- `crates/camera-v4l/src/v4l_impl/supervisor_tests.rs`: fake backend and the 10 CCB tests.
- `tests/invariants/src/capture_backend_contract.rs` (+ module declaration): CCB11, CCB12.
- Docs: `Docs/CAMERA_V4L_CRATE.md` (section "Hermetic Supervisor Tests", mapping row), ADR,
  matrix component `camera-capture-backend` (inserted after `camera-alias-and-v4l-guard-followups`)
  and CHT8 promoted with the RGB run.

Real-hardware check (owner's laptop camera, Azurewave 13d3:54bf "Integrated Camera", RGB only,
`uvcvideo`, `/dev/video0` + metadata node `/dev/video1`; no frame stored or logged):

- `SOOS_HW_TESTS=1 cargo test --locked -p soos-camera-v4l --test hardware_smoke_tests --
  --ignored`: 2 passed (enumeration; resolved camera streamed a frame through `V4lBackend`).
- `SOOS_HW_TESTS=1 cargo test --locked -p soos-camera-v4l --test capture_validation_tests --
  --ignored`: 1 passed (`test_v4l_streaming_drop_completes_within_budget_on_hardware`, C9 drop
  budget on a real stream).
- A temporary, uncommitted, deleted ignored test spawned `V4lCameraManager::spawn` on
  `/dev/video0` with a 1.5 s idle timeout: first frame 640x480 `Yuyv` stamped `Rgb`, frames kept
  flowing, auto-standby after 1.66 s (guarded teardown of a real stream), resumed on activity in
  0.30 s with health `Streaming`, never `Dead`, `Drop` joined in 0.20 s (< 500 ms C9).

## 7. Candid Review

Not run in this batch (the coordinator runs the independent Layer 2 review). Layer 1
(`./scripts/candid_review.sh`) result in §8.

## 8. Verification Results

```bash
export CARGO_BUILD_JOBS=4
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -p soos-camera-v4l -p soos-daemon \
  -p soos-invariants -- -D warnings
cargo test --locked --all-features -p soos-camera-v4l -p soos-daemon -p soos-invariants
ORT_SKIP_DOWNLOAD=1 cargo check --locked -p soos-camera-v4l --all-targets --all-features \
  --target i686-unknown-linux-gnu
SOOS_HW_TESTS=1 cargo test --locked -p soos-camera-v4l --test hardware_smoke_tests -- --ignored
SOOS_HW_TESTS=1 cargo test --locked -p soos-camera-v4l --test capture_validation_tests -- --ignored
./scripts/candid_review.sh
```

All green: 897 tests passed, 0 failed, 3 ignored (the three `SOOS_HW_TESTS` hardware tests)
across the three packages; the i686 check, the three hardware tests (run separately on the RGB
camera) and candid Layer 1 passed.

## 9. Known Limitations / Follow-ups — the IR-only remainder of #198

Everything in #198 that can be proven without a camera is now hermetic. What remains needs an
**RGB+IR module** (Windows-Hello style composite `uvcvideo` device, and ideally a node exposing
`Y8I` / `Y10` / `Y12` / `Y16`), which the owner does not have. Proposed hardware-validation issue
items, each with its hermetic counterpart:

1. **Smoke tests on RGB+IR** — run both `hardware_smoke_tests` with `SOOS_HW_TESTS=1`: the scan
   must list the IR capture node with formats, default `prefer_ir` auto-resolution must pick the
   IR node (through its `/dev/v4l/by-id/` alias, never the metadata node), and the streamed frame
   must be stamped `Infrared` (counterparts CHT3, CCB3; extends CHT8 beyond RGB).
2. **Real IR classification inputs** — record `soos-admin camera list --json` on the module
   (truncated card names, by-id IR tokens, IR frame-size signature, shared by-id stems) and
   attach it to CDX2 / CDX6 (walkthroughs 142 §8, 149 §9).
3. **Supervisor stamp on a plain `/dev/videoN` IR path** — `camera_device = "/dev/videoN"`
   pointing at the IR node: frames must be stamped `Infrared` through `supervisor_alias_hints`,
   including a node that streams a colour format under a neutral card name (counterparts CAG1,
   CAG2; walkthrough 152 §9).
4. **Real deep-greyscale capture** — on a `Y8I` / `Y10` / `Y12` / `Y16` node: the driver-granted
   stride, `validate_deep_grey_format` and `DeepGreyFormat::to_grey8` on real buffers, frames
   `Grey` 8-bit (counterparts CCP rows, CCB3 `Y16 ` case).
5. **IR node lifecycle under `uvcvideo`** — `EBUSY` while another process (an IR emitter tool or
   another face-auth service) streams the IR node, idle standby / resume of the IR stream, and
   suspend / replug of the composite module with re-resolution to the renumbered IR node
   (counterparts CCB2, CCB5, CCB8; CSH1–CSH4).

Not hardware-reproducible on demand (any camera): a real `VIDIOC_STREAMOFF` failure (teardown
panic, CCB9 / CAG4) and the double stream + arena teardown panic (abort, ADR 2026-10-01 "Evidence
Sweep Lists Partitions Through Their Descriptor; v4l Double Teardown Panic Left Upstream").
The IR emitter activation itself (`Docs/CAMERA_V4L_CRATE.md`, "IR Sensors and Emitter
Requirements") is adjacent to, not part of, #198.

The `VIDIOC_ENUM_FMT` loop that had the same "until the driver errors" shape as the
frame-size loop is bounded too (§11).

## 10. Addendum — Guarded, Bounded `VIDIOC_ENUM_FRAMESIZES`

Coordinator request after the first hand-off: close the §5 finding on this branch.

- **Design**: `v4l_guard::enumerate_indexed_bounded(max_indices, query)` walks a V4L2
  `VIDIOC_ENUM_*` index list: `Ok(Some)` entry, `Ok(None)` skipped entry (still counted), an
  error ends the list (returned only at index 0, as in `v4l` 0.14), and at most `max_indices`
  queries are issued (`truncated = true` when the bound, not the driver, stopped it; always the
  first indices, so deterministic). `v4l_guard::enum_framesizes_guarded(device, fourcc,
  max_indices)` issues `VIDIOC_ENUM_FRAMESIZES` itself through `v4l::v4l2::ioctl` (two
  `unsafe` blocks with `// SAFETY:` comments: zeroed POD `v4l2_frmsizeenum`, descriptor owned by
  the device) inside `guard_v4l_call`, converting with `FrameSizeEnum::try_from`.
  `sensor::device_frame_sizes(device, device_path, fourccs)` uses it with the existing
  `MAX_FRAME_SIZE_HINTS` (64) as per-fourcc index bound (a fourcc cannot contribute more distinct
  sizes than the total bound) and logs a truncation at debug level with the node path only. The
  `CaptureDevice::frame_sizes` seam method gained the path argument (logging only); callers in
  `sensor.rs`, `diagnostics.rs` and `v4l_impl.rs` pass it.
- **Red**: `v4l_guard::bounded_enumeration_tests` (4 tests) failed to compile (`cannot find
  function enumerate_indexed_bounded`); the new invariant
  `capture_backend_contract::test_ccb_frame_size_enumeration_is_guarded_and_bounded` failed with
  `unbounded v4l enum_framesizes called in: ["crates/camera-v4l/src/sensor.rs"]`. Both green
  after the change. The invariant **adds** a check next to CVF4 (whose raw-call list is not
  edited): no `enum_framesizes(` call in any `crates/camera-v4l/src` file, the guard defines the
  bounded wrapper, `sensor.rs` uses it with `MAX_FRAME_SIZE_HINTS`.
- **Setup-only edit of a test written on this branch**: the fake `frame_sizes` in
  `v4l_impl/supervisor_tests.rs` takes the new, ignored path argument. No assertion changed.
- **Hardware**: `soos-admin camera --format json probe /dev/video0` lists `1280x720, 960x540,
  640x480, 640x360, 320x240, 320x180`, identical to `v4l2-ctl --list-framesizes` for `MJPG` and
  `YUYV`; the three `SOOS_HW_TESTS` tests pass again.
- **Gates**: `cargo fmt --all -- --check`; `cargo clippy --locked --all-targets --all-features
  -p soos-camera-v4l -p soos-daemon -p soos-admin-cli -p soos-invariants -- -D warnings`;
  `cargo test --locked --all-features -p soos-camera-v4l -p soos-daemon -p soos-admin-cli
  -p soos-invariants` (1029 passed, 0 failed, 3 ignored hardware tests); the i686 check;
  `./scripts/candid_review.sh` (unsafe additions reported as confined to the adapter crate). All
  green.

## 11. Addendum — Guarded, Bounded `VIDIOC_ENUM_FMT`

Second coordinator request: the same treatment for format enumeration.

- **Design**: `v4l_guard::enum_formats_guarded(device, device_path)` (name kept: CVF4 requires
  it) no longer calls `v4l::video::Capture::enum_formats`. It issues `VIDIOC_ENUM_FMT`
  (`V4L2_BUF_TYPE_VIDEO_CAPTURE`) itself inside `guard_v4l_call`, reading only `pixelformat`, so
  the UTF-8 description `v4l` unwraps is never decoded. The private
  `bounded_format_fourccs(device_path, query)` runs it through
  `enumerate_indexed_bounded(MAX_ENUMERATED_FORMATS, ..)`: index-0 error → empty list (as `v4l`
  0.14 and the previous wrapper), first later error → end, at most `MAX_ENUMERATED_FORMATS` (64,
  a new named constant equal to `MAX_DIAGNOSTIC_FOURCCS`, so the diagnostics truncation is
  unchanged) indices, the first ones kept, truncation logged at debug level with the node path
  only. Two new `unsafe` blocks (zeroed POD `v4l2_fmtdesc`, ioctl on the device's own
  descriptor) carry `// SAFETY:` comments. The `CaptureDevice::pixel_formats` seam method gained
  the path argument (logging only); every caller in `sensor.rs` (`SystemV4lNodeProbe`,
  `frame_sizes_at`), `diagnostics.rs` and `v4l_impl.rs` passes it. No `enum_formats(` call is left
  in `crates/camera-v4l/src`.
- **Red**: two new tests in `v4l_guard::bounded_enumeration_tests`
  (`test_ccb_endless_format_enumeration_is_truncated_at_the_bound`,
  `test_ccb_format_enumeration_end_error_and_panic_semantics`) failed to compile (`cannot find
  function bounded_format_fourccs`, `cannot find value MAX_ENUMERATED_FORMATS`); the new
  invariant `capture_backend_contract::test_ccb_format_enumeration_is_guarded_and_bounded`
  (CCB15, added, CVF4 untouched) failed with `unbounded v4l enum_formats called in:
  ["crates/camera-v4l/src/v4l_guard.rs"]`. All green after the change.
- **Setup-only edits**: the fake `pixel_formats` in `v4l_impl/supervisor_tests.rs` takes the
  new, ignored path argument; `clippy::indexing_slicing` and `clippy::arithmetic_side_effects`
  were added to the `#[allow]` of the branch's own `bounded_enumeration_tests` module for the two
  new tests. No assertion changed.
- **Hardware** (`/dev/video0`, RGB-only): `soos-admin camera --format json probe /dev/video0`
  reports fourccs `MJPG`, `YUYV` (identical to `v4l2-ctl --list-formats`), the same six frame
  sizes, sensor `rgb`; `/dev/video1` (metadata node) stays `not_video_capture` with no format;
  the three `SOOS_HW_TESTS` tests pass.
- **Gates**: fmt, clippy (`-p soos-camera-v4l -p soos-daemon -p soos-admin-cli -p
  soos-invariants`, `-D warnings`), `cargo test --locked --all-features` on the same four packages
  (1032 passed, 0 failed, 3 ignored hardware tests), the i686 check and
  `./scripts/candid_review.sh` (unsafe additions confined to the adapter crate): all green.
