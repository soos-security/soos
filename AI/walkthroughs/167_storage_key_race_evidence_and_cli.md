# Walkthrough 167 — Race-free Key Publication, Offloaded PasswordFailed Evidence, Storage and CLI Hardening

- **Date**: 2026-10-02
- **Issue**: GitHub #303 (STO-NEW-1, MAJOR), #310 (DMN-NEW-1, MAJOR), #312 (STO-NEW-2..10, minor),
  findings of the full project review of 2026-10-02 (base `095eae9`); no backlog issue —
  **Branch**: `fix/storage-keys-evidence-cli`
- **Matrix criteria**: SKE1–SKE13 (component `storage-key-race-evidence-and-cli`)

## 1. Context & Objectives

1. **#303**: `MasterKey::load_or_create` in `soos-biometric-store` and `soos-evidence-store` checked
   `exists()`, generated a key and published it with `rename(2)`, which replaces unconditionally.
   The daemon, `soos-enroll` and the GUI can race on first boot: the loser's key overwrote the
   winner's, leaving the templates encrypted under the first key undecryptable. A failed write left
   its `.tmp` key file behind.
2. **#310**: `ConnectionDispatcher::handle_event` sealed the `PasswordFailed` evidence snapshot
   inline on a Tokio worker (encryption, `fsync`, then `rotate_retention`, which took a blocking
   `flock` without timeout, also held by `migrate_legacy_snapshots`). A burst of events during a
   migration could pin every worker.
3. **#312**: STO-NEW-2 `enroll` without `--yes` replaced a template created meanwhile; STO-NEW-3
   `test-pam` exited 0 on a rejected `Allow`; STO-NEW-4 `test-pam` used per-syscall full timeouts
   and started its deadline after `connect`; STO-NEW-5 the daemon always created the evidence key;
   STO-NEW-6 template reads blocked on a FIFO; STO-NEW-7 `gdm restore` could restore a stale
   backup; STO-NEW-8 `logs --file` read without bound and followed FIFOs; STO-NEW-9 guided
   enrollment samples were neither zeroized nor redacted in `Debug`; STO-NEW-10 the evidence
   `flock` waits were unbounded.

## 2. Architect Design

- Both `crypto.rs`: private `publish_new_key(path, tmp_path, key)` creates the temporary file
  (`create_new`, `0600`; a creation failure returns before anything of ours exists), writes and
  `fsync`s it, publishes with `std::fs::hard_link` (`link(2)`, `EEXIST` instead of replacement),
  always unlinks the temporary name, and on `AlreadyExists` drops its own key and returns
  `read_existing_key(path)` (same validation as any existing key).
- `soos-evidence-store`: `pub const EVIDENCE_LOCK_TIMEOUT: Duration = 5 s`,
  `EvidenceStore::with_lock_timeout(Duration)`, `EvidenceStoreError::LockTimeout(String)` and a
  private `lock_base_dir()` (directory opened `O_DIRECTORY | O_NOFOLLOW`, `LockExclusiveNonblock`
  polled every 10 ms, the pattern of `BiometricStore::lock_store`), used by `rotate_retention` and
  `migrate_legacy_snapshots`. The startup sweep keeps its try-once lock.
- `soos-daemon`: the `PasswordFailed` write goes through `self.evidence_writes.spawn_blocking`
  (the set the spoof evidence uses and `main.rs` drains). `initialize_pipeline` loads or creates the
  evidence key only when `config.evidence.enabled`, otherwise an in-memory `MasterKey::generate()`
  (the behaviour of `EvidenceStore::open`; `EvidenceStore::new` is kept so the SGU7 source check
  still holds).
- `soos-enrollment-cli`: `enroll` writes with `enroll` when `--yes` or `already_enrolled`, otherwise
  with `enroll_if_absent` (`AlreadyEnrolled` mapped to `EnrollmentCliError::AlreadyEnrolled`).
  `GuidedEnrollmentSession` stores `Zeroizing<Vec<f32>>` samples, has a manual `Debug` with counts
  only, and `compute_composite_embedding` returns `Result<Zeroizing<Vec<f32>>, String>`
  (`mean_direction` accumulates in a `Zeroizing` buffer). The only consumer change is
  `crates/gui/src/app.rs`, which no longer re-wraps the composite.
