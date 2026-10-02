# Walkthrough 176 — Presence Auto-Unlock Review Follow-ups

- **Date**: 2026-10-02
- **Issue**: GitHub #325 (GitHub-only, no backlog sub-issue; the branch is not registered in
  `scripts/sync_issue.py`, the commit carries `Closes #325`) — **Branch**:
  `fix/presence-review-followups`
- **Matrix criteria**: PFU1–PFU7 (`AI/VERIFICATION_MATRIX.md` section
  `presence-review-followups`), all `✅ Verified`
- **Phase documents**: `AI/architect_spec_presence_followups.md` (revision 3),
  `AI/plan_evaluator_report.md` (rounds 1–3, `APPROVED`), `AI/tester_contract_presence_followups.md`
  (50 new tests plus the PFU7 migration), `AI/auditor_constraints_presence_followups.md`
  (constraints A1–A18)

## 1. Context & Objectives

The review of PR #324 (presence auto-unlock, GitHub #323, walkthrough 175) left six follow-up
items, collected in GitHub #325:

1. The rate-limit attempt of a scan was stamped with the clock read at the start of the tick,
   although the account guard and the logind steps run between that read and the recording.
2. `DBUS_CONNECT_TIMEOUT_MS` (1000 ms) could never fire: the lazy connection was opened inside
   the first call, which already ran under the 500 ms call bound.
3. The PAM line scan of the account guard was token-based and could under-detect libpam on
   malformed files; `/etc/pam.conf` was ignored.
4. The "one account check in flight" behaviour had no behavioural test.
5. The worker polled logind every second even when nobody was enrolled.
6. `cli_deadline_json_tests::test_simulate_pam_auth_zero_timeout_is_clamped_to_pam_minimum`
   failed rarely under heavy CPU load.

## 2. Architect Design

- **Fresh stamp (PFU1)**: step 10 of `PresenceWorker::tick` takes the policy write lock, then
  reads the clock, then records the attempt with that value, which also marks the scan start.
  A clock error, or a reading older than the step-3 one, skips the tick as `ClockUnavailable`.
- **Connect step (PFU2)**: new trait method `PresenceLogind::connect()` with a default no-op
  (test doubles stay unchanged). `ZbusLogind::connect` is the only place that builds a bus
  connection (pinned address, `method_timeout`, `max_queued`, its own 1000 ms timeout around
  `build()`). The former lazy `connection()` became the pure accessor `current()`, which
  returns `BusUnavailable` on an empty slot. The worker awaits `connect()` under
  `DBUS_CONNECT_TIMEOUT_MS`, outside `bounded()`, right before the snapshot. Option (b),
  lowering the connect bound below the call bound, was rejected.
- **`/etc/pam.conf` (PFU3)**: `DEFAULT_PAM_CONF` and `SystemAccountGuard::with_pam_conf`. When
  none of the PAM directories is a directory (`metadata`, symlinks followed), a present
  `pam.conf` is `Undeterminable`. Next to a directory it is scanned like a stack file.
- **Superset scan (PFU4)**: a logical line is a hit when the text after the **first**
  `pam_faillock.so` contains any policy-option substring, or when some token is `include`,
  `substack` or `@include` (ASCII case-insensitive) and any later token holds `/`. The token
  matcher (`pam_tokens`) was removed.
- **Enrollment probe (PFU6)**: `BiometricStore::has_enrolled_template` (bounded by
  `MAX_ENROLLMENT_PROBE_ENTRIES` = 4096, `DirEntry::file_type`, canonical UID names, nothing
  opened, no store lock) runs as step 3b of every tick, before any D-Bus access. `false` skips
  as `NotEnrolled`, an error as `TemplateStoreError`; both clear the lock tracker.
- **PFU5**: no production change (the flag already existed); a behavioural test was added.
- **PFU7**: no production change (see §6).

## 3. Plan Evaluation

