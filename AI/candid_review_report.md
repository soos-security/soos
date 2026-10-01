# Candid Review Report

- **Date**: 2026-10-01
- **Target Branch**: `fix/p3-followups-batch` (GitHub #287 follow-ups; merges `fix/p3fu-*` topic branches)
- **Base (merge-base)**: `36577b6` (`origin/main`)
- **Reviewed-Diff-Fingerprint**: `f963190f68342d0c675c63045134aec02d23efe566bacea17cb3679320207a8f`
- **Fingerprint provenance**: `./scripts/candid_subagent.sh --prepare` on a clean working tree, and an independent recomputation of the gate's pinned `git diff` between the merge-base and `HEAD^{tree}` (the input of the pre-push hook / CI `--rev` gate). Both yield the value above.
- **Audited Files** (97): `.agents/skills/dev-workflow/references/project-facts.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/147_daemon_followups.md`, `AI/walkthroughs/148_pam_response_expiry_and_protocol_followups.md`, `AI/walkthroughs/149_camera_vision_followups.md`, `AI/walkthroughs/150_storage_migration_and_rustup_followups.md`, `Cargo.lock`, `Docs/BIOMETRIC_STORE_CRATE.md`, `Docs/CAMERA_V4L_CRATE.md`, `Docs/CI_CD_AND_SECURITY.md`, `Docs/DAEMON.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md`, `Docs/ENROLLMENT_CLI.md`, `Docs/EVIDENCE_STORE_CRATE.md`, `Docs/INFERENCE_ORT_CRATE.md`, `Docs/IPC_PROTOCOL.md`, `Docs/MEMORY_PROTECTION_AND_SWAP.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/PAM_DOCKER_TEST_MATRIX.md`, `Docs/PAM_MODULE.md`, `README.md`, `crates/admin-cli/{Cargo.toml, src/args.rs, src/camera.rs, src/daemon_config.rs, src/lib.rs, src/main.rs, src/status.rs, src/test_pam.rs}`, `crates/admin-cli/tests/{camera_config_tests, cli_deadline_json_tests, status_memory_locked_tests, status_tests, test_pam_freshness_tests, test_pam_tests, wire_tag_tests}.rs`, `crates/biometric-store/{src/crypto.rs, src/lib.rs, src/store.rs, tests/bulk_migration_tests.rs}`, `crates/camera-v4l/{src/diagnostics.rs, src/lib.rs, src/sensor.rs, src/v4l_guard.rs, src/v4l_impl.rs, tests/supervisor_classification_tests.rs, tests/v4l_panic_guard_tests.rs}`, `crates/daemon/{src/dispatcher.rs, src/main.rs, src/shutdown.rs, tests/daemon_followups_tests.rs, tests/panic_hook_build_policy_tests.rs, tests/pcx_wire_routing_tests.rs}`, `crates/enrollment-cli/{Cargo.toml, src/args.rs, src/error.rs, src/lib.rs, src/main.rs, src/service.rs, tests/migrate_tests.rs}`, `crates/evidence-store/{src/crypto.rs, src/lib.rs, src/snapshot.rs, src/store.rs, tests/legacy_migration_tests.rs}`, `crates/inference-ort/{src/embedding.rs, tests/embedding_copy_zeroize_tests.rs}`, `crates/pam/{src/ipc.rs, tests/common/mod.rs, tests/common/stamps.rs, tests/ipc_tests.rs, tests/pam_handle_tests.rs, tests/pcx_client_tag_tolerance_tests.rs, tests/response_expiry_tests.rs, tests/strict_decode_tests.rs, tests/wire_tag_tests.rs}`, `crates/protocol/{src/lib.rs, src/types.rs, tests/response_freshness_tests.rs}`, `packaging/soos-daemon.service`, `scripts/{camera_report.sh, install.sh, install_rustup.sh}`, `tests/distro/{arch_linux_test, debian_ubuntu_test, fedora_rhel_test}.sh`, `tests/docker/{mock_daemon.py, test_suite.sh}`, `tests/invariants/src/{camera_vision_followups_contract, distro_matrix, installer_templates_contract, lib, pam_response_expiry_contract, pam_response_freshness_contract, rustup_path_contract}.rs`, `tests/physical/pam_integration_test.sh`

## 1. Executive Summary

The diff merges four GitHub #287 follow-up branches: daemon hardening (debug-only panic
message, `accept()` backoff, tracked spoof-evidence writes drained at shutdown, `memory_locked`
in `soos-admin status`, pure `stamp_response`, systemd `StartLimitIntervalSec=320`), PAM response
expiry enforcement (`Response::check_freshness` → `IpcError::StaleResponse` → `PAM_IGNORE`, the same
rule in `soos-admin test-pam`, real stamps in every mock daemon), camera / vision follow-ups
(`soos-admin camera list --config`, non-fatal installer camera report, `v4l` panic guard,
`Zeroizing` embedding copy) and storage / install (`soos-enroll migrate [--dry-run]`,
`install_rustup.sh --no-modify-path`).