- `soos-admin-cli`: `PamTestRejection::DeadlineExceeded` (`deadline_exceeded`),
  `PamTestReport::exit_code()`; `simulate_pam_auth` starts an `ExchangeDeadline` before a
  non-blocking `nix` `connect` (EAGAIN retried every 5 ms until the deadline), re-arms
  `SO_SNDTIMEO`/`SO_RCVTIMEO` with the remaining budget before every syscall (`WouldBlock`/`TimedOut`
  → `AdminCliError::Timeout`) and checks the deadline after the last byte; rejection order nonce →
  deadline → freshness (`first_rejection`, like `pam_soos.so`). `GdmArgs::force`, `GdmOptions`,
  `configure_gdm_with_options` (`configure_gdm` keeps its signature with `force = false`);
  `restore_gdm_pam_file` refuses unless `strip_managed_rules(current) == backup`. `logs --file`:
  `MAX_LOG_TAIL_LINES = 10_000`, `MAX_LOG_LINE_BYTES = 16 KiB`, `O_NONBLOCK | O_CLOEXEC`, regular
  file on the descriptor, `VecDeque` ring buffer, lossy UTF-8.

## 3. Plan Evaluation

Condensed plan (coordinator scope). Key decisions checked against the code: `link(2)` instead of
`renameat2(RENAME_NOREPLACE)` (portable, no `unsafe`/libc call in a `forbid(unsafe_code)` crate);
STO-NEW-5 implemented in code rather than by changing the docs, because `Docs/ENROLLMENT_CLI.md`
(migrate: "a missing evidence key means evidence was never enabled") is the existing contract and
no test required the daemon to create a key while disabled.

## 4. Tester Contract

| Test | Matrix | Red evidence |
|---|---|---|
| `key_publication_race_tests::test_303_concurrent_load_or_create_returns_the_published_key` (both crates) | SKE1 | `round 0: racer 0 returned a key that is not the published key` (both crates) |
| `password_failed_evidence_offload_tests::test_310_password_failed_evidence_write_never_blocks_a_worker` | SKE2–SKE3 | `the PasswordFailed handler must return while the evidence write is blocked` (`tracked_writes: 0`); against the base revision the whole runtime froze on the unbounded `flock` |
| `bounded_lock_tests::test_310_rotate_retention_times_out_on_a_held_lock`, `test_312_migrate_legacy_snapshots_times_out_on_a_held_lock` | SKE4–SKE5 | `rotate_retention must not wait indefinitely for the evidence lock` / same for migrate (API stubbed first) |
| `enroll_concurrent_template_tests::test_312_enroll_without_yes_never_replaces_a_template_created_meanwhile` | SKE6 | `expected AlreadyEnrolled, got Ok(EnrollmentOutcome { .. replaced_existing: false })` |
| `evidence_key_opt_in_tests::test_312_disabled_evidence_never_creates_the_evidence_key`, `test_312_disabled_evidence_ignores_an_invalid_evidence_key` | SKE7 | `a disabled evidence store must not create the evidence key`; `KeyError("Evidence key ... has mode 644 ...")` |
| `template_fifo_tests::test_312_template_read_does_not_block_on_a_fifo` | SKE8 | `a template read must not block on a FIFO: Timeout` |
| `test_pam_exit_deadline_tests::test_312_test_pam_exits_1_on_an_allow_bound_to_another_nonce` | SKE9 | `a rejected Allow must not exit 0` |
| `test_pam_exit_deadline_tests::test_312_test_pam_deadline_is_cumulative_across_reads`, `test_312_test_pam_silent_daemon_times_out_within_the_deadline` | SKE10 | trickled response accepted (`accepted: true`, `response_ms: 520`); `Err(SocketIo(WouldBlock))` instead of `Timeout` |
| `gdm_restore_stale_backup_tests::test_312_gdm_restore_refuses_a_stale_backup` | SKE11 | `a stale backup must not be restored` (API stubbed first) |
| `logs_bounded_file_tests::*` | SKE12 | FIFO: `logs --file must not block on a FIFO: Timeout`; ring buffer and line bound assertions failed |
| `guided_enrollment_zeroize_tests::*` | SKE13 | compile error on the specified API (`expected Zeroizing<Vec<f32>>, found Vec<f32>`); the derived `Debug` printed the samples |

