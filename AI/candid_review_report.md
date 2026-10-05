# Candid Review Report

- **Date**: 2026-10-05
- **Target Branch**: `fix/gdm-hardening-and-flaky-tests`
- **Base (merge-base)**: `3a1abd9`
- **Reviewed-Diff-Fingerprint**: `737d0b65892830f6ae974c92f846207fb7f69a7aa2c97e2244e6bed76e5a4d42`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `AI/ARCHITECTURE.md`,
  `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_gdm_hardening_flaky_tests.md`,
  `AI/auditor_constraints_gdm_hardening_flaky_tests.md`, `AI/tester_contract_gdm_hardening_flaky_tests.md`,
  `AI/walkthroughs/180_gdm_hardening_flaky_tests.md`, `Docs/DAEMON.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md`,
  `crates/admin-cli/src/gdm.rs`, `crates/admin-cli/src/pam_stack.rs`,
  `crates/admin-cli/tests/cli_deadline_json_tests.rs`, `crates/admin-cli/tests/gdm_hardening_tests.rs`,
  `crates/daemon/src/main.rs`, `crates/daemon/src/sd_notify.rs`, `crates/daemon/tests/sd_notify_stamp_tests.rs`,
  `scripts/wait_daemon_ready.sh`, `tests/docker/systemd_unit_acceptance_test.sh`,
  `tests/invariants/src/gdm_hardening_flaky_tests_contract.rs`, `tests/invariants/src/lib.rs`,
  `tests/invariants/src/systemd_unit_acceptance_contract.rs`

## 1. Executive Summary

The diff implements the eight items of GitHub #333. In `gdm enable` it adds a single-descriptor read, a
pre-rename re-check, a `linkat` backup and clean-up of the backup on refusal. In the stack scan it adds the
read budget `MAX_PAM_STACK_READS` = 32, the enable/status agreement on a pre-anchor jump, and a structural
check of where a shared `[success=N]` jump lands. It also quotes the `--admin` path with `%q` in the
readiness timeout line, adds the `READY=1` send stamp with a deterministic harness ordering (OA-2), and a
bounded clamp-test retry (OA-3). It does not touch `crates/pam`, `crates/protocol`, `crates/policy` or the
password path. Every new failure path refuses (enable error) or reports `installed: false` (status). None
of them can lead to `PAM_SUCCESS`.

`cargo test -p soos-admin-cli -p soos-daemon -p soos-invariants` gave 0 failures.
`cargo clippy --all-targets -D warnings` on the same crates is clean.

Changes to pre-existing tests match the owner approvals OA-1, OA-2 and OA-3 exactly. OA-4 relaxes only
the new GHF7 invariant. I found no CRITICAL or MAJOR defect. Three MINOR or SUGGESTION items are listed
below.

## 2. Test Changes (mechanical listing from step 3)

Test files touched: `crates/admin-cli/src/pam_stack.rs` (inline `mod tests` plus a new `mod ghf_tests`),
`crates/admin-cli/src/gdm.rs` (a new `mod enable_race_tests`), `cli_deadline_json_tests.rs`, the new
`gdm_hardening_tests.rs`, the new `sd_notify_stamp_tests.rs`, `systemd_unit_acceptance_test.sh`,
`systemd_unit_acceptance_contract.rs`, the new `gdm_hardening_flaky_tests_contract.rs`, and
`tests/invariants/src/lib.rs` (module registration only).

The grep for removed assertions, tests or proptests matched only two matrix rows (SUA4 and PFU7, which are
rewritten, not removed). It matched no Rust assertion. The grep for new escape hatches (`#[ignore]`,
`should_panic`, tolerance, epsilon) found none in code.

| Change | Approval | Verification |
|---|---|---|
| `pam_stack.rs::tests::test_igf14_delegated_auth_reports_the_stack_of_the_shared_rule`: four lines appended to the `inner` fixture | OA-1 | The lines are byte-for-byte the four lines in spec §4.5.4. The assertion is untouched. The original fixture is now a negative test (`test_ghf5_jump_past_the_end_of_the_file_is_refused`), so coverage grew. |
| `systemd_unit_acceptance_test.sh` part 2: `ready_line < started_line` replaced by `listening_line < ready_line` plus `ready_sent_us <= active_us`, with an SGR strip and a numeric parse | OA-2 (timestamp variant) | Matches spec §4.7.4. The `started_line` presence check is kept. The `unknown` shape and a missing stamp both fail. The comparison runs after `active_us` is read. |
| `systemd_unit_acceptance_contract.rs`: one needle replaced by four, two `main.rs` message needles added | OA-2 ("invariant needle updated accordingly") | Strictly stronger: the four new needles cover the new checks, and the `main.rs` needles are additive. |
| `cli_deadline_json_tests.rs::capture_deadline_with`: retry loop through `capture_attempt` | OA-3 | `Required` makes one attempt, as before. The tolerated mode retries at most `MAX_CLAMP_ATTEMPTS` = 20 times, and only when the server saw `UnexpectedEof` and the client returned `Timeout`. Every other server error still panics (accept, other I/O, decode). `Ok` with no capture still panics. The deadline assertion in the test body is unchanged. Removed `.expect("read length"/"read body")` became explicit `panic!` on every other path. |
| New GHF7 invariant: clauses on `notify_ready_stamped` removed (inside a new, untracked test) | OA-4 | Only those clauses were removed. The two message shapes and the `current_monotonic_nanos` requirement remain. The pre-existing `systemd_readiness_tests` and `presence_unlock_contract` tests are untouched. |

## 3. Deep Reasoning Audit

