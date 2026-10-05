# Audit Constraints — GitHub #331 (install follow-ups, physical procedure, GDM double face verification)

- **Auditor**: Phase 3, 2026-10-05, branch `fix/install-gdm-followups` (base `origin/main` `b05477d`).
- **Inputs**: `AI/architect_spec_install_gdm_followups.md` (incl. Revision 1), `AI/plan_evaluator_report.md`
  (APPROVED round 2), `AI/tester_contract_install_gdm_followups.md`, uncommitted test diff and new test files.
- **Code inspected**: `crates/admin-cli/src/{gdm,pam_stack,status,main}.rs`, `crates/camera-v4l/src/v4l_impl.rs`
  (+ `supervisor_tests.rs`), `crates/daemon/src/presence/worker.rs`, `crates/daemon/tests/common/mod.rs`,
  `scripts/install.sh` (`run_release_build`, `check_build_target_dir`, artifact defaults), `scripts/wait_daemon_ready.sh`,
  `packaging/pam/arch/system-auth`.

## Test integrity check (pre-existing test files)

`git diff -- crates tests` removes exactly one line (`use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};`,
replaced by the same import plus `AtomicU64`). Pre-existing test files changed:

| File | Change | Verdict |
|---|---|---|
| `crates/daemon/tests/common/mod.rs` | `SpyCamera::stream_started_ns: AtomicU64` field, initialiser `AtomicU64::new(0)` at the single construction site, `stream_started_mono_ns` override (0 = `None`) | Accepted setup-only (spec §10, Revision 1). Default 0 keeps the trait default for every existing test. |
| `crates/admin-cli/src/pam_stack.rs` tests module | `clippy::arithmetic_side_effects` added to the existing `allow` list; new `test_igf*` functions appended | Lint attribute only; additive tests. |
| `crates/admin-cli/src/status.rs` | new `#[cfg(test)] mod systemctl_bound_tests` | Additive. |
| `crates/camera-v4l/src/v4l_impl/supervisor_tests.rs` | three `test_igf_stream_start_stamp_*` functions appended | Additive. |
| `tests/invariants/src/lib.rs` | `mod install_gdm_followups_contract;` registration | Additive. |

No assertion was modified, weakened or deleted. Red state confirmed for `gdm_shared_rule_tests`: 14 FAILED (A),
7 passed (the G guards listed in the contract), matching the contract table.

