# Walkthrough 152 — Camera Alias Lookup and `v4l` Guard Follow-ups

- **Date**: 2026-10-01
- **Issue**: GitHub #289 ("Camera" items 2-4; no backlog id) — **Branch**: `fix/p3fu2-camera-guards`
- **Matrix criteria**: CAG1–CAG5 (component `camera-alias-and-v4l-guard-followups`)
- **ADR**: 2026-10-01 "By-Id Alias Lookup for Plain Capture Nodes; Silent, Guarded v4l Teardown"
  (`AI/DECISIONS.md`)

---

## 1. Context & Objectives

Walkthrough 149 (§5, §9) left three camera follow-ups, listed in #289:

| # | Item | State on `main` (`789beb7`) |
|---|---|---|
| 1 | `camera_device` given as a plain `/dev/videoN` path | The supervisor looked up no by-id alias, so an IR node streaming a colour format under a neutral card name could be stamped `Rgb` |
| 2 | Caught `v4l` panic | `guard_v4l_call` converted the panic, but the process panic hook ran first (daemon: panic location logged; admin CLI: Rust's default panic line on stderr) |
| 3 | `v4l` panic in stream teardown | `Drop for mmap::Stream` (failing `VIDIOC_STREAMOFF`) was not guarded: the supervisor's `catch_unwind` marked the camera `Dead` until restart |

The other #289 items (`daemon.toml` readers, admin/GUI parity, storage, packaging) belong to
other batches and are out of scope.

## 2. Architect Design (Phase 1)

**Blast radius**: `soos-camera-v4l` only (`v4l_guard.rs`, `v4l_impl.rs`, `error.rs`,
`status.rs`, `sensor.rs`, `lib.rs`), plus invariants and docs. `soos-daemon`, `soos-admin-cli`,
`soos-enroll` and `soos-gui` consume the crate unchanged (the new `CameraError` variant is not
matched exhaustively outside the crate: grep).

**Item 1** — `pub fn supervisor_alias_hints(hints: SensorHints, device_path: &Path, by_id_dir:
&Path) -> SensorHints`, re-exported from the crate root. Called in `open_and_stream` right after
`let hints = supervisor_sensor_hints(...)` with `DEFAULT_BY_ID_DIR`. Only fills a missing
`by_id_name`; aliases come from `SystemCameraEnumerator::with_by_id_dir(..).by_id_aliases()`
(the single bounded scanner, invariant `test_single_bounded_by_id_scanner`), matched on the
canonical node; among several, the first with an IR token (`has_ir_token`, now `pub(crate)`),
else the smallest name. `supervisor_sensor_hints` is unchanged: the existing contract
`test_cvf_supervisor_keeps_shared_stem_ir_token_decisive` pins that the pure builder returns no
name for `/dev/video0` (and would fail on any host with a real alias if the builder read
`/dev/v4l/by-id`).

**Item 2** — in `v4l_guard.rs`: `thread_local! GUARD_DEPTH: Cell<u32>`, `static HOOK_FILTER:
Once`, `pub fn install_v4l_panic_hook_filter()` (idempotent, skipped while panicking because
`set_hook` panics there), a `DepthGuard` RAII counter, and `guard_v4l_call` installing the filter
then running the call inside the counter. The wrapper calls `previous(info)` unless the current
thread's depth is non-zero.

**Item 3** — `pub fn guarded_v4l_drop<T>(value: T) -> io::Result<()>`, `pub(crate) fn
mmap_stream_guarded<'a>(&Device, u32) -> io::Result<Stream<'a>>`, `CameraError::StreamTeardown
{ path, reason }` (kind `Io`), and the private `settle_stream_teardown(path, session, teardown)`:
streaming error wins; `Shutdown` + teardown error → `Shutdown` (logged); `Suspend` + teardown
error → `Err(StreamTeardown)` (backoff, reopen).

Latency budget: not touched (open path and teardown only; one bounded directory scan per stream
open; no change on the authentication path).

## 3. Plan Evaluation

Condensed in this single-agent run (no separate plan-evaluator report). Checks against the code:
CVF4/CVF5 invariants (`let hints = supervisor_sensor_hints(` must stay, no inline
`SensorHints {` literal, guarded wrappers only), RFX7 (no "same hints as the resolver" in
`v4l_impl.rs`), the single by-id scanner invariant (no `read_dir` outside `resolver.rs`), and the
`v4l` 0.14 sources (`Drop for Stream` always issues `VIDIOC_STREAMOFF`; `Drop for Arena` unmaps
and issues `VIDIOC_REQBUFS(0)`; both panic on any error but `ENODEV`).

## 4. Tester Contract (Phase 2)

| Test | Criterion | Red evidence |
|---|---|---|
| `crates/camera-v4l/tests/supervisor_alias_hint_tests.rs` (4 tests, fake by-id dir in `tempfile`) | CAG1, CAG2 | Compile failure: unresolved import `soos_camera_v4l::supervisor_alias_hints` |
| `crates/camera-v4l/tests/v4l_panic_hook_filter_tests.rs` (1 test, own binary: the hook is process-wide) | CAG3 | Assertion failure: `guarded panic reached the hook: ["guarded-first"]` |
| `crates/camera-v4l/tests/v4l_teardown_guard_tests.rs` (2 tests) | CAG4 | Compile failure: unresolved `v4l_guard::guarded_v4l_drop`, no variant `CameraError::StreamTeardown` |
| `v4l_impl::tests` (2 unit tests of `settle_stream_teardown`) | CAG4 | Written with the implementation (private function) |
| `tests/invariants/src/camera_alias_guard_contract.rs` (4 tests) | CAG1, CAG3–CAG5 | 4 failed: no `supervisor_alias_hints`, no `take_hook` / `set_hook` in the guard, raw `Stream::with_buffers(`, no ADR |

A real `v4l` stream needs a device, so the drop path is exercised with a value whose `Drop`
panics like the upstream one; the real teardown is covered by the hardware check in §6.

Migrated existing tests: **none**. No existing test file was edited; the only change to
`tests/invariants/src/lib.rs` is the new module declaration.

Flakiness: the three new integration test binaries were run 10 times in a row, all green.

## 5. Auditor Constraints (Phase 3)

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| 1 | No `unwrap`/`expect`/`panic!`/indexing in new production code | `v4l_guard.rs`, `v4l_impl.rs` | grep, clippy `-D warnings` |
| 2 | No new `unsafe` | camera-v4l | diff |
| 3 | The panic hook is never swapped per call nor removed; other panics always reach the previous hook (any thread) | `install_v4l_panic_hook_filter` | CAG3 test and invariant |
| 4 | The hook never panics: thread-local read with `try_with` (false → report) | wrapper hook | code review |
| 5 | `set_hook` is never called while panicking (it would panic) | `install_v4l_panic_hook_filter` | `thread::panicking()` check, CAG3 invariant |
| 6 | Panic payload never propagated nor logged (may quote device bytes) | `guarded_v4l_drop`, `StreamTeardown`, teardown `warn!` | CAG4 tests |
| 7 | Bounded alias scan (single scanner, 64 entries), no new directory scanner | `supervisor_alias_hints` | CAG1 tests, `test_single_bounded_by_id_scanner`, CAG1 invariant |
| 8 | No PAD weakening: the lookup never downgrades `Infrared`; the shared-stem rule stays out of the supervisor | `supervisor_alias_hints` | CAG2, CVF5 |
| 9 | Teardown failure stays fail-closed: frames withdrawn, `Recovering` not `Ready` | `settle_stream_teardown`, supervisor `Err` path | CAG4 unit tests, existing supervisor code |
| 10 | The stream is dropped before the device it borrows | `open_and_stream` | borrow checker |

Residual risk, documented in the ADR: a panic in `Drop for Stream` followed by a panic in its
`Arena` field during the unwind aborts the process; guarding it would require leaking the
mappings. Clearance: CLEARED.

## 6. Implementation (Phase 4)

- `crates/camera-v4l/src/v4l_guard.rs`: hook filter, depth counter, `guarded_v4l_drop`,
  `mmap_stream_guarded`; module documentation extended.
- `crates/camera-v4l/src/v4l_impl.rs`: `supervisor_alias_hints`, `settle_stream_teardown`, the
  open path uses the guarded stream creation and drops the capture source through the guard; the
  `supervisor_sensor_hints` doc points to the alias lookup.
- `crates/camera-v4l/src/error.rs` / `status.rs`: `CameraError::StreamTeardown` → `Io`.
- `crates/camera-v4l/src/sensor.rs`: `has_ir_token` is `pub(crate)`.
- Docs: `Docs/CAMERA_V4L_CRATE.md` (hint paragraph, panic-guard section, matrix mapping); ADR;
  matrix rows CAG1–CAG5.

Real-hardware check (owner's laptop camera, Azurewave/IMC 13d3:54bf "Integrated Camera", RGB only,
`uvcvideo`, by-id alias `usb-Azurewave_Integrated_Camera_SN0001-video-index0`; no frame stored):

- `SOOS_HW_TESTS=1 cargo test -p soos-camera-v4l --test hardware_smoke_tests -- --ignored`: 2
  passed (enumeration, one frame streamed through the guarded create / teardown path).
- A temporary, uncommitted ignored test spawned `V4lCameraManager` on the plain `/dev/video0`
  path with a 1.5 s idle timeout: frames stamped `Rgb` (alias found, no IR token), auto-standby
  reached after 1.65 s (guarded teardown of a streaming session), resumed on activity, health
  never `Dead`, final health `Streaming`.

## 7. Candid Review

Not run in this batch (the coordinator runs the independent Layer 2 review). Layer 1
(`./scripts/candid_review.sh`) result is in §8.

## 8. Verification Results

```bash
export CARGO_BUILD_JOBS=5
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -p soos-camera-v4l -p soos-invariants \
  -p soos-daemon -p soos-admin-cli -- -D warnings
cargo test --locked --all-features -p soos-camera-v4l -p soos-daemon -p soos-admin-cli \
  -p soos-invariants
./scripts/candid_review.sh
```

All green (pass counts in the branch report).

## 9. Known Limitations / Follow-ups

- Double panic in stream + arena teardown aborts the process (see ADR); needs an upstream `v4l`
  fix (fallible teardown) to remove.
- A host that installs its own panic hook after the first guarded call replaces the filter
  (guarded panics are then reported again; nothing is lost). `soos-daemon` installs its hook
  first.
- The alias lookup reads `/dev/v4l/by-id` once per stream open; a module whose by-id name has no
  IR token and that streams a colour format under a neutral card name is still stamped `Rgb`
  (no metadata says otherwise). IR checks on a real RGB+IR module remain an owner hardware check.