Round 1 found two major issues. F1: the token matcher under-detects libpam on two
counter-examples (an unterminated bracket opened after a no-break space, and a `\` before a
`#` comment). F2: the empty-store change breaks an existing worker test. The fixes were the
substring superset rule and a setup-only migration. Round 2 found that an `include` /
`substack` of a path is never followed (R2-F1), so such a line is now a hit, and that the
module is matched by name only (R2-F2), which is now documented. Round 3 approved revision 3.

## 4. Tester Contract

50 new tests:

- `presence_followups_tests` (20);
- `presence_followups_connect_tests` (5);
- `presence_followups_pam_conf_tests` (11);
- `enrollment_probe_tests` (8);
- `presence_followups_contract` invariants (6).

Setup-only migrations, with no assertion changed:

- `with_pam_conf(<tempdir>)` in three guard builders, so no test reads the host `/etc/pam.conf`;
- in the "not enrolled" case of `test_pau_unusable_templates_cost_no_attempt_and_no_camera`,
  UID 1001 is enrolled so that the store is not empty.

## 5. Auditor Constraints

The auditor issued constraints A1–A18. A13 and A14 were blocking tester additions: one case
for the first occurrence of the module name versus the last, and one for an include path
that is not in the next token. The tester added both, and the implementation was cleared.

## 6. Owner Decisions on the Flaky admin-cli Test (PFU7)

Diagnosis: `simulate_pam_auth` clamps `0` to `MIN_TIMEOUT_MS = 10` and starts one cumulative
deadline before `connect`. This is the PAM-parity contract of #231 / #312, and production stays
unchanged. The mock server thread must be scheduled, accept, read, stamp and reply within
those 10 ms. Under single-CPU contention it is sometimes not, and the client correctly reports
`Timeout`.

The harness runs 64 concurrent instances pinned to CPU 0 for 3 rounds (192 runs per
measurement).

| Measurement | Failures | Note |
|---|---|---|
| Idle host | 0/50 | |
| Clamp test, before any change | 3/192 | earlier session |
| Clamp test, before any change | 4/192, 5/192 | |
| 250 ms control test | 0/192 | |
| After option (b) | 2/192, 8/192, 18/192 | host load varied |
| After option (a) | 2/192, 0/192, 0/192 | |
| 250 ms control test, after option (a) | 0/192 | |

- **Option (b)**, tried first: a setup-only readiness channel, signalled by the server right
  before `accept()`. It made no measurable difference. All failures were still
  `simulated authentication must complete: Timeout`. The channel only removes the "thread not
  yet started" part of the window, not the wake-up latency under contention. It was reverted:
  the client's connect and write go to the listen backlog and the socket buffer whatever the
  server thread is doing.
- **Option (a)**, approved by the owner on 2026-10-02: in this one test, `simulate_pam_auth`
  may return `Ok` or `Err(AdminCliError::Timeout)`. Any other error still fails. The deadline
  assertion is unchanged and always runs. The server captures `deadline_monotonic_ns` from the
  request, and the test fails if that value does not arrive within 5 s. The 250 ms and
  oversized-timeout tests still require success.
- **Residual 2/576 failures** (2/192, 0/192 and 0/192 over three measurements after option
  (a)): in these runs the mock server read `UnexpectedEof` and no deadline was captured. The
  client itself was descheduled past its 10 ms deadline **before** it wrote the request. That
  is correct production behaviour: nothing is written after the deadline, so there is no
  `deadline_monotonic_ns` to check. Under the owner rule "fail if the captured value never
  arrives", this residual stays. It only appears under extreme single-CPU stress. Removing it
  would mean skipping the deadline assertion when no request was sent, which would need a new
  owner decision.

## 7. Implementation

- `crates/daemon/src/presence/worker.rs`:
  - step 3b, the enrollment probe;
  - step 4a, the connect step under `DBUS_CONNECT_TIMEOUT_MS`, which on failure calls
    `logind_failed`, clears the tracker and returns `LogindUnavailable`;
  - step 10, the fresh stamp under the policy write lock, with the guard dropped before every
    return;
  - the `NotEnrolled` doc was broadened.
- `crates/daemon/src/presence/logind.rs`:
  - the `connect()` trait method with a default no-op;
  - `ZbusLogind::connect` builds the connection;
  - the accessor `current()` replaces the lazy builder;
  - the docs now name `connect()`.
- `crates/daemon/src/presence/account.rs`:
  - `pam_conf` field and `with_pam_conf`;
  - `any_pam_dir`, `check_pam_conf` (called first in `check_pam_stacks`);
  - the substring and include rules (`sets_faillock_option`, `includes_a_path`); the
    `pam_tokens` matcher was removed;
  - doc comments on over-detection, the superset, Unicode whitespace, the `pam.conf` rule and
    the name-only limit.
- `crates/daemon/src/presence/mod.rs`:
  - `DEFAULT_PAM_CONF`;
  - documented coverage of `DBUS_CALL_TIMEOUT_MS` (whole snapshot, every round trip) and
    `DBUS_CONNECT_TIMEOUT_MS` (`connect()`, outside the call bound). Both values are unchanged.
- `crates/biometric-store/src/store.rs`: `MAX_ENROLLMENT_PROBE_ENTRIES`,
  `has_enrolled_template` and the private `is_template_file_name`.
- Docs:
  - `Docs/DAEMON.md` §6 (enrollment probe, connect step, what the 500 ms bound covers, the
    fresh stamp, the `/etc/pam.conf` rule, the over-detecting scan, the `VENDORDIR=/usr/etc`
    assumption);
  - `Docs/BIOMETRIC_STORE_CRATE.md` §3.6 and §4.2;
  - the ADR amendment in `AI/DECISIONS.md`;
  - the matrix section `presence-review-followups`.

`crates/pam`, the IPC protocol, `crates/daemon/src/main.rs` and the dependency set are
untouched.

## 8. Verification Results

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean.
- `cargo test --locked --workspace --all-targets --all-features`: 2618 passed, 0 failed,
  6 ignored.
- `cargo deny --locked check`: advisories, bans, licenses and sources ok (no new dependency).
- `./scripts/candid_review.sh` (layer 1): passed.
- `python3 scripts/sync_issue.py --check`: passed.

## 9. Known Limitations / Follow-ups

- The module is recognised by name: a renamed copy of `pam_faillock.so` is not detected. This
  is a root-only configuration and part of accepted risk (d).
- The guard assumes libpam's `VENDORDIR=/usr/etc`.
- The PFU7 residual described in §6.
- `BiometricStore::list_enrolled` is still unbounded and accepts non-canonical names. It is a
  diagnostic API that presence does not use (pre-existing, auditor note).