## Audit Constraints — Issue #331

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C1 | Zero `unwrap`/`expect`/`panic!`/`todo!`/`unreachable!`/unchecked indexing/unchecked arithmetic in new production code; `Instant + Duration` via `checked_add` (fallback: treat as expired), remaining time via `saturating_duration_since`/`checked_sub`; nanosecond/ms math in `presence_settle_window` and the worker deadline via `saturating_*`/`checked_*` only (`u64::MAX` inputs must not panic, IGF6 `saturates_without_panic`). | `status.rs::inspect_systemd_unit_with`, `presence/mod.rs::presence_settle_window`, `presence/worker.rs` scan step 11/12, `pam_stack.rs::is_primary_soos_rule`/`delegated_auth`, `gdm.rs` | `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`; grep `\.unwrap\(|\.expect\(|panic!` on non-test code = 0 |
| C2 | Helper reader thread created with `std::thread::Builder::new().name(..).spawn(..)` and its `Err` handled (kill + wait the child, return the unknown triple); never `std::thread::spawn` (panics on spawn failure). | `status.rs::inspect_systemd_unit_with` | grep `thread::spawn` in `crates/admin-cli/src` = 0; review |
| C3 | The `systemctl` child is **always reaped**: on timeout `kill()` then `wait()`; on reader error / oversize / non-zero exit after `try_wait` returned a status, no further wait needed; on any early return before `try_wait` succeeded, `kill()` + `wait()`. No `Child` dropped un-waited. stdin and stderr `Stdio::null()`, stdout piped; arguments exactly `show <unit> --property=ActiveState,SubState,MainPID`. | `status.rs::inspect_systemd_unit_with` | IGF11 `hanging_*`, `arguments_are_unchanged`, `missing_program_*`; review (zombie reaping is not observable by the tests) |
| C4 | Bounded read: `take(MAX_SYSTEMCTL_OUTPUT_BYTES + 1)`; more than 4096 bytes ⇒ unknown triple regardless of the exit code; the reader drops the pipe after reaching the bound (the child is never kept blocked on a full pipe until the deadline). Result handed over by `mpsc` with `recv_timeout(remaining)`; a reader still running at the deadline is detached (owns only the pipe and ≤ 4097 bytes). Total wall time ≤ `timeout` + one poll interval. `SYSTEMCTL_SHOW_TIMEOUT_MS = 1000`, `MAX_SYSTEMCTL_OUTPUT_BYTES = 4096`, poll 10 ms. | `status.rs` | IGF11 `oversized_output_is_unknown`, `descendant_holding_stdout_does_not_block`, `systemctl_show_timeout_is_one_second` |
| C5 | `is_primary_soos_rule` is conservative (only availability may be lost on a mis-classification, never a gate): module basename `pam_soos.so`, no arg starting with `event=` or `service=`, control `sufficient` (case-insensitive) or bracket whose `success` value is `done` or a decimal `N >= 1` (`usize` parse, no sign, no empty) and **every** other `key=value` is `ignore`; a token without `=`, a duplicate `success` key, a missing `success`, `success=0`, `-1`, empty ⇒ not primary. Called only on `is_auth()` rules. | `pam_stack.rs::PamLine::is_primary_soos_rule` | IGF17 unit + end-to-end tables (`test_igf17_*`) |
| C6 | Classification order in `scan_lines` exactly: continuation ⇒ refuse; non-auth ⇒ skip; delegation ⇒ recurse (`Stop`/`SharedSoos` propagate at once); credential ⇒ `Stop`; **primary soos ⇒ `SharedSoos(stack)`**; gate ⇒ collect; neutral ⇒ skip; else ⇒ the unchanged unclassified refusal. `MAX_PAM_INCLUDE_DEPTH`, `valid_name` check, `read_bounded_utf8` (O_NONBLOCK, regular file, 64 KiB, UTF-8) and every existing error text unchanged. A non-primary `pam_soos.so` before the credential module keeps the refusal (fail closed). | `pam_stack.rs::scan_lines`, `scan_stack`, `delegated_auth` | IGF14, IGF17 guard `non_primary_soos_rules_keep_the_refusal`, IGF18 `delegated_auth_keeps_refusals` / `keeps_the_depth_bound`; all `gdm_*_tests` green unchanged |
| C7 | The `stack` payload / `GdmStatus::shared_stack` is only ever a name that passed `scan_stack`'s `valid_name` check (`[A-Za-z0-9._-]`, no leading `.`): never a raw line, path or label, so the table output (`  Shared soos Rule:  <stack>`) cannot carry terminal escapes. | `pam_stack.rs`, `gdm.rs::get_gdm_status`, `main.rs` table | IGF16 `status_json_and_table_show_the_shared_stack`; review |
| C8 | `plan_gdm_enable` keeps step order of spec §2.4: continuation ⇒ `strip_managed_rules` ⇒ administrator rule in pristine ⇒ `Ok(None)` (never touched) ⇒ anchor scan (unchanged errors) ⇒ delegated scan **without `?`** ⇒ `SharedSoosRule` short-circuits (no jump check) ⇒ jump check ⇒ delegated error surfaces ⇒ insert. One shared anchor-scan helper for `plan_gdm_enable` and `shared_soos_stack` (no second copy of the pre-credential loop). | `gdm.rs::plan_gdm_enable`, `shared_soos_stack` | IGF18 (`jump_crossing_*`, `unclassified_*`, `administrator_rule_*`, `missing_include_*`); review for a single helper |
| C9 | `RemoveRedundant` writes only through `write_atomic` (exclusive temp file in the same directory, `fsync`, `rename`, directory `fsync`, temp removed on failure) with the file's own `mode & 0o7755` and owner; **never creates, rewrites, renames, chmods or deletes `<file>.soos-backup`**; `plan == None` ⇒ no write and no temp file. The `gdm.disable` flag is removed only after a successful `ensure_gdm_pam_line`. `regular_file_metadata` (symlink refusal) still runs first. | `gdm.rs::ensure_gdm_pam_line`, `EnablePlan` | IGF14 `assert_enable_writes_nothing` (inode/mtime/dir entries), IGF15 (backup bytes, mode, inode; restore; stale backup needs `--force`) |
| C10 | `get_gdm_status` fails closed: any error of the shared analysis (continuation, unclassified, no anchor, non-delegating anchor, missing/unreadable/oversized/non-UTF-8 include, depth) ⇒ `installed: false`, `shared_stack: None`; never an `Err` or panic to the caller; `enabled = installed && !disabled` unchanged (global `/etc/soos/disabled` still honoured). | `gdm.rs::get_gdm_status`, `shared_soos_stack` | IGF16 `unreadable_or_refused_stacks_fail_closed`, `stock_stack_without_soos_reports_not_installed`, `direct_rule_keeps_shared_stack_none` |
| C11 | `GDM_PAM_LINE` keeps `timeout_ms=2500`; no change to `crates/pam`, `crates/protocol`, `crates/policy`, `crates/daemon/src/{dispatcher,consensus,inference,config}.rs`, `packaging/pam/**`, `tests/docker/**`, `tests/distro/**`. | listed paths | `git diff --exit-code origin/main -- crates/pam crates/protocol crates/policy crates/daemon/src/dispatcher.rs crates/daemon/src/consensus.rs crates/daemon/src/inference.rs crates/daemon/src/config.rs packaging/pam tests/docker tests/distro` (clean at audit time); QFU5; IGF9 |
| C12 | `CameraManager::stream_started_mono_ns` is a **defaulted** method returning `None`; read only by `crates/daemon/src/presence/` (never by the dispatcher / PAM path); no `with_not_before` in the dispatcher; `DAEMON_DEFAULT_WARMUP_FRAMES` stays 0. | `camera-v4l/src/manager.rs`, daemon | IGF9 `test_igf_stream_start_settle_is_presence_only`, IWP11, IGF8 `default_*_is_none` |
| C13 | V4L stamp: stored with `Ordering::Release` right after `start_stream` returns `Ok` (before `run_capture_loop`), from the existing `monotonic_nanos()` (clock failure stores 0); cleared to 0 in `SupervisorShared::withdraw_frames` (suspend, error, shutdown, panic paths); getter returns `None` when `!is_ready()` or the value is 0 (`Acquire`). **No new `unsafe` block**; `open_and_stream` takes `&SupervisorShared` rather than an 8th parameter (no `too_many_arguments` allow). | `camera-v4l/src/v4l_impl.rs::open_and_stream`, `withdraw_frames`, `V4lCameraManager::stream_started_mono_ns` | IGF8 supervisor tests; `grep -c 'unsafe' crates/camera-v4l/src/v4l_impl.rs` unchanged vs `origin/main`; clippy |
| C14 | Mock setter `MockCameraManager::set_stream_started_mono_ns` uses an `AtomicU64` (0 = `None`), returns the value regardless of readiness, default `None` (Docker mock daemon behaviour unchanged). | `camera-v4l/src/mock.rs` | IGF8 `mock_stream_started_*` |
| C15 | Presence worker reads the stamp **after** `notify_activity()`/`wake_camera` and after `start_ns = clock_fn()`; `woke` sampled before `notify_activity()` (unchanged); deadline budget `DECISION_BUDGET_MS + RESPONSE_WRITE_MARGIN_MS + wait_ms` with saturating adds, `wait_ms <= PRESENCE_WAKE_SETTLE_MS`; the settle only removes evaluations (rate-limit attempt still recorded before the scan; k = 3 consensus and spoof veto unchanged). `PRESENCE_WAKE_SETTLE_MS` and `with_not_before(` stay literally in `worker.rs`. | `presence/worker.rs`, `presence/mod.rs` | IGF6, IGF7 (`first PAD >= 690 ms`, `< 1000 ms`, 39 attempts left), IWP8–IWP11 |
| C16 | The one new `debug!` logs only `settle_ms` (the wait) and `cause` (`"presence_wake"` / `"stream_start"` static strings); no uid-to-name, session, frame, embedding or template data; message text must pass the invariant log-keyword audit (no `frame`, `password`, … words — reword, never weaken). | `presence/worker.rs` | `soos-invariants` log audit; review |
| C17 | `install.sh`: `resolve_cargo_target_dir` defined once before `DEFAULT_ARTIFACT_DIR`; lexical join only; the later `BUILD_TARGET_DIR=` line removed; `check_build_target_dir` uses the same variable. The target path reaches the root build **only through the environment** (`SOOS_CARGO_TARGET_DIR=…`), expanded by the inner shell inside the single-quoted `-c` script — never interpolated into the `bash -lc` string (no injection through a path containing quotes, `$` or `;`). runuser positional arguments and AFC7 literals unchanged; non-root path keeps `SOOS_BINDIR="${PREFIX}/bin" "${BUILD_CMD[@]}"` as a substring. | `scripts/install.sh::resolve_cargo_target_dir`, `run_release_build` | IGF10 (four tests), AFC7, `installer_contract`; `shellcheck`-clean review |
| C18 | Build preflight check 2 must group the alternation before the actions: `find -H "${target}" \( \( -type d ! -perm -u+rx \) -o \( ! -type d ! -type l ! -perm -u+r \) \) -print -quit` — without the outer parentheses `-print -quit` binds only to the second alternative and unreadable directories are never reported. Each of the four checks runs independently with `2>/dev/null`, first hit in priority order wins, exactly one failure is printed; every untrusted string (`target`, `user`, `hit`) through `printf %q` and `preflight_fail_verbatim` (never `error()`/`echo -e`); the check stays read-only (no `chmod`/`chown` executed). The target path is absolute (C17) so `find` never parses it as an option. | `scripts/install.sh::check_build_target_dir` | IGF1, IGF2 (0000, 0300, fake root), IWP5–IWP7 |
| C19 | `wait_daemon_ready.sh`: stdout, stderr and exit status of each attempt captured separately **in memory** (no temp file, no pipe hiding the status); `$'\037'` framing parsed from the right, a report containing `\037` is treated as no report; exit 126/127 ⇒ stop after **that single attempt**, no stdout, `[ERROR] Cannot run '<admin>' (exit 126: found but not executable) …` / `(exit 127: not found)` with `<admin>` through `printf %q`, then the fix line and the sanitized tail, exit **1**; killed attempt (124/137) sets `status attempt killed after 5 s (no answer)`; tail = last `ADMIN_STDERR_TAIL_MAX=1024` characters, every control character except newline replaced by `?`, printed with `printf '%s\n'` and 8-space indent, never `echo -e`. The two existing timeout lines stay verbatim; exit codes 0/1/2, the manifest fast failure and the `timeout --kill-after` probe unchanged. | `scripts/wait_daemon_ready.sh` | IGF3–IGF5, IWP1–IWP4 |
| C20 | Documented readiness bound = `--timeout` + 7.5 s (37.5 s default) in the header, `usage()`, `README.md`, `Docs/PACKAGING_AND_PROVISIONING.md`; the degraded mode (no `timeout --kill-after`) stated as unbounded. | script header/usage, docs | IGF3 |
| C21 | No new dependency (crates or system tools); `cargo deny --locked check` green; `#![forbid(unsafe_code)]` unchanged in `admin-cli` and daemon `main.rs`. | workspace | `cargo deny --locked check`; `test_business_crates_forbid_unsafe_code` |
| C22 | Docs/ADR/walkthrough in English, needles of IGF19, QFU5, IWP13 and the screensaver keyword check kept; §6 rollback loop uses `sed -i --follow-symlinks` on `common-auth`/`system-auth`/`password-auth` only, guarded by `[ -f "$f" ]`; the 2026-10-05 ADR records the gate-scope change (gates after the shared rule governed by that stack) and the documented false negative / two fail-closed false positives of `gdm status`. | `AI/DECISIONS.md`, `AI/ARCHITECTURE.md`, `Docs/*`, `tests/physical/screensaver_test.md`, `README.md`, project-facts | IGF12, IGF13, IGF19, QFU5, IWP13; `scripts/candid_review.sh` English policy |
| C23 | Tests stay immutable: the developer must not edit any `test_igf*`, `gdm_*_tests`, `presence_*_tests`, `install_*_contract` or `SpyCamera` code. Run the timing-sensitive tests 10× once green (`presence_stream_settle_tests::test_igf_scan_*`, `systemctl_bound_tests::test_igf11_hanging_*`/`descendant_*`, IGF3/IGF5) and report flakes instead of loosening bounds. | all new tests | contract "Flakiness check" |

