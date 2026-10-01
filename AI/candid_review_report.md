# Candid Review Report

- **Date**: 2026-10-01
- **Target Branch**: `fix/p3fu4-batch`
- **Base (merge-base)**: `ccf37c1`
- **Reviewed-Diff-Fingerprint**: `42d7c189a379cc4b85eb85a0727612d8137c37de95857c788855d56044f51da0`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/156_mock_daemon_flake_and_store_poll_test.md`, `AI/walkthroughs/157_evidence_sweep_fd_listing.md`, `Docs/CAMERA_V4L_CRATE.md`, `Docs/EVIDENCE_STORE_CRATE.md`, `Docs/PAM_DOCKER_TEST_MATRIX.md`, `crates/evidence-store/Cargo.toml`, `crates/evidence-store/src/store.rs`, `crates/gui/src/store_tasks.rs`, `tests/docker/mock_daemon.py`, `tests/invariants/src/distro_matrix.rs`

## 1. Executive Summary

The diff merges two GitHub #293 branches. (A) The Docker mock daemon binds, configures and listens on a private staging name and then renames it to the published `--socket` path, and `EXPIRED_AGE_NS` goes from 1 s to 60 s; a static invariant pins both, and two GUI unit tests drive `StoreTaskRunner::poll` through its lost-outcome branch. (B) The evidence temp-file sweep now lists the base directory and every date partition through descriptors (`openat` relative to the locked, identity-checked base fd, `nix::dir::Dir`, `fstatat`/`unlinkat`/`fsync` on the partition fd), with a private test seam and two new unit tests.

The fingerprint was confirmed twice: `--prepare` on the clean working tree, and a manual `review_diff` of `HEAD^{tree}` against merge-base `ccf37c1` (the computation the `--rev` gate runs). Both gave `42d7c189…51da0`. No test assertion was removed or changed. The production changes keep every earlier safety rule and close the path re-resolution window. I found no CRITICAL or MAJOR defect. Three robustness/documentation suggestions are listed.

Independently re-run by the reviewer: `cargo test -p soos-evidence-store --all-features` (all suites green, including SGU5 `tests/temp_sweep_tests.rs` and the new `temp_sweep_fd_tests`); `cargo test -p soos-invariants mock_daemon` (7 passed, including the new test and `test_mock_daemon_socket_is_group_restricted`, `test_pre_mock_daemon_stamps_from_the_monotonic_clock`, `test_mock_daemon_malformed_modes_put_one_defect_on_the_wire`); `cargo test -p soos-gui test_fgp` (2 passed); `cargo clippy -D warnings` on the three touched crates; `cargo fmt --check`; `ORT_SKIP_DOWNLOAD=1 cargo check -p soos-evidence-store --all-targets --all-features --target i686-unknown-linux-gnu` (OK). I did not re-run the owner's full workspace, the Docker matrix or the 600-run stress loop, so those results are taken from the owner.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `tests/docker/mock_daemon.py` (harness, production-of-test code), `tests/invariants/src/distro_matrix.rs` (additions only: one new `#[test]`), plus inline test modules in `crates/gui/src/store_tasks.rs` (additions only, inside the existing `worker_failure_tests`) and `crates/evidence-store/src/store.rs` (new `#[cfg(test)] mod temp_sweep_fd_tests`).
- `grep '^-[^-].*(assert|#[test]|…)'`: one hit, patch line 435. It is prose in `Docs/PAM_DOCKER_TEST_MATRIX.md` (`assert_socket_modes`) that was re-wrapped and extended, not code. **No assertion removed or changed.**
- `grep '^+.*(#[ignore|cfg(any())|should_panic|tolerance|epsilon)'`: no hits.
- `grep '^[-+].*mod tests'`: no hits (the new module is `temp_sweep_fd_tests`).
- All 36 removed non-header lines were checked by hand: they are production code (`store.rs` sweep body, `sweep_candidate` signature, Cargo feature line, `mock_daemon.py` bind/chown/chmod/cleanup/constant) or docs prose. None is a test.
- Pinned constants: no harness, script or workflow pins `EXPIRED_AGE_NS` or the old `bind(sock_path)`/`chmod(sock_path)` text. Checked by grepping `tests/`, `scripts/`, `.github/` and `run_tests.sh`. The existing expiry checks (`expired.expires < before`, `issued > 0 && issued <= expires`) keep their exact assertions and still hold at 60 s.

## 3. Deep Reasoning Audit