I tried to break the PAM expiry rule (boundary, zero, overflow, skew, ordering against the deadline
and the nonce), the migration (crash, symlink / inode swap, key creation, failure isolation,
plaintext exposure), the accept loop and shutdown drain (busy loop, unbounded wait), the
`daemon.toml` reader, the per-thread `setresuid` test helper, the installer hook and the unit file.
No fail-open path, no new panic or thread in `crates/pam`, and no unapproved test weakening was
found. Three MINOR robustness findings and some suggestions remain; none blocks the merge.

Evidence executed by the reviewer (working tree = reviewed tree):
`cargo test --locked --all-features -p soos-protocol -p soos-pam -p soos-admin-cli -p soos-invariants -p soos-enrollment-cli -p soos-evidence-store -p soos-biometric-store`
→ every binary `ok`, zero failures (306 invariants included); `cargo test --locked --all-features -p soos-daemon --test daemon_followups_tests --test panic_hook_build_policy_tests --test pcx_wire_routing_tests` → 16 + 3 + 1 passed;
`systemd-analyze verify packaging/soos-daemon.service` → no diagnostics.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Commands run on `target/candid_diff.patch` (9427 lines):

- **Removed / changed checks** (`^-…assert|#[test]|…`): all 20 hits sit in
  `tests/invariants/src/distro_matrix.rs::test_mock_daemon_malformed_modes_put_one_defect_on_the_wire`
  (patch lines 8475–8500). **Owner-approved migration (b)**. Every former check has a stronger or
  equal replacement: the fixed `len == 41` / `prefix == 37` become `frame.len() == 4 + declared`
  plus "body consumed exactly" after decoding both varint stamps (`decode_mock_response`); version,
  echoed / non-echoed `request_id`, Allow / Deny verdict and the undecodable `malformed`
  discriminant (`> 3 && < 0x80`) are kept field for field; NEW: every non-expired mode must carry
  `issued` read from CLOCK_MONOTONIC between two out-of-process clock reads and
  `expires = issued + 2 s`, so stamps can never be a second defect; the new `expired` mode is
  checked to keep version 1, the id, an Allow and a consistent closed window. One defect per mode is
  preserved. **Justified.**
- Further removed assertion lines not matched by the regex (manual walk of every `-` line of test files):
  - `tests/invariants/src/installer_templates_contract.rs`: `StartLimitIntervalSec=60` → `=320`.
    **Owner-approved (a)**; a new test `test_daemon_unit_start_limit_interval_covers_burst_of_timed_out_starts`
    proves the interval covers `burst × (TimeoutStartSec + RestartSec)` (5 × 62 = 310 ≤ 320).
  - `tests/invariants/src/pam_response_freshness_contract.rs`: the "informational" / "not validated"
    requirement is replaced by "Enforced by the PAM client" + `IpcError::StaleResponse` + `PAM_IGNORE`
    plus two rule-table rows. **Owner-approved (c)**; the replay-protection checks of the test are kept.
- **Setup-only edits (no assertion change)**: literal stamps (`1000/2000`, `0/0`) → `fresh_stamps()` in
  `crates/pam/tests/{common/mod.rs, ipc_tests.rs, pam_handle_tests.rs, pcx_client_tag_tolerance_tests.rs, strict_decode_tests.rs, wire_tag_tests.rs}`
  and `crates/admin-cli/tests/{test_pam_tests.rs, cli_deadline_json_tests.rs, wire_tag_tests.rs}`;
  `memory_locked: Some(true)` in `crates/admin-cli/tests/status_tests.rs`; `run_as_unprivileged_peer_when_root()`
  in `crates/daemon/tests/pcx_wire_routing_tests.rs` (the `assert_ne!(getuid(), 0)` precondition is kept);
  new `mod` lines in `tests/invariants/src/lib.rs`. All within the allowed list. Fresh stamps also make
  the negative tests sharper (e.g. `test_ipc_request_id_mismatch_returns_ignore` now fails only on the nonce).
- **New escape hatches** (`#[ignore]`, `#[cfg(any())]`, `should_panic`, `tolerance`, `epsilon`): none in code
  (the only hits are ADR / matrix prose).
