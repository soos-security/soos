# Walkthrough 157 — Evidence Sweep Listing Through the Partition Descriptor and the v4l Double Teardown Panic

- **Date**: 2026-10-01
- **Issue**: GitHub #293 ("Hardening (optional)" items; no backlog id) — **Branch**: `fix/p3fu4-evidence-sweep-fd`
- **Matrix criteria**: ESL1–ESL2 (component `evidence-sweep-fd-listing`)
- **ADR**: 2026-10-01 "Evidence Sweep Lists Partitions Through Their Descriptor; v4l Double
  Teardown Panic Left Upstream" (`AI/DECISIONS.md`)

---

## 1. Context & Objectives

#293 lists the residual follow-ups of the #291 batch. This batch takes its two hardening items:

| # | Item | State on `main` (`ccf37c1`) |
|---|---|---|
| 1 | Evidence orphaned temporary-file sweep (walkthrough 154) | Each date partition was listed by path (`fs::read_dir(base/<date>)`) but examined and unlinked through the partition descriptor opened `O_DIRECTORY \| O_NOFOLLOW`: two views of one directory |
| 2 | `v4l` stream drop panic followed by an arena drop panic | Aborts the process; recorded as out of scope by the ADRs of walkthroughs 152 and 155, but its exact condition and effect were not spelled out |

The test items of #293 (mock-daemon flake, `StoreTaskRunner::poll` lost-outcome test) belong to
another batch; the hardware checks are for the owner.

## 2. Architect Design (Phase 1)

**Blast radius**: `soos-evidence-store` (`store.rs`, `Cargo.toml`), docs. No public API change,
no daemon change (`pipeline::sweep_orphaned_store_temp_files` calls the same public method).

**Item 1** — after the existing base-directory `flock` and device/inode re-check, no path is
resolved again:

- private `open_dir_at_no_follow(dir_fd, name: &CStr) -> Result<nix::dir::Dir, _>`
  (`openat` with `O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC`; `Dir::openat` closes the
  descriptor itself on failure);
- the base directory is listed through `open_dir_at_no_follow(<locked fd>, ".")` (own offset,
  the lock descriptor is untouched), each partition opened with `open_dir_at_no_follow(<locked
  fd>, <date>)`, then listed, examined (`fstatat`), unlinked from (`unlinkat`) and synced
  (`fsync`) through that one `Dir` descriptor;
- `is_dot_entry` skips `.` / `..` before the scan counter (the `fs::read_dir` listing never
  returned them, so the 65 536 bound counts the same entries); `is_directory_entry` uses the
  stream's `d_type` and falls back to `fstatat(AT_SYMLINK_NOFOLLOW)` on `DT_UNKNOWN`;
- `sweep_candidate` takes the name as `&CStr` (private);
- `nix` feature `dir` enabled in `crates/evidence-store/Cargo.toml` only (no new crate, lock
  file unchanged);
- test seam: the public `sweep_orphaned_temp_files()` delegates to the private
  `sweep_orphaned_temp_files_with(&mut dyn FnMut(&str))`, whose hook runs right after each
  partition open (no-op in production).

Rejected: a dev/ino re-check of the partition path before and after listing (still two views,
with a window between the checks), and listing through `/proc/self/fd/<n>` (depends on
`/proc`). Every existing rule is kept: exact name grammar, regular single-link file owned by root
or the effective UID, `TEMP_SWEEP_MIN_AGE` (60 s), `MAX_TEMP_SWEEP_REMOVALS` (256), lock tried
once, startup never blocked.

**Item 2** — docs only: no fork or patch of `v4l`. Verified against `v4l` 0.14.0
(`src/io/mmap/stream.rs:92`, `src/io/mmap/arena.rs:121`): both `Drop` impls ignore `ENODEV` and
`panic!` on any other error.

Latency budget: not touched (startup housekeeping only).

## 3. Plan Evaluation

Condensed in this single-agent run (no separate plan-evaluator report). Checked that no
invariant pins the old `fs::read_dir(&path)` text (`grep` of `tests/invariants/src`: only
`storage_aad_contract.rs` reads `store.rs`, for AAD bindings) and that the SGU5 contract tests
observe only outcomes, not the listing mechanism.

## 4. Tester Contract (Phase 2)

| Test | Criterion |
|---|---|
| `store::temp_sweep_fd_tests::test_esl_swapped_partition_is_listed_through_the_opened_descriptor` | ESL1: in the hook the partition is renamed away and replaced by a directory holding another old, matching temporary name; the replacement's file survives, the opened partition's orphan is removed, `removed == 1` |
| `store::temp_sweep_fd_tests::test_esl_unswapped_partition_is_still_swept` | ESL1: the public sweep (no-op seam) still removes an orphan |