### Logic & Architecture
- *Mock rename-publish*: `os.rename(staging, sock_path)` runs only after `bind`, `chown`, `chmod` and `listen(16)`. It is a single `rename(2)` in the same directory, so the published path either does not exist or is a socket that is already listening. Pollers (`[[ -S … ]]` loops in `tests/docker/test_suite.sh` and `tests/distro/*.sh`, `sock.exists()`/`wait_for_socket` in the invariant harnesses) can no longer see a bound socket that is not yet listening. The staging name is in the same directory as `--socket`, so the rename cannot cross devices and needs the same directory write permission that `bind` needed. → PASS.
- *Expired at 60 s*: `expires = max(now − 60 s, 2)`, `issued = max(expires − 2 s, 1)`. The PAM client rejects when `expires < now`, and that holds at once. A host with less than 60 s of CLOCK_MONOTONIC uptime gets the clamp (2, 1): still stamped, still consistent (`issued ≤ expires`), still expired. T15 keeps rejecting the expired Allow, and `issued` is never in the future, so the future-skew branch is not hit. → PASS.
- *Descriptor sweep*: after `Flock` and the dev/ino re-check, no path is resolved again. The base is listed through `openat(lock_fd, ".")`, which is a new open file description, so it has its own offset and leaves the lock fd untouched. Partitions are opened `openat(base_fd, name, O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC)`, and listing, `fstatat`, `unlinkat` and `fsync` all use that fd. `.`/`..` are skipped before `scanned` is incremented, so the bound counts the same entries as `fs::read_dir` did, and `MAX_TEMP_SWEEP_SCANNED_ENTRIES` / `MAX_TEMP_SWEEP_REMOVALS` checks keep their positions. If `d_type` is `DT_UNKNOWN`, the code falls back to `fstatat(AT_SYMLINK_NOFOLLOW)`, and an error there skips the entry, which fails safe (nothing is deleted). Symlinked partitions are refused twice: by `d_type`/lstat and by `O_NOFOLLOW`. Name grammar, regular file / single link / owner / age checks are unchanged in `sweep_candidate`. → PASS.
- *Scenario tried*: a partition renamed and replaced after the open. The new test proves the replacement's matching name is untouched and the opened directory's orphan is removed (`removed == 1`). The old implementation would have listed the replacement, failed `fstatat` with ENOENT on the old fd and removed nothing, so the test fails against the pre-fix code. → PASS.
- *GUI tests*: `poll` uses `handle.is_finished()` plus an empty `try_iter`, which leads to exactly one `pending.failed(WORKER_LOST_MESSAGE)` and `in_flight.take()`. The tests check the kind, the UID, busy until the report, free after it, and a single report. A wrong implementation (no synthesis, double report, wrong kind or UID) would fail. → PASS.

### PAM Concurrency & Deadlines
- `crates/pam` is not touched. The mock change makes the harness more deterministic and does not change any PAM deadline. → PASS (not applicable).

### Panic Safety & Fail-Closed
- `store.rs` production code adds no `unwrap`/`expect`/indexing. `date.to_str().unwrap_or_default()` cannot panic, and it only feeds the test seam (names were already filtered as UTF-8). Every nix error is mapped through `errno_to_io` into `EvidenceStoreError::Io`. The sweep only removes files, so no path can authorize anything. → PASS.

### Test Integrity & Anti-Weakening
- See §2: additions only. The new invariant checks the order bind < chmod < listen < rename and that `bind(sock_path)` is absent, and it parses `EXPIRED_AGE_NS ≥ 20 s`. It would catch a revert of either fix. The GUI tests set the private `in_flight` field from an inner test module, which is legitimate white-box access and does not mock a real-model contract. → PASS.

### Memory, Bounds & Secrets
- *Socket mode window*: the socket is bound under umask `0117` (0660, process group), then `chown` to `soos`, then `chmod` to `socket_mode` (other bits refused by argument validation). At no instant does it get an "other" bit. The intermediate states exist only on the unpublished staging name inside the 0750 `/run/soos`, and before `listen` they refuse connections. This is strictly better than the previous in-place sequence. → PASS.
- *Staging cleanup*: a stale `.mock-<pid>` from an earlier instance with the same PID is removed with `lexists` + `unlink` (`bind` would otherwise fail with EADDRINUSE). The SIGINT/SIGTERM cleanup removes both names, so a signal before the rename removes the staging socket and a signal after it removes the published path. → PASS, with the residual cases listed in §4 S1/S2.
- *fd leaks*: `base_dir` is dropped explicitly or on the early `return`. Each `partition` `Dir` is dropped at the end of its loop iteration, including on `?` and `break`. The lock is held until the function returns. The partition-name `Vec<CString>` is bounded by the scan limit. → PASS.
- *i686*: `st_mode & S_IFMT` uses `mode_t` on both sides. The reviewer's i686 `cargo check` of the crate passed. → PASS.
- `#![forbid(unsafe_code)]` stays in `soos-evidence-store`, and the nix APIs used are safe. No secrets, frames or embeddings are logged. → PASS.

### Supply Chain & Automation
- Only the `dir` feature of the already-locked `nix 0.29.0` is enabled, and only for `soos-evidence-store`. There is no new crate and no `Cargo.lock` source change, and no `.github/`, `scripts/` or hooks change. → PASS.

### English-Only Policy
- All code, comments, docs, matrix rows, the ADR and walkthroughs 156/157 are in English. The walkthrough numbers are new and unique (latest is 157). → PASS.

## 4. Detailed Findings & Action Items

No CRITICAL, MAJOR or MINOR findings.

- **[SUGGESTION]** `tests/docker/mock_daemon.py:312`: if `chown`, `chmod`, `listen` or `os.rename` raises (for example, `--socket` names an existing directory), the bound staging socket is left behind, because the `try/finally: cleanup()` only starts after the rename. Wrapping bind…rename in a `try` that unlinks `staging_path` on exception would make cleanup complete. This is test-harness only, with no security impact.
- **[SUGGESTION]** `tests/docker/mock_daemon.py` (staging name): a staging socket left by a SIGKILLed instance (the invariant harnesses use `child.kill()`) is only removed when a later instance gets the same PID. The harnesses use temporary directories and the Docker suite uses SIGTERM, so this is harmless today. A glob cleanup of `.mock-*` would remove the residual case.
- **[SUGGESTION]** `Docs/CAMERA_V4L_CRATE.md:413`: the continuation line "that finds it at least …" lost its two-space list indentation. Markdown renders it as a lazy continuation, but it is inconsistent with the surrounding item.

## 5. Final Verdict

**VERDICT: APPROVED**
