# Walkthrough 217 — Pin the Refusal of `LockState` on the Auth-Reserved Connection

- **Date**: 2026-10-17
- **Issue**: GitHub #410 (GitHub-only, no backlog id), part of epic #352 — **Branch**: `test/daemon-lockstate-reserve`
- **Origin**: PR #403 follow-up "Daemon: do not admit LockState onto the auth-reserved permit" (walkthrough 214 §9)
- **Matrix criteria**: LRR1–LRR3 (new component `lockstate-reserve-refusal`). No existing row changes.

## 1. Context & Objectives

PR #403 (lock-status bridge and swaylock-plugin widget) left a follow-up: refuse `LockState` on the
auth-reserved permit "at accept time". A UID that already holds its two regular daemon connections
(for example a `soos-gui` preview and a `soos-remote` camera view) gets one extra connection,
reserved for a face `Auth` from `pam_soos.so` (ADR 2026-10-12 S-3). A lock-status poll of the same
user can land on that reserve.

Triage found that `main` (9e82795) already refuses it. `ConnectionDispatcher::read_and_process`
closes, without an answer, every message on an `AdmissionClass::AuthReserve` connection that is not
a `Request` with `kind == Auth`. That check runs after the first frame is classified and before
`handle_request`. What was missing was test evidence for `LockState`: the existing
`test_rsh_reserve_closes_non_auth_without_payload` covers only `Status`, `PreviewFrame` and the
`PasswordFailed` event. This PR adds that evidence and clarifies the wording. It changes no
production code.

## 2. Architect Design

Spec: `AI/architect_spec_lockstate_reserve_refusal.md`.

- **Where the refusal happens.** `handle_connection` chooses the admission class from
  `SO_PEERCRED` alone, before it reads any byte. `read_and_process` reads the whole first frame
  within `AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS` (200 ms) and classifies it. It refuses a
  non-`Auth` message before `handle_request`, so `handle_lock_state_request` never runs: no clock
  read, no quota record, no sessions scan, no board read, no bytes answered. The RAII permit is
  released when the connection drops.
- **Why not at accept time.** At accept time only the peer UID is known, and the bridge, the widget
  and `pam_soos.so` in the locker share that UID. To tell them apart, the daemon would have to read
  the first frame before admission (peek-before-admit). That adds a new class of unadmitted
  pending connections, each needing its own bound and timer, which is a change to the S-3 design.
  It needs an owner decision and is out of scope. The ADR 2026-10-15 item (3) residual stays as it
  is: the poll may occupy the reserve for its short exchange, and the daemon refuses it.
