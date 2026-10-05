# Candid Review Report

- **Date**: 2026-10-05
- **Target Branch**: `fix/install-gdm-followups`
- **Base (merge-base)**: `b05477d`
- **Reviewed-Diff-Fingerprint**: `093ce4584408c25190daaca347ad5b44cebeb9b9a3ea7bcc1f61b869bc16b191`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `AI/ARCHITECTURE.md`,
  `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_install_gdm_followups.md`,
  `AI/auditor_constraints_install_gdm_followups.md`, `AI/tester_contract_install_gdm_followups.md`,
  `AI/walkthroughs/179_install_gdm_followups.md`, `Docs/CAMERA_V4L_CRATE.md`, `Docs/DAEMON.md`,
  `Docs/DISTRIBUTION_DEPLOYMENT.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `README.md`,
  `crates/admin-cli/src/gdm.rs`, `crates/admin-cli/src/main.rs`, `crates/admin-cli/src/pam_stack.rs`,
  `crates/admin-cli/src/status.rs`, `crates/admin-cli/tests/gdm_shared_rule_tests.rs`,
  `crates/admin-cli/tests/gdm_shared_status_tests.rs`, `crates/camera-v4l/src/manager.rs`,
  `crates/camera-v4l/src/mock.rs`, `crates/camera-v4l/src/v4l_impl.rs`,
  `crates/camera-v4l/src/v4l_impl/supervisor_tests.rs`, `crates/camera-v4l/tests/stream_start_stamp_tests.rs`,
  `crates/daemon/src/presence/mod.rs`, `crates/daemon/src/presence/worker.rs`,
  `crates/daemon/tests/common/mod.rs`, `crates/daemon/tests/presence_stream_settle_tests.rs`,
  `scripts/install.sh`, `scripts/wait_daemon_ready.sh`,
  `tests/invariants/src/install_gdm_followups_contract.rs`, `tests/invariants/src/lib.rs`,
  `tests/physical/screensaver_test.md`

## 1. Executive Summary

Second review round of GitHub #331. The diff covers every acceptance line of the issue:
(1) build preflight detects read-only files and unreadable subtrees with `chmod -R u+rwX` advice;
(2) the real `--timeout + 7.5 s` bound of `wait_daemon_ready.sh` is documented; (3) the last
`soos-admin status` stderr (bounded through `tail -c 4096`, then 1024 characters, control
characters neutralized) is kept in the timeout error and exit 126/127 fails fast; (4) the presence
settle is keyed on `CameraManager::stream_started_mono_ns()` as well as on the scan's own wake;
(P-1) a relative `CARGO_TARGET_DIR` is resolved once against the checkout and passed explicitly to
cargo; (P-2) `systemctl show` is bounded to 1000 ms with kill + reap and bounded output;
(§2) physical procedure rollback loop and worker discovery; (§3) `gdm enable` inserts no managed
block (and removes a redundant one, without touching the backup) when the delegated stack reaches
an active primary `pam_soos.so` rule, and `gdm status` reports it as installed with
`shared_stack`.

All three findings of the previous round are resolved: the `RemoveRedundant` path now applies the
jump-crossing check when managed rules are removed (test
`test_igf18_block_removal_with_crossing_jump_is_refused`), stderr capture is bounded before
truncation, and `gdm status` refuses a shared rule that a pre-anchor jump bypasses
(`jump_skips_anchor`). No CRITICAL or MAJOR finding remains. Targeted test suites
(`soos-admin-cli`, `soos-invariants`, `soos-camera-v4l` with `mock-camera`,
`soos-daemon --test presence_stream_settle_tests`) pass locally; both scripts pass `bash -n`; the
readiness helper was smoke-tested (stderr tail with an ESC byte rendered as `?`, exit 127 stops in
11 ms).

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `crates/admin-cli/tests/gdm_shared_rule_tests.rs` (new),
  `crates/admin-cli/tests/gdm_shared_status_tests.rs` (new),
  `crates/camera-v4l/src/v4l_impl/supervisor_tests.rs` (additions only),
  `crates/camera-v4l/tests/stream_start_stamp_tests.rs` (new),
  `crates/daemon/tests/common/mod.rs` (new `SpyCamera::stream_started_ns` field and trait method;
  default 0 = `None` = previous trait behaviour, no existing assertion touched),
  `crates/daemon/tests/presence_stream_settle_tests.rs` (new),
  `tests/invariants/src/install_gdm_followups_contract.rs` (new) and `tests/invariants/src/lib.rs`
  (module registration), plus inline test modules added in `crates/admin-cli/src/pam_stack.rs`
  (additions to the existing `mod tests`, one extra `clippy::arithmetic_side_effects` allow scoped
  to tests) and `crates/admin-cli/src/status.rs` (new `mod systemctl_bound_tests`).
- Removed/changed assertions: the only `^-` hit is line 61 of the patch, an `AI/DECISIONS.md` ADR
  paragraph that contains the word "assertion" (amended in place to point at the #331 ADR); it is
  not a test. **No test assertion was removed or modified.**
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance/epsilon): none.
- `mod tests` lines changed: none removed; additions only.

## 3. Deep Reasoning Audit

### Logic & Architecture

- *Shared-rule classification*: tried `required`/`requisite`/`optional`, `success=ok`,
  `success=0`, `success=-1`, empty value, duplicate `success`, `default=die|bad`,
  `new_authtok_reqd=done`, `event=`/`service=` arguments, `pam_soos_other.so`; all rejected by
  `is_primary_soos_rule` and covered by IGF17. `sufficient`, `done`, `N>=1`, path forms and `-auth`
  accepted. `service=` exclusion keeps `gdm.disable` effective (verified in
  `crates/pam/src/config.rs:132`, flag keyed on `PAM_SERVICE` containing `gdm`). PASS.
- *Ordering*: the primary rule is only recognized inside a delegated stack, after the credential
  check (a soos rule after `pam_unix.so` still inserts the block, IGF17) and before the gate check,
  so gates in intermediate stacks are naturally evaluated before the shared rule. A non-primary
  soos rule before the credential module stays a refusal. PASS.
- *include vs substack*: with `substack`, `done` ends only the substack and the parent continues;
  this is stricter than the managed `[success=done]` block, never fail-open. PASS.
- *Backup integrity*: `RemoveRedundant` never creates or rewrites the backup; `gdm restore` still
  returns pristine bytes and still requires `--force` for a stale backup (IGF15). PASS.
- *Jump check on removal*: `pristine != content` with a crossing jump now refuses with the
  existing error; nothing removed → `Ok(None)` without a check (no target moves). PASS.
- *Presence settle*: `presence_settle_window` is total over `u64`, saturating, clamps a future
  stream stamp to `start_ns`, `Some(0)` = `None`, `wait_ms <= settle_ms`; `woke` path keeps the
  #329 bound; a stream younger than 1000 ms started by a PAM request is now covered. V4L stamp is
  stored before the first dequeue (STREAMON) and cleared in `withdraw_frames`; the getter returns
  `None` when not ready. A stale stamp read across a suspend race only lengthens the settle
  (conservative). The settle is presence-only; the PAM path is untouched. PASS.
- *Install P-1*: relative `CARGO_TARGET_DIR` resolved lexically against `WORKSPACE_ROOT` and passed
  explicitly to both build paths, through the environment (never interpolated into `bash -lc`).
  PASS.
- *Status P-2*: hanging child killed and reaped at the deadline; a descendant holding stdout only
  detaches the reader thread (CLI process); oversized output and non-zero exit → `unknown`. PASS.
- FINDING (MINOR): enable/status disagreement when a pre-anchor jump skips the delegation, see §4.

### PAM Concurrency & Deadlines

No file of `crates/pam` changed. The 1000 ms module default now applies to GDM when the shared rule
is used (owner-accepted trade-off recorded in the issue and ADR). PASS.

### Panic Safety & Fail-Closed

No `unwrap/expect/panic!/unreachable!` in production hunks (all hits are in test modules or test
files). `plan_gdm_enable` keeps a total match on `DelegatedAuth` without a panic path. Every
analysis failure in `gdm status` reports `installed: false`. No path turns an error into face
acceptance. PASS.

### Test Integrity & Anti-Weakening

New tests would fail against plausible wrong implementations (e.g. accepting `success=ok`,
creating a backup on removal, skipping the jump check on removal, missing the stream bound, an
unbounded `systemctl`). No existing test weakened. FINDING (MINOR): no test exercises the new
`jump_skips_anchor` refusal in `gdm status`.

### Memory, Bounds & Secrets

PAM files still read with `MAX_PAM_FILE_BYTES` and `MAX_PAM_INCLUDE_DEPTH`; `systemctl` output
capped at 4097 bytes; readiness stderr capped by `tail -c 4096` then 1024 characters; the
printed stack name is a validated include name (no terminal escapes). No biometric data or
credential involved. PASS.

### Supply Chain & Automation

No `Cargo.*`, `deny.toml`, `.github/` or hook change. Scripts: no new external dependency beyond
coreutils `tail`/`find`; `printf %q` used for every user-controlled path in messages. PASS.

### English-Only Policy

All added code, comments, docs and walkthrough text are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/admin-cli/src/gdm.rs` (`plan_gdm_enable` shared branch vs
  `shared_soos_stack`) — when a pre-anchor `[...=N]` jump lands beyond the delegation and the
  delegated stack carries a shared primary rule with no managed block, `gdm enable` returns
  success without writing (and clears `gdm.disable`) while the returned status says
  `installed: false`. Fail-closed for reporting, but confusing; consider refusing with the jump
  error in that case or printing why the integration is not reported. Add a status test for
  `jump_skips_anchor`.
- **[SUGGESTION]** `scripts/install.sh` (`check_build_target_dir`) — the write check now flags any
  read-only regular file in the target directory, as the issue asks. Cargo can usually unlink a
  read-only file inside a writable directory, so this may be a false positive for some build
  scripts that copy read-only inputs; keep an eye on reports.
- **[SUGGESTION]** `crates/camera-v4l/src/v4l_impl.rs` — the stream stamp precedes the real
  `VIDIOC_STREAMON` by the buffer set-up only (microseconds); the settle may end that much early.
  Negligible; documented.

## 5. Final Verdict

**VERDICT: APPROVED**