- **Inline test modules** (`mod tests` added / removed): none.
- **New tests**: `crates/protocol/tests/response_freshness_tests.rs` (boundary `now == expires`, saturation near
  `u64::MAX`, clock failure, value-free display), `crates/pam/tests/response_expiry_tests.rs` (expired /
  unstamped / inverted / future-dated Allow → `PAM_IGNORE`, fresh Allow → `PAM_SUCCESS`),
  `crates/admin-cli/tests/{test_pam_freshness_tests, camera_config_tests, status_memory_locked_tests}.rs`,
  `crates/daemon/tests/{daemon_followups_tests, panic_hook_build_policy_tests}.rs`,
  `crates/{biometric-store/tests/bulk_migration_tests, evidence-store/tests/legacy_migration_tests, enrollment-cli/tests/migrate_tests}.rs`,
  `crates/camera-v4l/tests/{v4l_panic_guard_tests, supervisor_classification_tests}.rs`,
  `crates/inference-ort/tests/embedding_copy_zeroize_tests.rs` and three invariant modules. Each fails
  against a plausible wrong implementation (e.g. `>` instead of `>=` at expiry, a missing freshness call
  in PAM, a hook that logs the payload in release, a migration that rewrites v2 files).

## 3. Deep Reasoning Audit

### Logic & Architecture

- *Expiry boundary*: `check_freshness` rejects `now >= expires` (`crates/protocol/src/types.rs`), so a
  response is dead at its expiry instant; pinned by `test_pre_response_is_expired_at_its_expiry_instant`. PASS.
- *Zero / overflow*: `now == 0` → `ClockUnavailable` (the PAM `monotonic_nanos` returns 0 on failure, the
  admin path passes `unwrap_or(0)`); `issued == 0 || expires == 0` → `Unstamped`; `expires < issued` →
  `Inverted`; `now.saturating_add(skew)` cannot wrap; daemon `issued.saturating_add(2 s)`. PASS.
- *Order of rules*: the error is the first violated rule; ordering only changes the reason, never the
  outcome (every variant is a rejection). PASS.
- *False rejection of a legitimate Allow*: the daemon stamps in `build_response` right before the write
  (single stamping path `stamp_response`, no other `Response` construction in `crates/daemon/src`, no
  production `with_clock_fn`), from `nix` `CLOCK_MONOTONIC`; the PAM client reads the same clock after the
  last byte. The response must arrive before the PAM deadline (≤ 5 s clamp) anyway, but the relevant gap
  is write → read (milliseconds) against a 2 s window, and `issued` can only lead `now` by read
  granularity (10 ms bound). No legitimate path is rejected; a time-namespaced PAM host is documented as
  unsupported. PASS.
- *Daemon clock downgrade*: refactored into the pure `stamp_response` with identical semantics (Allow →
  `Unavailable/InternalError` with 0/0 stamps when the clock fails or reads 0); the log line now prints the
  post-downgrade verdict, which is more accurate. PASS.
- *Mock daemon*: default `--stamps monotonic` stamps at send time (after the `timeout` delay); argparse
  last-wins makes `start_mock_daemon --mode allow --stamps zero` effective; `expired` sends a consistent
  window closed 1 s ago. PASS.
- *`camera list` precedence*: flag > `daemon.toml` > default; the clap default `prefer_ir` no longer masks
  the config (`ValueSource::CommandLine`); sentinel devices mean auto wherever they come from; the note goes
  to stderr so the JSON schema on stdout is unchanged. Keys and types match the daemon loader
  (`[pipeline] camera_device: Option<PathBuf>`, `sensor_preference: Option<String>`). PASS.
- *Supervisor hints*: `supervisor_sensor_hints` is a verbatim extraction; it can only add an IR token
  (stricter PAD policy), never remove one. PASS.
- *systemd*: `StartLimitIntervalSec`/`StartLimitBurst` stay in `[Unit]`; 5 timed-out starts span
  4 × 62 s before the 6th attempt at 310 s, inside 320 s, so the limit is reachable. PASS.
- *Migration semantics*: v2 files are never rewritten (templates additionally re-validate CBOR + UID);
  legacy evidence must carry the id of its file name; per-file errors are isolated; a missing master key,
  biometrics directory or evidence key yields a skip, never a creation. FINDING (MINOR-1) on the template
  rewrite race.

### PAM Concurrency & Deadlines

- The only production change in `crates/pam` is one pure, allocation-free `check_freshness` call after
  `matches_request` and `deadline.remaining()`, plus a `pub use` and an error variant. No thread, no Tokio,
  no new blocking call, no stdout/stderr. PASS.