- **What is pinned.** The reserve answers nothing to `LockState`. That refusal consumes no
  `LockState` quota and touches no board state. Repeated polls cannot keep the reserve away from
  PAM.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`: **VALIDATION_VERDICT: APPROVED** (self-check against the
architecture, the ADRs, the backlog, the matrix and the code). One MINOR finding was adopted. The
check "the request beyond the quota is refused" is wall-clock dependent, so it runs only when the
requests fit inside the one-second window and is never a flaky hard assertion. The 8 answered
requests are asserted unconditionally.

## 4. Tester Contract

New tests only, in `crates/daemon/tests/peer_limits_tests.rs`, inside a new
`mod auth_reserve::lock_state_on_reserve`. The diff of the file is additions only: no existing
line, assertion, helper or constant is changed. The new tests reuse the existing `two_held`,
`permits_back`, `closed_promptly`, `exchange`, `auth_request` and `auth_response` helpers. Like
their siblings, they return early as root.

| Row | Test | Contract |
|---|---|---|
| LRR1 | `test_lrr_reserve_closes_lock_state_without_payload` | `LockState` (own UID) on the reserve: closed without payload, 6 permits free again |
| LRR2 | `test_lrr_reserve_lock_state_consumes_no_quota_and_leaves_the_board` | After one reserve refusal, all 8 `LockState` of one window answered on the regular connection `c1` (the next one refused when they fit in the window), board view and tracked UIDs unchanged (a revealed scan is published first, sequence 2), `Auth` on the reserve answered |
| LRR3 | `test_lrr_repeated_reserve_lock_state_cannot_hold_the_pam_reserve` | 16 reserve refusals, each releasing the permit; then the full quota on `c1` and an `Auth` on the reserve still answered (`Unavailable`, no pipeline) |

**Red evidence.** The behaviour is already on `main`, so the tests pass on unmodified code. To show
that they can fail, three temporary mutations of the reserve guard in
`crates/daemon/src/dispatcher.rs` were run, each reverted with `git checkout` and never committed:

| Mutation | Result |
|---|---|
| A: the guard also admits `LockState` on the reserve | LRR1, LRR2, LRR3 fail: `Capped connection must not receive a payload` |
| B: the guard records the `LockState` quota before refusing | LRR2 fails (`LockState 7 on a regular connection is answered ...`), LRR3 fails (`LockState 0 ...`) |
| C: the guard publishes a scan on the board before refusing | LRR2 fails: `the refused request leaves the board unchanged` |

## 5. Auditor Constraints

1. No production code, protocol wire or `crates/pam` change. Met: the diff touches the test file
   and documentation only.
2. No existing test or assertion modified, weakened or removed. Met: `git diff --numstat` of the
   test file shows 192 additions and 0 deletions.
3. Every wait is bounded. Met: reads go through `closed_promptly` (500 ms) or on a live connection
   that the daemon answers; `permits_back` is bounded at 2 s; the loop is bounded by
   `RESERVE_LOCK_STATE_ATTEMPTS` (16).
4. No flaky time dependence. Met: the only wall-clock-dependent assertion is conditional on
   staying inside the window. The suite passed 15/15 normally, 10/10 pinned to one CPU, and the
   new tests passed 30/30 pinned to one CPU with four busy loops on it.
5. The tests use their own temporary sockets and dispatchers, so they share no state with parallel
   tests. Met: every test uses `two_held` with a unique socket name.
6. No sensitive data. Met: no frame, embedding or credential involved; nothing is logged.

`Clearance: CLEARED`.

## 6. Implementation

- `crates/daemon/tests/peer_limits_tests.rs`: new module `auth_reserve::lock_state_on_reserve`
  with the helpers `lock_state_request`, `lock_state_answered`, `refuse_lock_state_on_reserve`
  and `assert_full_quota_available`, and the three tests above.
- `AI/VERIFICATION_MATRIX.md`: new component `lockstate-reserve-refusal`, rows LRR1–LRR3, Verified.
- `AI/DECISIONS.md`: dated clarification entry of 2026-10-17. It makes no new decision and amends
  no ADR. It says the refusal happens at first-frame classification, not at accept, and why
  accept-time refusal is not feasible without peek-before-admit.
- `Docs/IPC_PROTOCOL.md` §14 and `Docs/LOCKSCREEN_WIDGET.md` §7: the same clarification.
- `AI/architect_spec_lockstate_reserve_refusal.md`, `AI/plan_evaluator_report.md`.

## 7. Candid Review

Done by an independent reviewer, which records its findings in `AI/candid_review_report.md`, bound
to the diff fingerprint of this branch.

## 8. Verification Results

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked -p soos-daemon --all-features --all-targets -- -D warnings`: clean.
- `cargo test --locked -p soos-daemon --all-features -- --test-threads=3`: 777 passed, 0 failed.
- `peer_limits_tests`: 31 passed (3 new). Repeated 15 times: 0 failures. Pinned to one CPU, 10
  times: 0 failures. New tests pinned to one CPU with four busy loops, 30 times: 0 failures.
- Mutations A, B and C: each fails the expected test or tests (section 4).
- `cargo test --locked -p soos-invariants --all-features -- --test-threads=3`: 707 passed (matrix
  citations of LRR1–LRR3 resolved).
- The full workspace gates run through `./save.sh` before the push.

## 9. Known Limitations / Follow-ups

- The #403 follow-up "Daemon: do not admit LockState onto the auth-reserved permit" is **closed**.
  The daemon already refuses it at first-frame classification, and this PR pins that with tests.
  Literal accept-time refusal would need peek-before-admit, a change to the ADR 2026-10-12 S-3
  design. It is left for an explicit owner decision and is not planned.
- Not covered here (other #403 follow-ups): reducing the widget's 512 MiB worst-case shared
  memory, the tighter deadline test, the LKA1 wording, and the motion tokens in `theme.rs`.
