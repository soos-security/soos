# Architect Spec — GitHub #333: GDM PAM file hardening, enable/status agreement and flaky tests

- **Issue**: GitHub #333 (GitHub-only follow-ups of #331 / PR #332, no backlog id). The branch
  `fix/gdm-hardening-and-flaky-tests` is **not** registered in `scripts/sync_issue.py`; the squash commit carries
  `Closes #333`.
- **Base**: `origin/main` at `3a1abd9` (PR #332 merged).
- **Sources read**: issue #333, walkthroughs 176 and 179, `AI/candid_review_report.md`,
  `AI/auditor_constraints_install_gdm_followups.md`, ADR 2026-10-05 and the 2026-10-02 amendment (PFU7) in
  `AI/DECISIONS.md`, `crates/admin-cli/src/{gdm,pam_stack,test_pam}.rs`,
  `crates/admin-cli/tests/{gdm_shared_rule_tests,gdm_shared_status_tests,gdm_tests,cli_deadline_json_tests}.rs`,
  `scripts/wait_daemon_ready.sh`, `tests/docker/systemd_unit_acceptance_test.sh` (part 2),
  `tests/invariants/src/{systemd_unit_acceptance_contract,install_gdm_followups_contract,install_presence_warmup_contract}.rs`,
  `crates/daemon/src/{main,sd_notify,logging,pipeline}.rs`, `packaging/pam/**`, `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1,
  matrix rows SUA4, PFU7, IGF14–IGF19.
- **Matrix component**: `gdm-hardening-flaky-tests`, prefix **GHF** (free: no row, test name or doc uses `GHF`).
- **Next walkthrough**: `AI/walkthroughs/180_gdm_hardening_flaky_tests.md`.

### Items requiring owner approval (summary, details in §4.6, §4.7, §4.8)

| Id | What changes in an existing test | Why it is not a weakening |
|---|---|---|
| **OA-1** | `crates/admin-cli/src/pam_stack.rs` `tests::test_igf14_delegated_auth_reports_the_stack_of_the_shared_rule`: append four auth lines to the `inner` fixture (setup only, assertion untouched) | The fixture's `[success=4]` jump lands past the end of the stack; libpam treats that as "bad jump in stack" (`PAM_PERM_DENIED`), so the fixture is itself the documented `installed: true` false positive that item 5 removes |
| **OA-2** | `tests/docker/systemd_unit_acceptance_test.sh` part 2 and the needle in `tests/invariants/src/systemd_unit_acceptance_contract.rs::test_systemd_acceptance_harness_asserts_ready_ordering_and_clean_stop` (+ matrix SUA4 wording) | The `ready_line < started_line` comparison orders two processes logging through two journald inputs, after the daemon already sent `READY=1`; no production change can make it deterministic. Replaced by a deterministic, causally exact check (READY=1 send time ≤ systemd's `ActiveEnterTimestampMonotonic`) |
| **OA-3** | `crates/admin-cli/tests/cli_deadline_json_tests.rs` helper `capture_deadline_with` (`Completion::TimeoutTolerated` only): bounded retry when the client timed out before writing its request | The deadline assertion is unchanged and must still pass on a captured request; only a run in which no request exists (correct PAM-parity behaviour, walkthrough 176 §6 residual) is repeated, at most 20 times |

If an approval is refused: OA-1 → item 5 falls back to "keep documented" (§4.5.4); OA-2 / OA-3 → the flaky test stays
flaky and the item stays open (no production change can fix it, §4.7.2 / §4.8.2).

---

## 1. Scope & Blast Radius

| Area | Files | Change |
|---|---|---|
| GDM enable (items 1, 2, 4) | `crates/admin-cli/src/gdm.rs` | Single-descriptor read of the PAM file, pre-rename re-check, backup clean-up on refusal, enable refuses a pre-anchor jump that skips the shared rule |
| PAM stack scan (items 3, 5) | `crates/admin-cli/src/pam_stack.rs` | Total read budget `MAX_PAM_STACK_READS`; structural check of a shared `[success=N]` jump target |
| Readiness helper (item 6) | `scripts/wait_daemon_ready.sh` | Timeout line prints `${q_admin}` |
| Daemon readiness log (item 7, OA-2) | `crates/daemon/src/main.rs`, `crates/daemon/src/sd_notify.rs` | `READY=1` send timestamp in the readiness line |
| systemd harness (item 7, OA-2) | `tests/docker/systemd_unit_acceptance_test.sh`, `tests/invariants/src/systemd_unit_acceptance_contract.rs` | Deterministic ordering check |
| Deadline test (item 8, OA-3) | `crates/admin-cli/tests/cli_deadline_json_tests.rs` | Bounded retry in the tolerated helper |
| Docs | see §8 | |

**Not touched** (auditor: `git diff --exit-code origin/main -- …`): `crates/pam`, `crates/protocol`, `crates/policy`,
`crates/daemon/src/{dispatcher,consensus,inference,config,pipeline}.rs`, `crates/daemon/src/presence/**`,
`crates/admin-cli/src/test_pam.rs` (production deadline code stays as is, PAM parity), `packaging/**`,
`tests/distro/**`, `scripts/install.sh`.

**Consumers of changed items**: every changed function in `gdm.rs` / `pam_stack.rs` is private or `pub(crate)`;
the only new public item is `pub const MAX_PAM_STACK_READS` (re-exported from `gdm.rs` beside
`MAX_PAM_INCLUDE_DEPTH`). `GdmStatus`, `GdmAction`, `configure_gdm*` signatures are unchanged. `sd_notify` gains one
public function used only by `main.rs`.

---

## 2. Constants & Config

| Name | Location | Value | Semantics |
|---|---|---|---|
| `MAX_PAM_STACK_READS` (new) | `crates/admin-cli/src/pam_stack.rs`, `pub const …: usize`, re-exported by `gdm.rs` | `32` | Maximum number of included stack files opened by one `delegated_auth` analysis (one `gdm enable` or one `gdm status`). Every `scan_stack` call that reaches the open consumes one unit, including a target that is then missing or refused, and a file included twice counts twice. The edited GDM file itself is not counted. Reaching the 33rd open refuses (fail closed). `0` is not a valid value (not configurable). Packaged stacks need at most 3 (Arch). |
| `MAX_PAM_INCLUDE_DEPTH` | unchanged, 4 | | Checked before the budget |
| `MAX_PAM_FILE_BYTES` | unchanged, 64 KiB | | Applies to the single-descriptor read too |
| `MAX_CLAMP_ATTEMPTS` (new, test-only, OA-3) | `crates/admin-cli/tests/cli_deadline_json_tests.rs` | `20` | Bounded retries of the tolerated clamp run |

No configuration file key, no daemon constant, no PAM constant changes.

---

## 3. Error Taxonomy (all `AdminCliError::GdmConfig(String)`, exit code unchanged)

Every refusal writes nothing, keeps `gdm.disable`, creates no backup and leaves no temporary file (except that a
backup created by the same run is removed again, §4.1). `gdm status` maps every refusal to `installed: false`,
`shared_stack: None`. None of these paths touches the PAM module or the password path.

| Id | Trigger | Exact message (`{p}` = `pam_file.display()`, `{s}` = validated stack name) |
|---|---|---|
| E1 (new) | PAM file changed between the read and the rename of `gdm enable` (identity or bytes differ, or the file vanished) | `'{p}' changed concurrently while `gdm enable` was running; nothing was written. Check the file and re-run `soos-admin gdm enable`` |
| E1-restore (unchanged text) | same, `gdm restore` | `'{p}' changed concurrently while `gdm restore` was running; nothing was written. Check the file and re-run `soos-admin gdm restore`` (byte-identical to today) |
| E1b (suffix) | E1, and the backup this run created could not be removed | E1 text + ` The backup '{b}' created by this run could not be removed: {err}` (`{b}` = backup path) |
| E2 (kept texts) | PAM file missing / symlink / non-regular / oversized / not UTF-8, now detected on the open descriptor | missing: `PAM file '{p}' does not exist`; symlink or non-regular: `Refusing to use '{p}': not a regular file (symlink or special file)`; oversized: `PAM file '{p}' exceeds 65536 bytes`; non-UTF-8: `PAM file '{p}' is not valid UTF-8; refusing to edit it`; open/inspect/read failure: `Failed to open PAM file '{p}': {e}` / `Failed to inspect PAM file '{p}': {e}` / `Failed to read PAM file '{p}': {e}` |
| E3 (new) | Read budget exhausted | `the auth stack includes more than 32 stack files (limit MAX_PAM_STACK_READS) at '{s}'; refusing to guess the stack order` — the number is formatted from the constant |
| E4 (new) | Shared primary rule reached while a pre-anchor `[...=N]` jump in the GDM file lands beyond the delegation, and no managed rule is removed | `a [...=N] jump in the PAM file lands beyond the shared auth stack '{s}', so that branch never reaches its pam_soos.so rule; nothing was written and GDM face login is not enabled automatically (see Docs/DISTRIBUTION_DEPLOYMENT.md section 2.1)` |
| E5 (new) | Shared primary rule `[success=N …]` whose jump target is not verifiable in the same file | `the shared auth stack '{s}' runs pam_soos.so with a [success={n}] jump that does not land on a rule of the same file (it runs past the end of the file or over an include, substack, @include, malformed or continued line); soos cannot tell where a face match leads, so pam_soos.so is not inserted automatically (see Docs/DISTRIBUTION_DEPLOYMENT.md section 2.1)` |

Stable substrings the tester may key on: E1 `changed concurrently while \`gdm enable\``, `re-run`; E3 `more than 32
stack files`; E4 `lands beyond the shared auth stack`, `nothing was written`; E5 `[success=` and `does not land on a
rule of the same file`.

Priority inside `plan_gdm_enable` (shared branch): existing crossing error when managed rules are removed (keeps
`test_igf18_block_removal_with_crossing_jump_is_refused`) → E4 → `Ok(None)` / `RemoveRedundant`. E3 and E5 are raised
inside `delegated_auth`, so they keep the existing priority of delegated-scan errors (after the in-file jump-crossing
check on the non-shared path, unchanged).

---

## 4. Behaviour per issue item

### 4.1 Item 1 — `gdm enable` re-checks the PAM file before the atomic rename

New private seam, mirroring `restore_gdm_pam_file_with`:

```rust
/// GitHub #333: `ensure_gdm_pam_line` with a hook run once, right before the rename of the PAM
/// file (after the temporary file is written and synced). Production passes a no-op.
fn ensure_gdm_pam_line_with(
    pam_file: &Path,
    before_rename: &mut dyn FnMut(),
) -> Result<(), AdminCliError>;

fn ensure_gdm_pam_line(pam_file: &Path) -> Result<(), AdminCliError> {
    ensure_gdm_pam_line_with(pam_file, &mut || {})
}
```

Behaviour:

1. Read the file once into a `PamFileSnapshot` through one descriptor (§4.2). The planned content is computed from
   exactly those bytes.
2. `plan == None` → return `Ok(())`: nothing is written, the hook is **not** called, no re-check (no rename happens).
3. `EnablePlan::Insert`: the backup is created **without replacement** (Revision 1, finding 3): the pristine bytes
   are written to an exclusive temporary file in the same directory (`create_new`, mode `0600`, `fchown`, final
   `chmod`, `fsync`, exactly as `write_and_rename`), its `(dev, ino)` is taken from that descriptor, then it is
   published with `fs::hard_link(tmp, backup)` (`linkat(2)`, never replaces an existing name), the temporary name is
   removed and the directory fsynced. `EEXIST` ⇒ `created_backup = None` (an existing backup, possibly created by a
   concurrent run, is kept and never touched); success ⇒ `created_backup = Some((dev, ino))`; any other error ⇒ the
   usual `Failed to write PAM file atomically '<backup>': {e}` error, temp file removed, PAM file untouched. The
   former `symlink_metadata` pre-check may stay as a fast path but is no longer relied on. Then
   `write_atomic_checked(pam_file, updated, mode, owner, &mut || { before_rename(); ensure_pam_file_unchanged(pam_file, Some(&snapshot), GdmOperation::Enable) })`.
4. `EnablePlan::RemoveRedundant`: the same `write_atomic_checked` call; never creates or touches the backup.
5. On **any** failure of step 3's PAM-file write (E1 or I/O) with `created_backup == Some((dev, ino))`: remove the
   backup only if `fs::symlink_metadata(&backup)` is a regular file with exactly that `(dev, ino)` (i.e. the inode this
   run linked; a backup replaced or re-created meanwhile is left alone), then fsync the directory; a removal failure,
   or a mismatch, appends E1b to the returned error (`{err}` = `it was replaced` for a mismatch).
   `created_backup == None` ⇒ the backup is never touched. Residual (stated, root-only): a replacement of the
   backup between that `lstat` and `unlink` is not detected.
6. `gdm.disable` is removed by `configure_gdm_with_options` only after `ensure_gdm_pam_line` returned `Ok` (unchanged
   order), so E1 keeps the flag.

`ensure_pam_file_unchanged` gains the operation:

```rust
/// Which `gdm` command performs the checked rename (message wording only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GdmOperation { Enable, Restore }

fn ensure_pam_file_unchanged(
    pam_file: &Path,
    expected: Option<&PamFileSnapshot>,
    operation: GdmOperation,
) -> Result<(), AdminCliError>;
```

Comparison (Revision 1, finding 4): `Restore` compares `dev`, `ino` and bytes exactly as today (restore semantics and
`restore_race_tests` untouched); `Enable` additionally compares `mode` (full `st_mode & 0o7777`), `uid` and `gid`, so
a concurrent `chmod`/`chown` aborts with E1 instead of being reverted to the mode read at the start. The re-read uses
the same `O_NOFOLLOW | O_NOCTTY | O_NONBLOCK | O_CLOEXEC` bounded reader; for enable `expected` is always `Some`. The
restore message stays byte-identical.

**Residual window (stated, inherent without locking, identical to `gdm restore` since #318):** a change made after
`ensure_pam_file_unchanged` returned and before `rename(2)` completes is still replaced. The check closes the
read → rename window except for those few system calls.

### 4.2 Item 2 — type check and read on the same descriptor

`ensure_gdm_pam_line_with` no longer calls `regular_file_metadata(pam_file, …)` nor `read_bounded_utf8(pam_file)`.
It opens the PAM file once with `O_RDONLY | O_NOFOLLOW | O_NOCTTY | O_NONBLOCK | O_CLOEXEC` (Revision 1, finding 9:
`O_NOCTTY` because the open now precedes the type check; the restore snapshot reader and `read_backup_bounded` take
the same flag), takes the metadata from that
descriptor (`File::metadata`, i.e. `fstat`), refuses a non-regular file and a size above `MAX_PAM_FILE_BYTES` before
reading, reads at most `MAX_PAM_FILE_BYTES + 1` bytes through the same descriptor, and derives `mode & 0o7755`, `uid`
and `gid` from that same `fstat`. `ELOOP` (symlink) → E2 symlink text. A FIFO never blocks (`O_NONBLOCK`) and is
refused as non-regular before any read.

Type changes (private):

```rust
/// The PAM file as one `gdm` command read it: identity, exact bytes and (GitHub #333) the
/// mode and owner of the same descriptor.
#[derive(Debug)]
struct PamFileSnapshot { dev: u64, ino: u64, bytes: Vec<u8>, mode: u32, uid: u32, gid: u32 }
```

`ensure_pam_file_unchanged` compares per §4.1 (a method such as `matches(&self, other, operation)` replaces the
derived `PartialEq` use): identity and bytes for restore, plus mode/uid/gid for enable. `read_pam_file_snapshot` keeps returning `Ok(None)` for
a missing file (restore needs it); enable maps `None` to the E2 "does not exist" text. Its refusal wording for restore
(`Refusing to replace '{p}': …`) is kept; enable uses `Refusing to use '{p}': …` (today's enable text) — the
developer may parameterise the verb. `regular_file_metadata` is deleted if it has no other caller (it has none today).

`get_gdm_status` is read-only and keeps following symlinks like libpam (out of scope, unchanged).

### 4.3 Item 3 — total include/read budget

```rust
/// Number of stack files `delegated_auth` may still open (GitHub #333).
struct ScanBudget { reads_left: usize }   // starts at MAX_PAM_STACK_READS
```

`delegated_auth` creates one `ScanBudget` per call and threads `&mut ScanBudget` through `scan_lines` / `scan_stack`
(signatures are private; tester calls `delegated_auth` only). In `scan_stack` the order is: depth check (unchanged) →
name validation (unchanged) → budget: `reads_left == 0` → E3, else `reads_left -= 1` (checked arithmetic, no
`saturating_sub` masking) → `read_bounded_utf8`. Effects: at most 32 bounded reads (≤ 2 MiB) per `gdm enable` or
`gdm status`, independent of width or repetition. 32 sibling includes of a neutral leaf followed by a credential are
accepted; 33 are refused with E3; `gdm status` on the 33-include stack reports `installed: false`.

### 4.4 Item 4 — `enable` and `status` agree when a pre-anchor jump skips the delegation

In `plan_gdm_enable`, shared branch (`Some(Ok(DelegatedAuth::SharedSoosRule { stack }))`):

```text
if pristine != content && jump_crosses { return Err(jump_crossing_error()) }   // unchanged
if jump_skips                           { return Err(E4(stack)) }              // new
if pristine == content                  { return Ok(None) }                    // unchanged
Ok(Some(EnablePlan::RemoveRedundant { updated: pristine }))                    // unchanged
```

`jump_skips = scan.jump_skips_anchor()` is computed from the same `AnchorScan` as `jump_crosses`, so
`shared_soos_stack` (status) and `plan_gdm_enable` (enable) use one predicate. Agreement invariant (testable; Revision 1, finding 5 adds the qualifier):
**for every input on which `gdm enable` returns `Ok`, the returned `GdmStatus` (and a later `gdm status` on the
unchanged tree) reports `installed: true`; for a GDM file that is a regular file (not a symlink or special file)
readable within `MAX_PAM_FILE_BYTES` as UTF-8, whenever `gdm status` reports `shared_stack: Some(s)`, `gdm enable`
returns `Ok` without writing.** A symlinked `gdm-password` stays a documented divergence: `status` follows it like
libpam (unchanged), `enable` refuses it (E2). A jump that lands exactly on the delegation (crosses, does not skip;
`JUMPING_GDM_PASSWORD`) keeps today's accept-without-edit (`test_igf18_jump_crossing_with_shared_rule_is_accepted_without_edit`).

### 4.5 Item 5 — target of a shared `[success=N]` rule

**Decision: implement a conservative structural check (fail closed); keep the semantic part documented.**

#### 4.5.1 libpam facts relied on

- A jump counts the following handlers of the same management group (`auth`); other types, comments and blank lines
  do not count. A `-auth` rule whose module is missing stays in the chain (silent handler) and is counted; the
  packaged Arch `system-auth` already relies on this.
- `include` / `@include` splice the target's handlers into the chain, so a jump can continue into the including file;
  `substack` handlers form one unit at a deeper level and a jump cannot leave a substack.
- A jump that runs past the end of the chain is "bad jump in stack": libpam forces `PAM_PERM_DENIED`. A face success
  through such a rule never authenticates.
- In a normal (non-cached) chain the jump itself does not set the stack's impression or status: libpam only skips N
  handlers, and the success comes from the target and the rules after it (hence `optional pam_permit.so` on Arch,
  `required pam_permit.so` on Debian). Those rules see the impression accumulated **before** the soos rule, so a
  failed `required` gate still fails the stack, whereas `success=done` returns at once with the soos success. A jump
  therefore never grants more than `success=done`: a wrong target can only turn a face success into a failure or a
  password prompt (availability), never into a bypass. (Revision 1, finding 6.)

#### 4.5.2 Rule

New `PamLine` method:

```rust
/// GitHub #333: the decimal jump `N >= 1` of the `success` action of a primary soos rule
/// (`None` for `sufficient` or `success=done`). Only meaningful when `is_primary_soos_rule()`.
pub(crate) fn primary_success_jump(&self) -> Option<usize>;
```

In `scan_lines`, when `stack.is_some()` and `line.is_primary_soos_rule()`: if `primary_success_jump()` is
`Some(n)`, the rule counts as shared only if `jump_lands_in_stack(rest, n)` holds, where `rest` are the raw lines of
the same file after the rule; otherwise return E5 (enable refuses, status `installed: false`).
`jump_lands_in_stack(rest, n)` walks `rest` in order:

- a line whose `trim_end()` ends with `\` → `false`;
- blank or `#` comment → skip; any other line `PamLine::parse` rejects (malformed) → `false`;
- a rule whose type keyword (case-insensitive, optional leading `-`) is not one of `auth`, `account`, `password`,
  `session` → `false` (malformed; libpam installs such a line as a must-fail handler that may count toward the jump;
  Revision 1, finding 7);
- an `account`, `password` or `session` rule → skip (not counted);
- an auth delegation (`include`, `substack`, `@include`, any case) → `false`;
- any other auth rule (including `-auth`) → count it; when the count reaches `n + 1` this is the target → `true`;
- end of file before that → `false`.

`scan_lines` therefore needs indexed access to its lines (e.g. `&[&str]` instead of an iterator; private change).
Rules before the shared rule and the target's own control are not otherwise inspected (unchanged §2.3 of the #331
spec: rules after the shared rule are governed by that stack).

Packaged and contract stacks keep passing: Arch `[success=4]` lands on `optional pam_permit.so`; Debian
`[success=2]` (pam-auth-update rewrite) lands on `required pam_permit.so`; `shared_auth_with("[success=1 …] pam_soos.so
timeout_ms=1500")` lands on `[default=die] pam_faillock.so authfail` (exists in file → accepted); `success=done` and
`sufficient` are unaffected.

#### 4.5.3 Residual kept documented (justification)

A jump that lands inside the file on a rule that then refuses (e.g. `pam_deny.so`, `pam_faillock.so authfail`, a
`required` credential module) still reports `installed: true`. Detecting it would require modelling the return value
of every module after the target; the error is availability-only (4.5.1, last bullet), never a bypass, and
`test_igf17_qualifying_edge_forms_count_as_shared` contractually accepts such a target. Docs keep a narrowed
false-positive sentence (§8).

#### 4.5.4 Owner approval OA-1 (required to implement 4.5.2)

`pam_stack.rs::tests::test_igf14_delegated_auth_reports_the_stack_of_the_shared_rule` builds `inner` as
`preauth` / `[success=4 default=ignore] pam_soos.so` / `[success=1 default=bad] pam_unix.so`: the jump has one rule
after it and lands past the end of the whole chain (bad jump → `PAM_PERM_DENIED`). With 4.5.2 the call returns E5 and
the test fails. Minimal setup-only change — append these four lines to the `inner` content, assertion untouched:

```text
auth optional pam_soos.so event=password-failed timeout_ms=20
auth [default=die] pam_faillock.so authfail
auth optional pam_permit.so
auth required pam_env.so
```

(`[success=4]` then skips `pam_unix`, the event rule, `authfail`, `pam_permit` and lands on `pam_env`.) If the owner
refuses, 4.5.2 is not implemented and item 5 is closed by the justification of 4.5.1/4.5.3 plus a doc sentence on
bad jumps.

### 4.6 Item 6 — quoted `--admin` path in the timeout line

`scripts/wait_daemon_ready.sh` line 228 becomes exactly:

```bash
echo "[ERROR] soos-daemon did not report healthy within ${TIMEOUT_S} s ('${q_admin} status' kept failing)." >&2
```

`q_admin` is already computed with `printf '%q'` at line 161. Every other message keeps its text (IWP1–IWP4,
IGF3–IGF5 key on `did not report healthy within 2 s`, `Last '<path> status' error:` and the `Inspect:` line; fixture
paths contain no character `%q` escapes). No raw `${ADMIN_BIN}` may remain in any `echo`/`printf` message.

### 4.7 Item 7 — `systemd unit acceptance` journal order flake

#### 4.7.1 Root cause

`crates/daemon/src/main.rs` logs `Reported readiness to systemd` **after** `sd_notify::notify_ready()` returned, i.e.
after `READY=1` is already queued on systemd's notify socket. PID 1 may process it and log
`Started soos-daemon.service` before the daemon thread formats its line. Independently, the daemon logs to stdout
(journald stdout stream) while PID 1 logs through the native journal socket; journald dispatches both inputs at the same
event priority and documents no cross-input ordering, so even a line written before `READY=1` can be stored after
`Started` when journald is busy (startup log burst on a loaded CI runner). The harness check
`listening_line < ready_line && ready_line < started_line` thus orders two processes through two inputs; only
`listening_line < ready_line` (one process, one FIFO stream) is deterministic.

#### 4.7.2 Why no production-only fix

Making `ready_line < started_line` deterministic needs both (a) the matched line written before `READY=1` and (b) the
same journald input as PID 1. (a) contradicts the past tense of the needle `Reported readiness to systemd` that the
invariant requires in `main.rs` (logging "Reported" before reporting would be false on a send failure); (b) needs a
hand-written native-journal client for selected daemon lines (or all lines, which loses unit attribution for the last
lines of an exiting process and changes every log line's transport). Rejected as disproportionate and untruthful.

#### 4.7.3 Production change (made in all cases)

- `crates/daemon/src/sd_notify.rs` (Revision 1, finding 2: built on the existing socket-parameterised seam):

  ```rust
  /// GitHub #333: [`notify_to`] that also returns the CLOCK_MONOTONIC time, in microseconds
  /// (`ns / 1000`, floored), read immediately before the datagram is sent. The stamp is
  /// `Some` only when the outcome is `Sent` and the clock read succeeded; a clock error never
  /// prevents the send. Validation, errors and `NotSupervised` are exactly those of `notify_to`
  /// (no clock read when not supervised).
  pub fn notify_to_stamped(socket: Option<&OsStr>, message: &str)
      -> io::Result<(NotifyOutcome, Option<u64>)>;

  /// `notify_to_stamped(NOTIFY_SOCKET, READY_MESSAGE)`.
  pub fn notify_ready_stamped() -> io::Result<(NotifyOutcome, Option<u64>)>;
  ```

  `notify_to` is refactored so validation, address resolution and socket set-up run first and the clock is read right
  before `send_to_addr`; `notify_to`, `notify_ready`, `notify_stopping` keep their signatures and behaviour. Clock:
  `soos_daemon::pipeline::current_monotonic_nanos()`. Unit tests use a temporary `UnixDatagram` path passed to
  `notify_to_stamped` (no environment mutation).
- `crates/daemon/src/main.rs` (Revision 1, finding 1: the value lives in the **message text**, which the ANSI-enabled
  compact formatter does not style, unlike a structured field):

  ```rust
  match sd_notify::notify_ready_stamped() {
      Ok((NotifyOutcome::Sent, Some(us))) =>
          info!("Reported readiness to systemd (ready_sent_monotonic_us={us})"),
      Ok((NotifyOutcome::Sent, None)) =>
          info!("Reported readiness to systemd (ready_sent_monotonic_us=unknown)"),
      Ok((NotifyOutcome::NotSupervised, _)) => {}
      Err(err) => warn!(error = %err, "Failed to report readiness to systemd"),
  }
  ```

  Exact shapes: `Reported readiness to systemd (ready_sent_monotonic_us=<ASCII decimal digits, no sign or separator>)`
  or `Reported readiness to systemd (ready_sent_monotonic_us=unknown)`. Both keep the prefix
  `Reported readiness to systemd` that the harness and the existing invariant key on. Logged once per start; no
  request, frame or key data.

#### 4.7.4 Test change — OA-2 (REQUIRES OWNER APPROVAL)

Harness `part2_notify_readiness`: keep the `listening_line`, `ready_line`, `started_line` extraction and the presence
check; replace the order check by:

```bash
local ready_sent_us
ready_sent_us="$(grep -m1 'Reported readiness to systemd' <<< "${log}" \
    | sed -e 's/\x1b\[[0-9;]*m//g' \
    | sed -n 's/.*(ready_sent_monotonic_us=\([0-9][0-9]*\)).*/\1/p')"
[[ -n "${ready_sent_us}" ]] \
    || fail "the readiness line carries no numeric ready_sent_monotonic_us (missing, or 'unknown': the daemon could not read CLOCK_MONOTONIC)"
(( listening_line < ready_line )) \
    || fail "daemon log order is not socket bound -> readiness reported (lines ${listening_line}, ${ready_line})"
```

and, after `active_us` is read, `(( ready_sent_us <= active_us ))` with
`fail "READY=1 sent at ${ready_sent_us} us, after systemd entered active at ${active_us} us"`. The success message
says `bind -> READY=1 (sent ${ready_sent_us} us) -> active/Started (${active_us} us)`.

Invariant needle in `test_systemd_acceptance_harness_asserts_ready_ordering_and_clean_stop`:
`"listening_line < ready_line && ready_line < started_line"` → `"listening_line < ready_line"` and
`"ready_sent_us <= active_us"`, plus (Revision 1, finding 1) the harness needles `"(ready_sent_monotonic_us=\\([0-9][0-9]*\\))"`
(the parse) and `"s/\\x1b\\[[0-9;]*m//g"` (defensive SGR strip); `main.rs` must contain
`"Reported readiness to systemd (ready_sent_monotonic_us={us})"` and
`"Reported readiness to systemd (ready_sent_monotonic_us=unknown)"` (in addition to the existing
`"Reported readiness to systemd"`). Optional (tester's choice): a daemon unit test rendering both messages through
`fmt().compact()` with ANSI on and asserting the harness `sed` extracts the digits. All of this stays inside OA-2
("invariant needle updated accordingly"); no other existing test changes. Matrix
SUA4: "the journal order is socket bound → readiness reported → `Started`" → "socket bound → readiness reported (one
stream) and `READY=1` sent no later than `ActiveEnterTimestampMonotonic` (`Started`)".

Not a weakening: `ActiveEnterTimestampMonotonic` is set by PID 1 when it processes `READY=1` and logs `Started`; the
send stamp is taken before the datagram exists, so `sent ≤ active` holds by causality and fails exactly when the
daemon would report readiness after systemd declared the unit started (the property SUA4 protects). The socket
check right after `systemctl start` (unchanged) still proves bind before `READY=1`. Minimal alternative if the owner
prefers no production change: drop `&& ready_line < started_line` only (keeping `listening_line < ready_line` and
the `Started` presence check) — weaker, because the READY=1 → Started order then rests on the socket check alone.

### 4.8 Item 8 — `test_simulate_pam_auth_zero_timeout_is_clamped_to_pam_minimum`

#### 4.8.1 Root cause

`0` is clamped to 10 ms and one cumulative deadline starts before `connect` (PAM parity, #231 / #312, owner decision
2026-10-02). Under full-suite load the client can be descheduled past 10 ms after `connect` succeeded and before its
first `write`: either `connect_before_deadline` returns `Err(Timeout)` from its post-`connect()`
`remaining_or_timeout()` check (`test_pam.rs:378`), or `write_all_before_deadline` does so before its first `write`
(`test_pam.rs:403`); in both cases the connection exists and no byte is written (correct). The mock server then reads
EOF, panics on `expect("read length")`, drops the sender, and `rx.recv_timeout` fails with "the mock server must
capture the request deadline". This is exactly walkthrough 176 §6 "Residual 2/576"; option (a) of #326 only tolerated
`Timeout`, it cannot produce a deadline that was never sent. (`connect` itself never times out here: the first
`connect()` always runs and a unix listener with a free backlog accepts it immediately, so the server's `accept`
cannot hang.)

#### 4.8.2 Why no production fix

Writing the request after the deadline (e.g. a non-blocking last-chance write) would diverge from `pam_soos.so`,
which never writes once its deadline has expired, and would make the daemon receive requests that are already
expired; starting the deadline after `connect` would break STO-NEW-4. `test_pam.rs` stays unchanged.

#### 4.8.3 Test change — OA-3 (REQUIRES OWNER APPROVAL)

In `capture_deadline_with`, `Completion::TimeoutTolerated` only (the `Required` path, the 250 ms and the oversized
tests are byte-identical in behaviour):

- the server thread sends `Option<u64>`: `Some(deadline)` after decoding a request; `None` exactly when the connection
  was accepted and then `read_exact` of the 4-byte length prefix, or of the full declared body, failed with
  `io::ErrorKind::UnexpectedEof` (client closed before a complete request; covers both timeout sites of §4.8.1). It no
  longer panics on that path only; any other server failure (accept, other read error, decode, oversize prefix) still
  panics as today;
- one attempt = fresh tempdir, socket, server, `before`, `simulate_pam_auth`, `after`, `recv_timeout(CAPTURE_WAIT)`,
  `join`;
- outcome `Ok` → the server must have sent `Some` (else panic as today); outcome `Err(Timeout)` with `Some` → use it;
  `Err(Timeout)` with `None` → next attempt; any other error → panic as today;
- at most `MAX_CLAMP_ATTEMPTS = 20` attempts; if none captured a deadline → panic
  `"no request was written in {MAX_CLAMP_ATTEMPTS} attempts; the clamp could not be observed"`;
- the returned `(before, deadline, after)` belong to the attempt that captured the deadline; the assertion in
  `test_simulate_pam_auth_zero_timeout_is_clamped_to_pam_minimum` is untouched.

Not a weakening: the asserted property (deadline == clamped 10 ms on `CLOCK_MONOTONIC`) must still hold on a real
captured request, every non-`Timeout` error still fails, and a deadline outside the window still fails on the first
captured attempt. Only runs with nothing to assert are repeated, with a bound (residual ≈ 0.35 % per run in walkthrough 176; the
20-miss figure is indicative only, since misses correlate under sustained load).

---

## 5. Invariants Touched

- **Fail closed / never `PAM_SUCCESS` from an error**: every new path refuses or reports `installed: false`; no change
  to `crates/pam`, the PAM stack files or the password path (ARCHITECTURE §2 invariants unaffected).
- **Bounded I/O**: PAM analysis bounded in depth (4), width/total (32 opens) and size (64 KiB each); daemon change is
  one clock read; harness and test retries bounded.
- **Atomic, race-checked writes of `/etc/pam.d`** (GitHub #318 extended to enable): a change (bytes, inode, mode or
  owner) made between the read and the pre-rename re-check aborts the write; a change in the remaining re-check →
  `rename(2)` window is still lost (inherent residual, as for restore); a backup is never replaced (`linkat`) and only
  the inode this run created is removed on refusal.
- **No panic paths** in `gdm.rs`, `pam_stack.rs`, `sd_notify.rs`, `main.rs` production code (checked arithmetic, no
  `unwrap`/`expect`/indexing).
- **No sensitive data in logs**: the new log field is a timestamp.
- **Test immutability**: only OA-1..OA-3, each setup/harness-level, each owner-gated.
- **PAM parity of `soos-admin test-pam`**: unchanged.
- Matrix rows: IGF14–IGF18 (must stay green), VCO4 (restore race, unchanged text), SUA4 (wording, OA-2), PFU7
  (annotation, OA-3).

## 6. Latency Budget

The PAM authentication path is not touched (admin CLI, one daemon log field before `READY=1`, tests). No budget change.

## 7. Test Hooks for the Tester

- `gdm.rs`: `ensure_gdm_pam_line_with(pam_file, &mut dyn FnMut())` (private; unit tests in a new
  `#[cfg(test)] mod enable_race_tests` next to `restore_race_tests`): edit / replace-by-same-bytes / delete in the hook
  → E1, file keeps the concurrent content, no temp file, created backup removed (Insert), existing backup byte-identical
  (RemoveRedundant), hook called exactly once on a real write and zero times when nothing is written.
- `configure_gdm(&GdmAction::Enable, …)` (public) for E2 (symlink, FIFO without blocking, oversize, non-UTF-8,
  missing), E4, E5, E3 and the agreement property; `get_gdm_status` for the `jump_skips_anchor` status path.
- `pam_stack.rs` unit tests: `delegated_auth` for E3 (32 accepted / 33 refused) and E5 (past end, over
  include/substack/`@include`, malformed or continued line in the span, `-auth` counted, non-auth lines not counted,
  Arch/Debian targets accepted); `PamLine::primary_success_jump`.
- Script: invariant test running `wait_daemon_ready.sh` with an `--admin` path containing a space and a `$`, asserting
  the timeout line shows the `%q` form; static check that no message prints a raw `${ADMIN_BIN}`.
- Daemon: unit test of the send-stamp function with a temporary `UnixDatagram` notify socket (`Sent` + `Some(us)`
  within `[before, after]` µs on `CLOCK_MONOTONIC`; `NotSupervised` → no stamp); invariant: `main.rs` logs
  `ready_sent_monotonic_us` in the `Reported readiness to systemd` event.

## 8. Documentation Drift / ADR

Docs to update:

- `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1: `gdm enable` re-checks the file before the rename (E1) and reads it through
  one `O_NOFOLLOW` descriptor; read budget 32; a pre-anchor jump past the delegation now makes `enable` refuse (E4) as
  `status` reports `installed: false`; a shared `[success=N]` rule whose jump runs past its file or over an include is
  refused (libpam "bad jump" note); the false-positive sentence narrows to an in-file target that refuses
  (`pam_deny.so`, `authfail`).
- `AI/ARCHITECTURE.md` GDM paragraph (~177–186): one sentence on the agreement and the jump-target check.
- `Docs/DAEMON.md` readiness paragraph (~232): `ready_sent_monotonic_us` field.
- `.agents/skills/dev-workflow/references/project-facts.md` §2: row `MAX_PAM_STACK_READS` = 32
  (`crates/admin-cli/src/pam_stack.rs`).
- `AI/VERIFICATION_MATRIX.md`: rows GHF1–GHF9 below; SUA4 wording (OA-2) and a PFU7 annotation (OA-3), each only if
  approved.
- `AI/walkthroughs/180_gdm_hardening_flaky_tests.md`.

Proposed matrix rows (component `gdm-hardening-flaky-tests`):

| Row | Criterion |
|---|---|
| GHF1 | `gdm enable` re-checks identity + bytes before the rename (Insert and RemoveRedundant): E1, nothing written, created backup removed, existing backup untouched, `gdm.disable` kept; restore message unchanged |
| GHF2 | Type check, size check, read, mode and owner come from one `O_NOFOLLOW` descriptor; symlink/FIFO/oversize/non-UTF-8/missing keep their texts; a FIFO never blocks |
| GHF3 | `MAX_PAM_STACK_READS` = 32 total opens per analysis (32 accepted, 33 refused with E3), applied to `enable` and `status` |
| GHF4 | Enable/status agreement: a pre-anchor jump past the delegation with a shared rule → E4 (nothing written, flag kept) and `status` `installed: false`, `shared_stack: None`; enable `Ok` ⇒ `installed: true` over every IGF/GHF fixture |
| GHF5 | Shared `[success=N]` rule counts only when its target is an auth rule of the same file reached without crossing a delegation, malformed or continued line (E5 otherwise); Arch, Debian and IGF17 forms still shared |
| GHF6 | `wait_daemon_ready.sh` timeout line uses `%q` (`q_admin`); no raw `${ADMIN_BIN}` in messages |
| GHF7 | Readiness line carries `ready_sent_monotonic_us`; harness asserts `listening_line < ready_line` and `ready_sent_us <= active_us` (OA-2) |
| GHF8 | Zero-timeout clamp test retries (≤ 20) only attempts that wrote no request; deadline assertion unchanged (OA-3) |
| GHF9 | ADR, §2.1, ARCHITECTURE, DAEMON, project-facts and walkthrough 180 updated |

ADR text to add to `AI/DECISIONS.md`:

> * **[2026-10-05] GDM PAM File Hardening, Enable/Status Agreement and Deterministic Flaky-Test Fixes (GitHub #333;
>   matrix GHF1–GHF9):** (1) `soos-admin gdm enable` reads `gdm-password` through one `O_NOFOLLOW | O_NONBLOCK`
>   descriptor (type, size, bytes, mode and owner from the same `fstat`) and, like `gdm restore` since #318, re-reads it
>   right before the rename: a different inode, bytes, mode or owner abort with "changed concurrently", nothing
>   written (a change in the last few system calls before `rename(2)` remains an inherent residual); the backup is
>   published with `linkat(2)` (never replaces one) and only the inode created by the same run is removed on refusal. (2) The delegated-stack analysis opens at most
>   `MAX_PAM_STACK_READS` = 32 stack files per run in addition to the depth bound of 4 (fail closed beyond). (3) A
>   pre-anchor `[...=N]` jump landing beyond the delegation now makes `enable` refuse, matching `status`
>   (`installed: false`): `enable` succeeds exactly when the returned status is installed. (4) A shared primary
>   `[success=N]` soos rule counts only when its target is an auth rule of the same stack file reached without crossing
>   an include, substack, `@include`, malformed or continued line; an unknown type keyword is malformed; libpam turns a jump past the
>   chain end into `PAM_PERM_DENIED`. A target inside the file that refuses (`pam_deny.so`, `pam_faillock.so authfail`)
>   stays a documented false positive: the jump itself sets no result, success comes from the target and later rules,
>   so a jump never grants more than `success=done` and the error costs availability only.
>   (5) The systemd acceptance harness no longer orders the daemon's stdout line against PID 1's `Started` line
>   (two journald inputs, logged after `READY=1`); the daemon logs the `CLOCK_MONOTONIC` time taken just before sending
>   `READY=1` in the message text (`Reported readiness to systemd (ready_sent_monotonic_us=<us>|unknown)`, unaffected by
>   ANSI field styling) and the harness requires it ≤ `ActiveEnterTimestampMonotonic` (owner approval
>   OA-2). (6) The 0 ms clamp test of `soos-admin test-pam` repeats, at most 20 times, only attempts in which the client
>   correctly timed out before writing its request; the captured-deadline assertion is unchanged and production keeps
>   PAM parity (owner approval OA-3; completes the 2026-10-02 PFU7 decision). Walkthrough 180.

Drift found: walkthrough 179 §9 calls the shared-jump case "fail-closed false positive"; with 4.5.1 it is precisely
"availability-only" (a false `installed: true` is not fail closed for reporting). The ADR wording above fixes it.

## 9. Existing Tests Possibly Affected

| Test | Expected effect |
|---|---|
| `crates/admin-cli/src/pam_stack.rs::tests::test_igf14_delegated_auth_reports_the_stack_of_the_shared_rule` | **Fails with 4.5.2** — OA-1 fixture change |
| `tests/invariants/src/systemd_unit_acceptance_contract.rs::test_systemd_acceptance_harness_asserts_ready_ordering_and_clean_stop` | **Needle change** — OA-2 |
| `tests/docker/systemd_unit_acceptance_test.sh` part 2 | **Order check change** — OA-2 |
| `crates/admin-cli/tests/cli_deadline_json_tests.rs` helper | **Retry** — OA-3 |
| `crates/admin-cli/tests/gdm_shared_rule_tests.rs` (21), `gdm_shared_status_tests.rs` (7), other `pam_stack.rs` tests | Must stay green unchanged (checked: Arch `[success=4]`, Debian `[success=2]`, `[success=1 … timeout_ms=1500]` targets exist in-file; `test_igf18_block_removal_with_crossing_jump_is_refused` keeps the crossing error by priority; `test_igf18_jump_crossing_with_shared_rule_is_accepted_without_edit` lands on, not beyond, the anchor) |
| `crates/admin-cli/src/gdm.rs::restore_race_tests` (VCO4, 5) | Green unchanged (restore message and comparison unchanged) |
| `crates/admin-cli/tests/gdm_tests.rs::test_gdm_enable_refuses_a_symlinked_pam_file`, `gdm_followup_tests.rs` (FIFO include, symlinked include target), `gdm_stack_order_tests.rs`, `gdm_commented_rule_tests.rs`, `gdm_restore_stale_backup_tests.rs` | Green unchanged (include targets still follow symlinks; only the edited file uses `O_NOFOLLOW`, as today's refusal) |
| `tests/invariants/src/install_presence_warmup_contract.rs` (IWP1–IWP4), `install_gdm_followups_contract.rs` (IGF3–IGF5) | Green unchanged (substrings kept) |
| `tests/invariants/src/lib.rs` gdm source scans (`.soos-backup`, `sync_all`, `fs::rename`, `pub const GDM_PAM_LINE`) | Green unchanged |
| `tests/invariants` matrix-citation invariants | New GHF rows must cite existing tests only |

## Revision 1 (2026-10-05) — plan evaluator findings

| Finding | Change |
|---|---|
| 1 MAJOR (ANSI breaks parse) | §4.7.3: value moved into the message text with two exact shapes (`…=<digits>)` / `…=unknown)`), two `info!` calls; §4.7.4: parse anchored on `(ready_sent_monotonic_us=<digits>)` after a defensive SGR strip, harness fails on `unknown`/missing; new invariant needles for both message shapes, the parse and the strip; optional rendering unit test. Within OA-2 |
| 2 (seam) | §4.7.3: `notify_to_stamped(Option<&OsStr>, &str)` on top of `notify_to`; `notify_ready_stamped()` delegates; tests pass a temp socket path |
| 3 (backup race) | §4.1 step 3: backup published with `linkat` (no replace, `EEXIST` ⇒ not ours); step 5: removed only if `(dev, ino)` is the one this run created; residual stated |
| 4 (overclaim, metadata) | §4.1, §5, ADR: residual re-check → `rename` window stated; enable comparison adds mode/uid/gid (restore unchanged) |
| 5 (agreement) | §4.4: invariant qualified to a regular, readable GDM file; symlink divergence documented |
| 6 (libpam wording) | §4.5.1, ADR: the jump sets no result; success comes from the target and later rules |
| 7 (unknown type) | §4.5.2: unknown type keyword ⇒ malformed ⇒ E5 |
| 8 (OA-3 precision) | §4.8.1 names both timeout sites; §4.8.3 defines `None` as accepted + `UnexpectedEof` on prefix or body; probability indicative only |
| 9 (`O_NOCTTY`) | §4.2: `O_NOCTTY` on the enable open, the snapshot re-read and the backup reader |

No change requires a test modification beyond OA-1/OA-2/OA-3. §8 doc list unchanged in scope;
`Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1 must also state the residual window (finding 4) and the regular-file
qualifier of `status`/`enable` agreement (finding 5); `Docs/DAEMON.md` documents the two exact readiness message shapes.