- A stale response cannot extend the wait: the check runs after the socket is dropped. PASS.

### Panic Safety & Fail-Closed

- Every `ResponseFreshnessError` maps to `IpcError::StaleResponse`, which `feedback_message` and the PAM
  return path treat as `Err(_)` → `PAM_IGNORE` with the generic "unavailable" text; `PAM_SUCCESS` still
  requires `Ok((Verdict::Allow, _))`. Tried: expired Allow, unstamped Allow, inverted, future-dated, clock
  failure — all `PAM_IGNORE` (tests in `response_expiry_tests.rs`, Docker T15 unstamped + expired). PASS.
- `soos-admin test-pam` reports any stale response as `PAM_IGNORE (stale daemon response: …)` before
  looking at the verdict. PASS.
- `v4l_guard`: `query_caps`, `enum_formats`, `set_format` (the three `v4l` 0.14 conversions with
  `unwrap`/`expect`) are wrapped in `catch_unwind`; no unguarded call remains in `crates/camera-v4l/src`
  (`grep`); `enum_framesizes` has no panic site in `v4l` 0.14. The payload is dropped (device-chosen
  bytes). The release profile keeps `panic = "unwind"`. PASS.
- Daemon panic hook: `PanicMessagePolicy::for_build()` → `Withhold` whenever `debug_assertions` is off;
  `[profile.release]` does not enable debug assertions and every packaging path builds `--release`; the
  debug message is truncated to 512 chars. PASS.
- `BlockingTasks::lock` recovers from poisoning instead of panicking; a panicking evidence job is
  counted, not propagated. PASS.

### Test Integrity & Anti-Weakening

- See §2: the only changed assertions are the three owner-approved migrations (a), (b), (c); all other test
  edits are the allowed setup-only edits. No assertion removed elsewhere, no new `#[ignore]`. PASS.
- `pcx_wire_routing_tests` root-safe helper: raw `SYS_setresuid` changes only the calling thread's
  credentials (kernel credentials are per-thread; glibc's wrapper would broadcast). `#[tokio::test]` is a
  current-thread runtime on the same thread, and any blocking-pool thread is spawned from that thread
  and inherits its credentials, so `bind`, `connect` and `SO_PEERCRED` all see UID 65534. The saved UID
  stays 0, so `RootRestore::drop` can always go back to root; the guard is declared first, hence dropped
  last (after the server and the temp dir), and it also runs on unwind. A root restore failure would
  panic inside a drop during unwinding (abort) — acceptable in a test binary. With
  `--test-threads=1` the test runs on the harness thread and is restored by the same guard. PASS.
- Panic hook tests install the process-global hook inside one test function and reset it with
  `take_hook`. PASS.

### Memory, Bounds & Secrets

- `daemon.toml` reader: `take(1 MiB + 1)` then size check, UTF-8 + TOML errors collapsed into a value-free
  `Malformed`; missing / unreadable files fall back to defaults with a note (non-root safe when the file is
  0600). FINDING (MINOR-2): `metadata()` then `File::open()` without `O_NONBLOCK`.
- Evidence migration: `O_NOFOLLOW | O_NONBLOCK` open, regular-file check, bounded read
  (`read_bounded_evidence`), AEAD authentication against the path binding, temp file
  `create_new` 0600 + fsync, inode/device re-check immediately before `rename`, directory fsync, then a
  3-pass CSPRNG overwrite of the old inode through a handle opened on the verified inode. A crash leaves
  either the original file or the new one plus at most one non-`.enc` temp file (ciphertext only). PASS.
- Template migration goes through the existing `enroll` path (O_NOFOLLOW temp 0600, fsync, rename, dir
  fsync, overwrite of the previous inode). Plaintext CBOR stays in `Zeroizing`; reports carry UIDs, paths
  and error strings only (`test_smi_failure_reasons_never_contain_embedding_values`,
  `test_smi_cli_json_summary_is_valid_and_carries_no_embedding`). PASS.
- `MasterKey::load_existing` (both stores) refuses symlinks and validates via the existing
  `read_existing_key`; it never creates a key or directory. PASS.
- `BiometricEmbedding::to_vec` now returns `Zeroizing<Vec<f32>>`. PASS.
- `BlockingTasks` is bounded by the number of jobs actually running (reaped on each spawn); spoof evidence
  is at most one job per request and capped by the store. PASS.
- `accept()` backoff: 5 ms doubling to 1 s, floor 1 ms, reset on success; shutdown interrupts the sleep;
  finished handlers keep being reaped. The drain of evidence writes uses the remainder of the same
  `connection_timeout` budget (`saturating_sub`). PASS (see SUGGESTION-1 on runtime drop).
- `memory_locked` in `soos-admin status` is a boolean already present in `StatusResponse`. PASS.

### Supply Chain & Automation

- `Cargo.lock` adds only workspace `toml` to `soos-admin-cli` and `soos-evidence-store` to
  `soos-enrollment-cli` (both already in the tree); no new crate, no `deny.toml` change, no workflow change. PASS.
- `install.sh`: the camera report runs only on a live install after the install is committed, behind
  `[[ -x soos-admin ]]`, via `bash … || warn …`; `camera_report.sh` uses `set -uo pipefail` (no `-e`),
  bounds the probe with `timeout --kill-after=2` (default 15 s), validates `--timeout`, and exits 0 for
  every runtime outcome (2 only on a usage error, still caught by `|| warn`). It cannot fail an install. PASS.
- `install_rustup.sh`: `--no-modify-path`, PATH export only for its own `rustc --version` check, digests
  unchanged. PASS.

### English-Only Policy

- Code, comments, docs, ADRs, walkthroughs 147–150 and script output in the patch are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR-1]** `crates/biometric-store/src/store.rs:286-305` (`migrate_template`) — the template is read and
  decrypted, then rewritten through `enroll()`, which re-opens whatever file is at the path at that moment.
  Unlike the evidence path, there is no inode check between the read and the `rename`. If `soos-enroll delete`
  or a re-enrollment of the same UID runs at the same moment, the migration can bring back the deleted template
  (and overwrite the fresh one) or replace the new template with the old one. This needs two root operations
  racing within milliseconds and is documented as "callers must not run both at once", so it is not a
  security break. Fix: record `(dev, ino)` when reading, and in `migrate_template` refuse the rewrite
  (report it as a failure) if the path no longer holds that inode, as `migrate_snapshot_file` already does;
  or take a per-store lock that enroll/delete/migrate all share.