**Red evidence**: with the seam added and the old by-path listing kept, the swap test failed at
"the orphan of the opened partition must be removed through its descriptor": the names came from
the replacement directory, their `unlinkat` in the opened one found nothing (`ENOENT`, kept), and
the real orphan was never examined. The control test passed.

**Existing tests touched**: none. `crates/evidence-store/tests/temp_sweep_tests.rs` (8 SGU5
tests) and `crates/daemon/tests/store_temp_sweep_tests.rs` (SGU7) are unchanged and green.

## 5. Auditor Constraints (Phase 3)

1. No `unwrap` / `expect` / `panic` in production code — met (`CString::to_str` failure maps to an
   empty hook argument with `unwrap_or_default`; partition names are already validated dates).
2. No `unsafe` (the crate keeps `#![forbid(unsafe_code)]`) — met: `nix::dir::Dir::openat` and
   `Dir::iter` are safe APIs; no raw descriptor ownership is transferred by hand.
3. No descriptor leak — met: `Dir` owns its descriptor (`closedir` on drop, closed by `nix` on
   an `fdopendir` failure); the base stream is dropped before the partitions are processed;
   `O_CLOEXEC` on every open.
4. Symlink-safe, fail closed — met: every open is `O_NOFOLLOW | O_DIRECTORY`, every `fstatat`
   is `AT_SYMLINK_NOFOLLOW`; an unopenable partition is skipped (as before); a listing error is
   an `Io` error (as before), which the daemon logs without failing the startup.
5. Bounded — met: the same removal and scan bounds; `.` / `..` excluded from the count.
6. No secret in logs or errors — met (counts and errno only; no logging added).

## 6. Implementation (Phase 4)

- `crates/evidence-store/src/store.rs`: the descriptor-only listing, the four private helpers
  (`open_dir_at_no_follow`, `errno_to_io`, `is_dot_entry`, `is_directory_entry`), the seam, the
  updated method documentation and the `temp_sweep_fd_tests` unit module.
- `crates/evidence-store/Cargo.toml`: `nix = { workspace = true, features = ["dir"] }`.
- `Docs/EVIDENCE_STORE_CRATE.md` §4.6 ("One view per directory") and §5 (ESL1).
- `Docs/CAMERA_V4L_CRATE.md`: the former one-sentence "Not guardable" remark becomes the "Known
  upstream limitation: double teardown panic aborts" paragraph:
  - *condition*: `Drop for Stream` panics on a `VIDIOC_STREAMOFF` error other than `ENODEV`, and
    during that unwind `Drop for Arena` panics on a `munmap` or `VIDIOC_REQBUFS(0)` error other
    than `ENODEV` (both ioctls failing on a device that still answers; an unplugged device
    returns `ENODEV`, ignored by both);
  - *effect*: process abort (`SIGABRT`, uncatchable). For `soos-daemon` it is fail-closed: PAM
    clients lose the socket and return `PAM_IGNORE` (password fallback, never `PAM_SUCCESS`),
    systemd restarts the daemon (`Restart=on-failure`, `RestartSec=2`, `StartLimitBurst=5` in
    320 s); a temporary file left by a write in progress is removed by a later startup sweep.
    `soos-enroll` / `soos-gui` / `soos-admin` abort the command with nothing partial stored;
  - *why it stays*: needs a fallible teardown upstream in `v4l`; `ManuallyDrop` leaks and a fork
    remain rejected.

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) run locally (§8). The layer 2 sub-agent review is run by
the orchestrator before the push (not part of this batch run).

## 8. Verification Results

```bash
export CARGO_BUILD_JOBS=6
cargo fmt --all -- --check                                                # clean
cargo clippy --locked --all-targets --all-features -p soos-evidence-store \
  -p soos-invariants -- -D warnings                                       # clean
cargo test --locked --all-features -p soos-evidence-store -p soos-daemon \
  -p soos-invariants                                                      # all passed
ORT_SKIP_DOWNLOAD=1 cargo check --locked -p soos-evidence-store --all-targets --all-features \
  --target i686-unknown-linux-gnu                                         # Finished, no warning
./scripts/candid_review.sh                                                # layer 1 passed
```

## 9. Open Points

- The `v4l` double teardown panic stays until `v4l` offers a fallible teardown; revisit on a
  `v4l` upgrade.
- The base directory itself is still validated by path once (`symlink_metadata`, then the
  device/inode re-check after the lock), unchanged from walkthrough 154.