### Migrated existing tests (setup only, no assertion changed)

The `PasswordFailed` write is now asynchronous, so three existing tests that inspect the evidence
store right after the connection handler returned wait for the tracked write first
(`dispatcher.evidence_writes().drain(5 s)`); their assertions are untouched:
`pipeline_integration_tests::test_12_4_password_failed_event_captures_evidence_snapshot`,
`pipeline_integration_tests::test_181_password_failed_snapshot_records_frame_metadata`,
`peer_limits_tests::test_password_failed_events_are_rate_limited_per_peer_uid` (and the negative
`test_unprivileged_peer_cannot_report_event_for_foreign_uid`, so that it cannot pass by racing the
write).

### Flakiness check

`password_failed_evidence_offload_tests` 10/10, `test_pam_exit_deadline_tests` 5/5,
`key_publication_race_tests` and `bounded_lock_tests` 5/5.

## 5. Auditor Constraints

1. No `unwrap`/`expect`/indexing in production code (clippy `-D warnings` on all targets).
2. No `unsafe` in the touched business crates; the bounded `connect` uses safe `nix` wrappers.
3. Temporary key files: `create_new` + `0600` at creation, never unlinked when this call did not
   create them, unlinked on every other path.
4. Every new wait is bounded: evidence lock (`EVIDENCE_LOCK_TIMEOUT`), `test-pam` (one cumulative
   deadline, zero budget never passed to `set_*_timeout`), `logs --file` (ring buffer, line bound,
   non-blocking open).
5. No embedding value in `Debug` (`GuidedEnrollmentSession`), composite and fusion buffers zeroized.
6. Fail closed: a contended evidence lock skips retention (logged by the caller, never fatal); a
   stale `gdm` backup is refused without changes; a FIFO template is `CorruptFile`.

## 6. Implementation

Files: `crates/biometric-store/src/{crypto,store}.rs`, `crates/evidence-store/src/{crypto,store,error,lib}.rs`,
`crates/daemon/src/{dispatcher,pipeline}.rs`, `crates/enrollment-cli/src/{service,guided_enrollment}.rs`,
`crates/admin-cli/src/{test_pam,main,gdm,args,logs}.rs`, `crates/gui/src/app.rs` (composite type
only), new tests listed in §4, three migrated test setups, `Docs/{BIOMETRIC_STORE_CRATE,EVIDENCE_STORE_CRATE,DAEMON,ENROLLMENT_CLI,IPC_PROTOCOL,DISTRIBUTION_DEPLOYMENT}.md`,
`AI/VERIFICATION_MATRIX.md`.

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) run locally; the layer-2 sub-agent review is run by the
coordinator before the push (not part of this hand-off).

## 8. Verification Results

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean.
- `cargo test --locked --all-features -p soos-biometric-store -p soos-evidence-store -p soos-enrollment-cli -p soos-admin-cli -p soos-daemon -p soos-gui -p soos-invariants`
  (with `SOOS_MODELS_DIR` pointing at the SFace deployment): 1374 passed, 0 failed, 2 ignored.
- `cargo check --all-targets --all-features --target i686-unknown-linux-gnu -p soos-biometric-store -p soos-evidence-store -p soos-admin-cli`:
  clean (`soos-daemon` / `soos-enrollment-cli` depend on `ort-sys`, which has no i686 prebuilt).

## 9. Known Limitations / Follow-ups

- `link(2)` needs a filesystem with hard links (every supported `/var/lib` filesystem has them).
- `logs --file` still scans the whole regular file (memory is bounded, time is linear in its size).
- `gdm restore` compares and then writes without a lock: a concurrent edit in that window is not
  detected (administrator tool, same as `gdm enable`).
