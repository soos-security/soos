# Walkthrough 155 — `O_PATH` Config Open, test-pam Acceptance Fields, Hook Filter Re-installation and 32-bit Check

- **Date**: 2026-10-01
- **Issue**: GitHub #291 (camera / diagnostics "Suggestions" items; no backlog id) — **Branch**: `fix/p3fu3-camera-diagnostics`
- **Matrix criteria**: CDF1–CDF5 (component `config-open-and-diagnostics-followups`)
- **ADR**: 2026-10-01 "`O_PATH` Config Open, Re-installable v4l Hook Filter and test-pam Acceptance
  Fields" (`AI/DECISIONS.md`)

---

## 1. Context & Objectives

#291 lists the residual follow-ups of the #289 batch. This batch takes five of them:

| # | Item | State on `main` (`56b0350`) |
|---|---|---|
| 1 | `daemon_config.rs`: a symbolic link to a device node | The reader opened the path with `O_RDONLY \| O_NONBLOCK` before `fstat`, so the driver's `open` ran (side effects) before the handle was refused |
| 2 | `soos-admin test-pam` JSON | `verdict` shows the raw daemon verdict (possibly `Allow`) for a mismatched or stale response; only the free-text `pam_result` said it was rejected |
| 3 | `install_v4l_panic_hook_filter()` | A host hook installed after the first guarded call replaced the filter for good: calling the function again did nothing (`Once`) |
| 4 | `SYS_setresuid32` branch of the PCX test helper | Never compiled (no 32-bit target on the dev host) |
| 5 | `v4l` stream drop panic + arena drop panic | Aborts; out of scope (needs a fallible teardown upstream in `v4l`), restated in the ADR |

The GUI, `soos-enroll`, biometric-store and evidence-store items of #291 belong to another batch.

## 2. Architect Design (Phase 1)

**Blast radius**: `soos-camera-v4l` (`daemon_config.rs`, `v4l_guard.rs`, and `monotonic_nanos` in
`mock.rs` / `v4l_impl.rs`), `soos-admin-cli` (`test_pam.rs`), `soos-daemon` (one call in
`main.rs`), `soos-pam` (`monotonic_nanos` in `ipc.rs`, found by item 4), invariants and docs.

**Item 1** — private `read_bounded_regular_file_via(path, reopen_path: impl Fn(RawFd) -> PathBuf)`
(test seam), production passing `proc_self_fd_path(fd) = /proc/self/fd/<fd>`. Steps: `O_PATH |
O_CLOEXEC` open (symbolic links followed; `ENOENT` → `NotFound`, other → `Unreadable(kind)`),
`fstat` of that handle (`!is_file` → `NotARegularFile`, size > `MAX_DAEMON_CONFIG_BYTES` →
`TooLarge`), reopen `O_RDONLY | O_NONBLOCK | O_CLOEXEC` through the seam (any failure →
`Unreadable(kind)`, so a missing `/proc` is never `NotFound`), `fstat` again (`!is_file` →
`NotARegularFile`, other `dev`/`ino` → `Unreadable(Other)`), then the unchanged bounded read.
Public API and `DaemonConfigError` unchanged.

**Item 2** — `pub enum PamTestRejection { RequestIdMismatch, StaleResponse }`
(`serde(rename_all = "snake_case")`, `as_str()`), and two new fields of `PamTestReport` after
`verdict`: `accepted: bool` (bound and fresh) and `rejected_reason: Option<PamTestRejection>` (the
mismatch is checked first, like the PAM module). Table line `Response Accepted:   yes` /
`no (<reason>)`. No field removed or renamed; `pam_result` wording unchanged.

**Item 3** — in `v4l_guard.rs`: `place_filter_on_top()` serialized by a `Mutex`, taking the current
hook once (`take_hook`) and setting once (`set_hook`) either the same hook (when it is the live
filter) or a new filter wrapping it. Identity: `LIVE_ADDRESS` (heap address of the latest filter
closure) is compared only while `LIVE_GENERATION` is non-zero; the closure owns a
`FilterLiveness(generation)` token whose `Drop` clears it, before the memory can be reused (no ABA).
`install_v4l_panic_hook_filter()` (public, explicit) always runs the placement and marks the
`Once` done; the guard's private `install_filter_once()` runs it on the first guarded call only,
so a guarded call never touches the hook afterwards. New `pub fn
v4l_panic_hook_filter_installations() -> u64` (wrap count, diagnostics and tests).
`soos-daemon` calls the explicit function right after its `tracing` hook. `soos-admin`,
`soos-enroll` and `soos-gui` install no hook (grep: no `set_hook` outside `crates/daemon`,
`crates/pam` and `v4l_guard.rs`), so they need no call.

**Item 4** — try the real 32-bit `cargo check` first; only fall back to extracting a helper if it
is impossible.