## Owner Approvals (2026-10-05)

The project owner approved, in the session that produced this spec:

- **OA-1**: approved. Setup-only fixture change in `test_igf14_delegated_auth_reports_the_stack_of_the_shared_rule` (four auth lines appended to `inner`, assertion unchanged); item 5 is implemented.
- **OA-2**: approved, timestamp variant. The readiness log line carries `ready_sent_monotonic_us`; the harness checks `listening_line < ready_line` and `ready_sent_us <= ActiveEnterTimestampMonotonic` instead of `ready_line < started_line`; the invariant needle and matrix row SUA4 are updated accordingly.
- **OA-3**: approved. Bounded retry (at most 20 attempts) in `capture_deadline_with`, tolerated mode only, for attempts where the client timed out without writing; the deadline assertion is unchanged and any other error still fails.
- **OA-4** (2026-10-05, after Phase 4 found that `test_ghf7_daemon_logs_the_ready_send_time_in_the_message_text` forbade the `sd_notify::notify_ready()` call that the pre-existing `systemd_readiness_tests::test_daemon_reports_ready_only_after_socket_bind` and `presence_unlock_contract::test_pau_main_spawns_presence_after_ready_and_stops_it_at_shutdown` require): approved. `sd_notify::notify_ready()` itself becomes the stamped variant (returns `(NotifyOutcome, Option<u64>)`); `notify_to_stamped` stays the test seam and `notify_ready_stamped()` stays as an alias used by `sd_notify_stamp_tests`; `main.rs` calls `sd_notify::notify_ready()` and logs the two exact message shapes. Only the new GHF7 invariant is relaxed: its clauses requiring `notify_ready_stamped()` and forbidding `sd_notify::notify_ready()` in `main.rs` are removed; every other GHF7 assertion stays. The two pre-existing tests are untouched. §4.7.3 and C13 are superseded on the function name only.