## Fail-open analysis (GDM shared rule)

- Writing nothing / removing the managed block can only **remove** a face success path from `gdm-password`; it never
  adds one, never alters a gate and never touches the password path. A mis-classification (C5) therefore costs at most
  face availability (the two documented `installed: true` false positives), never a lockout bypass.
- Gates before the shared rule (GDM file, intermediate stacks, shared stack up to the rule) are still classified; any
  unknown rule refuses (C6/C8). Gates after the shared rule are governed by the shared stack, identical to `sudo`,
  `login` and the lockers, and identical to the pre-#331 effective behaviour (the shared rule already retried after a
  miss at the managed block). Owner-approved and recorded in the ADR (C22). Accepted.
- `[success=done …]` inside a Fedora `substack` ends only the substack (GDM file tail still runs); through
  `include`/`@include` it ends the GDM stack exactly like the former managed block placed before the delegation.
  No new skip.

## Test strength review

- GDM: a broken implementation that treats any `pam_soos.so` as shared (IGF17 guard), deletes or recreates the backup
  (IGF15 inode/bytes/mode), leaves a temp file (dir-entry sets), skips classification before the shared rule (IGF18),
  or reports `installed` on refused stacks (IGF16) fails. Adequate.
- Presence: exact boundary values (IGF6) and a lower **and** upper bound on the first PAD call (IGF7) catch both a
  missing settle and a settle keyed on the scan start. Adequate.