Latency budget: not touched (configuration reading in the clients, daemon start-up, diagnostics).

## 3. Plan Evaluation

Condensed in this single-agent run (no separate plan-evaluator report). Checked against the
existing contracts that must keep passing unchanged: DGP7 (`custom_flags(`, `libc::O_NONBLOCK`,
`libc::O_CLOEXEC`, `.metadata()`, `is_file()`, `.take(`; no `File::open(`, `fs::metadata(`,
`O_NOFOLLOW`, ...; the first `custom_flags(` before the first `.metadata()`), CAG3 (exactly one
`take_hook()` and one `set_hook(` in `v4l_guard.rs`, `call_once`, `thread_local!`,
`previous(info)`, `thread::panicking()`) and DGP8 (the PCX cfg selection text). The design keeps
all of them.

## 4. Tester Contract (Phase 2)

| Test | Criterion |
|---|---|
| `crates/camera-v4l/tests/daemon_config_open_path_tests.rs::test_cdf_symlink_to_a_character_device_is_refused_without_opening_it` | CDF1: a private pseudo-terminal slave watched with inotify `IN_OPEN`; a link to it (and the node itself) is `NotARegularFile` with no open event; a real open is seen (control) |
| `crates/camera-v4l/tests/daemon_config_open_path_tests.rs::test_cdf_regular_file_is_read_through_the_reopened_handle` | CDF1: a linked regular file is still opened and read |
| `daemon_config::tests::test_cdf_*` (4 unit tests) | CDF1: `/proc/self/fd` seam reads; missing `/proc` → `Unreadable(NotFound)`; other inode → `Unreadable(Other)`; non-regular reopen → `NotARegularFile` |
| `crates/admin-cli/tests/test_pam_acceptance_tests.rs::test_cdf_*` (4 tests) | CDF2: accepted / mismatch / stale / mismatch wins / stable snake_case and JSON round trip |
| `crates/camera-v4l/tests/v4l_panic_hook_reinstall_tests.rs::test_cdf_filter_is_reinstalled_on_top_of_a_later_host_hook_without_double_wrapping` | CDF3: replacement by a host hook, re-installation, idempotence (wrap count), host hook chaining to the filter |
| `config_open_diagnostics_contract::test_cdf_*` (4 invariants) | CDF1 open order, CDF3 daemon call placement, CDF4 no `timespec` `cast_unsigned`, CDF5 ADR |

**Red evidence**: the char-device test failed at the `IN_OPEN` assertion (the old reader opened the
pty slave); the admin and re-install tests failed to compile on the specified API only
(`PamTestRejection`, `accepted`, `rejected_reason`, `v4l_panic_hook_filter_installations`); the
invariants CDF1, CDF3 and CDF5 failed; CDF4 was written after the i686 check had failed to compile
(see §6) and was green once the conversions were fixed.

**Existing tests touched** (setup only, no assertion changed):
`crates/admin-cli/tests/cli_deadline_json_tests.rs::test_pam_test_report_json_escapes_special_characters`
builds a `PamTestReport` literal: the two new fields were added (`accepted: true,
rejected_reason: None`); its round-trip assertion is unchanged. `pcx_wire_routing_tests.rs` was not
modified (a temporary probe was added and reverted, see §8).

## 5. Auditor Constraints (Phase 3)

1. No `unwrap` / `expect` / `panic` in production code — met (the hook placement recovers a
   poisoned lock with `PoisonError::into_inner`; conversions use `try_from` with a 0 fallback, as
   before).
2. Fail closed: every reopen failure is `Unreadable` (defaults with a note), never `NotFound`, never
   a silent success — met, unit-tested.
3. No path-based re-check: the reopen goes through the pinned descriptor, and the reopened handle is
   compared by `dev`/`ino` — met.
4. No `unsafe` added to production code — met (`hook_address` uses `ptr::from_ref(..).addr()`); the
   new integration test uses `libc` calls with `// SAFETY:` comments (adapter crate).
5. The hook filter never drops a hook, never wraps itself, never calls `set_hook` while the thread
   panics, and keeps exactly one `take_hook` / `set_hook` — met (CAG3 invariant unchanged and green).
6. `test-pam` prints neither nonce (reasons are fixed strings) — met; existing DGP1 tests green.
7. `crates/daemon/src/main.rs`: a single added call (plus a comment) after the existing hook
   installation — met (invariant CDF3).

## 6. Implementation (Phase 4)

- `crates/camera-v4l/src/daemon_config.rs`: `O_PATH` pin, `/proc/self/fd` reopen, identity
  re-check, test seam and unit tests; module documentation updated.
