# Walkthrough 97 — Daemon Local Session Binding for Facial Auth

- **Date**: 2026-09-30
- **Issue**: Review finding DMN-09 (GitHub #160) — **Branch**: `fix/daemon-local-session-check`
- **Matrix criteria**: LSB1–LSB7 (new, ✅ Verified), LSB8 (hardware check, pending)
- **ADR**: 2026-09-30 "Local Session Binding for Facial `Auth`" in `AI/DECISIONS.md`

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, area Privileged Daemon)
confirmed a MAJOR finding:

- `SessionValidator` only parsed `UID=` and `ACTIVE=1`/`STATE=active`, so an SSH session
  (`REMOTE=1`, no seat) counted as an "active session".
- `verify_peer_credentials` lets any root peer target any UID, and the dispatcher rebuilt
  `PeerCredentials { pid: None }`, discarding the kernel PID that identifies the caller.
- `pam_soos.so` sits in the shared `auth` stacks used by `su`, `sudo` and `sshd`.

Impact: user A (local TTY or SSH) runs `su B` while B sits in front of the camera. `su` is setuid
root, B has an active session, the camera sees B, the daemon returns `Allow`, and A obtains B's
shell without B's password. An `ssh B@host` login through `common-auth` had the same outcome.

Objective: face verification only for a local, active, seat-attached session of the target user,
tied to the caller's own session when the peer is root; fail closed on every lookup failure;
logind access behind a mockable trait.

## 2. Architect Design

New module `crates/daemon/src/session_policy.rs` (the dispatcher change is limited to passing the
kernel PID and calling the policy):

- `SessionRecord { uid, active, remote: Option<bool>, seat, class }` with `parse`,
  `check_local_seat_session_of(uid)` and `is_active_non_remote_of(uid)`.
- `SessionDenial` (9 variants) with stable log codes (`as_str`).
- `trait LogindSource: Send + Sync + Debug` — `session_id_of_pid`, `session`, `sessions`.
- `SystemLogind` — production source over `/run/systemd/sessions` and `/proc`
  (`/proc/<pid>/cgroup` → `session-<id>.scope`, the mapping used by `sd_pid_get_session`).
- `parse_session_id_from_cgroup` — single well-formed session component or `None`.
- `LocalSessionPolicy { new, disabled, from_validator, is_enforced, authorize_auth }`.

Decision table (`authorize_auth(peer, target_uid)`):

| Peer | Rule |
|---|---|
| root | peer PID > 0 → caller session exists → `UID == target`, active, `REMOTE=0`, `SEAT` set, `CLASS=user` |
| `peer.uid == target` | target owns an active session not flagged `REMOTE=1` |
| other | `CallerSessionForeign` (defense in depth; Step 6 already rejects it) |

Legitimate cases: `sudo` in a local terminal (caller session = the user's seat session), polkit
helper and lock-screen workers running inside the user's session scope, screen lockers running
PAM as the user. Denied: `sudo`/`su` from SSH, `su <victim>` from another user's session, `sshd`
logins, remote-only users. The initial GDM/TTY login (no session yet) was already refused by the
previous "target owns an active session" rule and keeps using the password.

`DispatcherConfig` gains no field (existing tests build it exhaustively); the policy is derived
from the `SessionValidator` (same directory and enforcement flag) and can be overridden with
`ConnectionDispatcher::with_session_policy`. `with_session_validator` rebuilds the policy from the
new validator so both stay consistent.

## 3. Tester Contract (Red Phase)

`crates/daemon/tests/session_policy_tests.rs` (32 tests, `MockLogind` plus fake `/proc` and
sessions trees in `tempfile` directories, no systemd needed). Red evidence against the API-only
stub (commit `test(daemon): specify local-session policy contract for auth requests`):

```
test result: FAILED. 7 passed; 25 failed; 0 ignored
```

The 7 stub passes are the trivially satisfied allow cases and negative parsing cases; every deny
case, parser case and dispatcher case failed on its assertion. Existing tests were not modified:
`policy_concurrency_tests::test_session_validator_parses_active_session` and
`test_auth_rejected_for_uid_without_active_session` (non-root peer, fixtures without `REMOTE=`)
keep their exact expectations under the unprivileged-peer rule.

## 4. Auditor Constraints

1. No `unwrap`/`expect`/indexing in the new production code; all errors map to a `SessionDenial`.
2. Bounded I/O: session records ≤ 4 KiB, cgroup files < 16 KiB (reaching the bound fails closed),
   at most 1024 directory entries scanned (more fails closed).
3. Session IDs are ASCII alphanumeric, 1–64 bytes, validated before any path join; record files
   are read only if `symlink_metadata` reports a regular file.
4. Logs carry only the peer UID, target UID and a reason code; session contents (`REMOTE_HOST`,
   `USER`, `SERVICE`) are never logged.
5. No error path yields `Allow`; the PAM module maps the `ProtocolError` reply to `PAM_IGNORE`.

## 5. Implementation (Green Phase)

- `session_policy.rs`: parser, cgroup resolver, `SystemLogind`, `LocalSessionPolicy`.
- `dispatcher.rs`: `handle_request` receives `peer.pid`; the rebuilt `PeerCredentials` keeps it;
  Step 6b calls `session_policy.authorize_auth` for `RequestKind::Auth` and answers
  `ProtocolError`/`UidMismatch` on denial (reason class unchanged, no protocol change).
- `session.rs`: `SessionValidator` (preview path) reuses `SessionRecord::parse` and ignores
  sessions flagged `REMOTE=1`.
- `lib.rs`: exports the module and `LocalSessionPolicy`, `LogindSource`, `SessionDenial`.

## 6. Verification

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean.
- `cargo test --locked --workspace --all-targets --all-features --no-fail-fast`: all green,
  `session_policy_tests` 32/32. The only failure seen during the gate was the wall-clock
  benchmark `soos-vision::bench_tests::test_pipeline_latency_budget_under_150ms_p95` on a host with
  load average above 40 (parallel builds); it passes when re-run alone and is unrelated.
- `./scripts/candid_review.sh`: passed.
- Docker PAM matrix: not affected (it drives `pam_soos.so` against `tests/docker/mock_daemon.py`,
  not the Rust dispatcher).

## 7. Residual Risks & Follow-ups

- **Hardware check (LSB8)**: confirm on a logind host that the GDM reauthentication worker, KDE and
  `swaylock` run inside the user's session scope, so lock-screen face unlock keeps working;
  otherwise they fall back to the password (safe, but a usability regression).
- **PID reuse**: `SO_PEERCRED` reports the PID at `connect(2)`; the PAM host blocks on the reply,
  so reuse before the cgroup read is unlikely. `SO_PEERPIDFD` (Linux ≥ 6.5) would close it.
- **`ProtectProc=invisible` (review DMN-14)**: would hide other users' `/proc/<pid>` from the
  daemon and make every root-peer request fail closed; that hardening must preserve read access
  to `/proc/<pid>/cgroup`.
- The initial GDM/TTY login keeps using the password; a greeter-aware rule would need a reliable
  way to tie the display-manager worker to the greeter's seat and is out of scope.