### Logic & Architecture

- **Enable re-check.** I traced Insert and RemoveRedundant: the hook runs once inside
  `write_atomic_checked`, before the rename. `plan == None` writes nothing and never calls the hook.
  `Restore` keeps its comparison (`dev`, `ino`, bytes) and its exact message. `Enable` also compares
  mode, uid and gid. The snapshot keeps the unmasked mode (`& 0o7777`), so an unchanged `0o664` file
  passes the re-check and is still written as `0o644`.
  → PASS
- **Backup.** `link_new_file` publishes with `hard_link`. On `EEXIST` it returns `None`, and that branch
  never touches the backup; a symlink planted at the backup path is never followed (tested). The
  `create_new` error on the temporary file is not mistaken for "backup exists": only the `hard_link`
  error is mapped. `discard_created_backup` removes the backup only when the inode matches and the PAM
  file holds no managed block. This also covers a directory fsync failure after the rename (the file now
  holds the block, so the backup is kept). → PASS
- **Priority in `plan_gdm_enable`.** The crossing error is moved ahead of `pristine == content` but guarded
  by `pristine != content`, so its behaviour is unchanged. Next comes E4 on `jump_skips_anchor()`, the same
  predicate `shared_soos_stack` uses. That gives the agreement. → PASS
- **Off-by-one in `jump_lands_in_stack`.** The target is the auth handler that comes after the `n` skipped
  ones (`skipped == n`), which is correct. Tested with `n = 2`: two following rules are refused, three are
  accepted. `kind` has its leading `-` stripped by `parse`, so `-auth` is counted and `-session` is
  skipped. `@include` parses as `AtInclude` and is refused. Unknown type keywords (`authx`, `foo`) are
  refused. `account include` inside the span is skipped, which is correct because libpam splices only the
  account group. A duplicate `success=` key is already rejected by `is_primary_soos_rule`, so the
  first-match lookup in `primary_success_jump` cannot disagree with libpam's last-wins rule. → PASS
- **Budget.** The decrement happens after the depth and name checks and before the open. A missing target
  still costs one unit. `checked_sub` refuses on the 33rd open. 32 opens are accepted and 33 refused, with
  nested opens counted. The status path goes through the same `delegated_auth`. → PASS
- **`wait_daemon_ready.sh`.** Only line 228 changed. No raw `${ADMIN_BIN}` remains in any message. → PASS

### PAM Concurrency & Deadlines

`crates/pam` is untouched. `test_pam.rs` is untouched, so PAM parity holds. The daemon change is one
clock read before `send_to_addr`, after validation and socket set-up. The write timeout is unchanged and
the clock is not read when the daemon is not supervised. → PASS

### Panic Safety & Fail-Closed

There is no `unwrap`, `expect`, `panic!` or indexing in new production code: `lines.get(..)`,
`checked_add` and `checked_sub` are used instead. A clock error yields `None` and the
`ready_sent_monotonic_us=unknown` message; it never blocks `READY=1`. Every new admin path is a refusal,
and status maps each one to `installed: false`. A wrong jump target that passes the check can only cost
availability (spec §4.5.1, documented). → PASS

### Test Integrity & Anti-Weakening

See §2. The new tests fail against plausible wrong implementations: the hook-count asserts, the exact E1
and E1b texts, the inode-preservation asserts, the off-by-one pair, the 32/33 boundary, the harness `sed`
pipeline replayed on a compact log line with ANSI colours, and `-auth` counted versus other groups not
counted. → PASS

### Memory, Bounds & Secrets

Reads are bounded: `MAX_PAM_FILE_BYTES` per file and at most 32 opens, so at most about 2 MiB per
analysis. The temporary backup file is created with `0600` and `create_new`, then `fchown` and `chmod`
are applied before the link. The only new logged value is a monotonic timestamp; no frames, embeddings
or credentials are logged. No `unsafe` was added. → PASS

### Supply Chain & Automation

No change to `Cargo.*`, `deny.toml` or `.github/`. The script changes are one message line and the
harness check. The GNU `sed` `\x1b` escape works in the Docker images (Debian and Arch, GNU sed). → PASS

### English-Only Policy

All code, comments, docs and walkthroughs are in English. A grep for French words found nothing. → PASS

## 4. Detailed Findings & Action Items

- **[MINOR]** `tests/invariants/src/gdm_hardening_flaky_tests_contract.rs:223`: the doc comment still says
  the daemon reports readiness "through `notify_ready_stamped`". After OA-4, `main.rs` calls
  `sd_notify::notify_ready()`, which is the stamped variant. Reword the comment when this file is next
  touched. Comment only, no behaviour change.
- **[MINOR]** `crates/admin-cli/src/gdm.rs` `publish_backup`: if `hard_link` succeeds but removing the
  temporary name or fsyncing the directory fails, the function returns an error without the identity of
  the backup it just created. The enable then aborts and leaves that backup in place. It holds the
  pristine bytes, so no data is lost and a later `gdm restore` or `gdm enable` stays correct. This is a
  root-only, I/O-failure-only case: a leftover file, never a security issue.
- **[SUGGESTION]** `crates/admin-cli/src/gdm.rs` `publish_backup`: the temporary name is
  `.<backup>.soos-tmp-<pid>`. If a run crashes after creating it and the PID is later reused, the next run
  fails once with `AlreadyExists` and removes the stale file, after which the run after that succeeds.
  This is acceptable as it is; reusing the random-suffix scheme of `write_atomic_checked`, if it has one,
  would avoid the one failed run.

## 5. Final Verdict

**VERDICT: APPROVED**