- `crates/admin-cli/src/test_pam.rs`: `PamTestRejection`, `accepted`, `rejected_reason`, table line.
- `crates/camera-v4l/src/v4l_guard.rs`: re-installable filter (`place_filter_on_top`,
  `FilterLiveness`, `install_filter_once`, `v4l_panic_hook_filter_installations`).
- `crates/daemon/src/main.rs`: `soos_camera_v4l::v4l_guard::install_v4l_panic_hook_filter();` after
  the panic hook.
- **Item 4**: `rustup target add i686-unknown-linux-gnu` worked. The first `cargo check` stopped in
  the `ort-sys` build script ("no prebuilt binaries available for target i686-unknown-linux-gnu");
  `ORT_SKIP_DOWNLOAD=1` skips that lookup (nothing is linked by `cargo check`). The check then
  failed in `soos-camera-v4l`: `monotonic_nanos` in `mock.rs` and `v4l_impl.rs` computed
  `tv_sec.cast_unsigned()`, a `u32` where `time_t` is 32-bit, returned as `u64` (E0308). A
  whole-workspace check found the same pattern in `crates/pam/src/ipc.rs`. All three now use
  `u64::try_from` on both fields (same 0 fallback). With that, the workspace (all targets, all
  features) and the `SYS_setresuid32` branch type-check for i686, and the PCX test for
  `armv7-unknown-linux-gnueabihf`. No helper extraction was needed; the command is documented in
  `Docs/DEVELOPMENT_WORKFLOW.md` §4.2.
- Docs: `Docs/CAMERA_V4L_CRATE.md` (reader open sequence, hook re-installation),
  `Docs/IPC_PROTOCOL.md` (`accepted` / `rejected_reason`), `Docs/DEVELOPMENT_WORKFLOW.md` §4.2.

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) run locally (§8). The layer 2 sub-agent review is run by
the orchestrator before the push (not part of this batch run).

## 8. Verification Results

```bash
cargo fmt --all -- --check                                                # clean
cargo clippy --locked --all-targets --all-features -p soos-camera-v4l -p soos-admin-cli \
  -p soos-daemon -p soos-pam -p soos-invariants -- -D warnings            # clean
cargo test --locked --all-targets --all-features -p soos-camera-v4l -p soos-admin-cli \
  -p soos-daemon -p soos-pam -p soos-invariants                           # 1152 passed, 0 failed
ORT_SKIP_DOWNLOAD=1 cargo check --locked --workspace --all-targets --all-features \
  --target i686-unknown-linux-gnu                                         # Finished, no warning
ORT_SKIP_DOWNLOAD=1 cargo check --locked --tests --target armv7-unknown-linux-gnueabihf \
  -p soos-daemon --test pcx_wire_routing_tests                            # Finished
./scripts/candid_review.sh                                                # layer 1 passed
```

To confirm the x86 branch is really type-checked, a temporary
`#[cfg(target_arch = "x86")] const _: () = assert!(SETRESUID_SYSCALL != libc::SYS_setresuid32, "probe");`
was appended to `pcx_wire_routing_tests.rs`: the i686 check failed with "evaluation panicked:
probe" (the constant is `SYS_setresuid32` there); the probe was then reverted.

Hardware (RGB laptop camera, no frame stored): `soos-admin camera list` without
`/etc/soos/daemon.toml` reports the defaults and enumerates `/dev/video0`; with `--config` set
to a symbolic link to `/dev/video0` it prints "is not a regular file; using the soos-daemon
defaults" (the link target is refused before any open).

## 9. Known Limitations / Follow-ups

- A `v4l` stream drop panic followed by an arena drop panic still aborts the process (needs a
  fallible teardown upstream in `v4l`; ADR).
- The 32-bit check is a manual command, not a CI job; adding a CI job (with `ORT_SKIP_DOWNLOAD=1`)
  is left to the owner.
- The reader needs `/proc` to read the file at all: on a host without `/proc` the clients fall back
  to the daemon defaults with an "unreadable" note, while `soos-daemon` still reads the file.
- A host hook that chains to the filter (`take_hook` then calls it) stays in the chain after a
  re-installation, so the chain grows by one filter per such host hook; harmless (each filter only
  forwards), and no current binary does it.

## Integration note (batch merge)

The full workspace gate on the combined `fix/p3fu3-batch` branch caught an intermittent failure
of `test_cdf_rejection_reasons_are_stable_snake_case`: the `PamTestReport` latencies (`f64`) did
not always parse back from JSON to the identical value (`0.010166` vs `0.010166000000000001`),
because `serde_json` parses floats with its fast, not exactly round-tripping, algorithm by default.
The production fix enables the `float_roundtrip` feature of `serde_json` in the workspace
`Cargo.toml`, so every soos JSON output parses back to the exact value it printed. The test is
unchanged; eight consecutive runs pass. No new crate is pulled in (`Cargo.lock` is unchanged).