- **[MINOR-2]** `crates/admin-cli/src/daemon_config.rs:79-86` — `std::fs::metadata(path)` and then
  `File::open(path)` (blocking, follows symlinks). A path swapped to a FIFO between the two calls would block
  `soos-admin camera list` with no time limit (the installer run is still capped by `timeout`). The default
  path is root-owned, so the impact is low. Fix: open first with `O_NONBLOCK` (optionally `O_NOFOLLOW`), then
  check `file.metadata()?.is_file()` on the open handle.
- **[MINOR-3]** `crates/daemon/src/shutdown.rs:372-396` (`BlockingTasks::drain`) and `crates/daemon/src/main.rs:197`
  — when the budget runs out, the code detaches the jobs and calls them "abandoned". But the `#[tokio::main]`
  runtime, when it is dropped, still waits for blocking-pool jobs that are already running, so the process
  does not exit until they finish. This is not a regression (the old detached `spawn_blocking` behaved the same),
  and an evidence write is short, bounded file I/O. Fix: either word the log and docs as "not awaited by
  the drain; the process exits when the write returns", or end with `Runtime::shutdown_timeout` so the drain
  budget is really the limit.

### Suggestions (non-blocking)

- **[SUGGESTION-1]** `crates/enrollment-cli/src/service.rs:425` — `open_template_store_checked` checks that the
  directory exists and then calls `BiometricStore::new`, which creates the directory when it is missing. If the
  directory is removed in between, `migrate` would create it. Consider a store constructor that does not create
  anything, to match the "never creates a directory" ADR wording exactly.
- **[SUGGESTION-2]** `packaging/soos-daemon.service` — the invariant ignores the stop time after a start timeout
  (SIGTERM, then up to `TimeoutStopSec`). Today this only makes the start limit easier to reach, so it is
  harmless. If `TimeoutStopSec` or the stop logic grows, consider adding it to
  `test_daemon_unit_start_limit_interval_covers_burst_of_timed_out_starts` as a bound.
- **[SUGGESTION-3]** `soos-gui` still does not check response stamps (follow-up noted in the ADR). Track it as an
  issue so the three clients do not diverge.

## 5. Final Verdict

No CRITICAL or MAJOR finding. A stale, expired, unstamped, inverted or future-dated response never yields
`PAM_SUCCESS`. A legitimate daemon Allow is accepted with a wide margin. The migration never creates keys and
leaves refused files untouched. All test-assertion changes are the three owner-approved migrations.

**VERDICT: APPROVED**