- Gaps (covered by review constraints, not blocking): zombie reaping after `kill()` (C3), thread-spawn failure (C2),
  "exactly one attempt" on exit 126/127 (C19; the < 3 s bound allows several 0.5 s polls), the non-directory branch of
  the check-2 predicate (C18; the directory branch, the one the precedence bug breaks, is tested).
- Test hygiene notes (acceptable): `test_igf11_descendant_holding_stdout_does_not_block` leaves a `sleep 30` orphan for
  30 s; the IGF11 fake scripts are written once under a `OnceLock` (ETXTBSY risk only if another lib test forks while
  they are written — watch in the 10× run).

### Pre-existing violations found (not introduced by this change)

1. `gdm.rs::ensure_gdm_pam_line` (`Insert`, and `RemoveRedundant` by design) does not re-check the PAM file before the
   rename (unlike `gdm restore`, GitHub #318): a concurrent administrator edit between read and rename is lost.
   Recommended follow-up: reuse `write_atomic_checked` + `ensure_pam_file_unchanged` with an enable-specific message.
   Not required for #331 (spec keeps `Insert` semantics).
2. `ensure_gdm_pam_line` checks `symlink_metadata` then reads through `read_bounded_utf8` (follows symlinks): a
   check/use window on a root-owned `/etc/pam.d`; root-only threat, pre-existing.
3. `scan_stack` bounds depth (4) but not breadth: a stack with thousands of `include` lines of non-terminating stacks
   causes up to O(n^4) bounded reads. Root-controlled files only; now also reached by `gdm status`. Recommended
   follow-up: a total-read budget.
4. `wait_daemon_ready.sh` prints `${ADMIN_BIN}` unquoted in the existing timeout line (kept verbatim for IWP1).

### Clearance: CLEARED

The developer may start, subject to C1–C23. C18 (find operator grouping) and C17 (no path interpolation into the
`bash -lc` string) are the two easiest constraints to get wrong.
