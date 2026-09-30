# Walkthrough 97 — Daemon Local Session Binding for Facial Auth

- **Date**: 2026-09-30
- **Issue**: Review finding DMN-09 (GitHub #160) — **Branch**: `fix/daemon-local-session-check`
- **Matrix criteria**: LSB1–LSB7 and LSB9–LSB12 (✅ Verified), LSB8 (hardware check, pending)
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

Legitimate cases (see §8 for the user-manager rework): `sudo` in a local terminal (caller session = the user's seat session), polkit
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

## 8. Rework — Callers Under the systemd User Manager (candid review finding 1)

### Finding

The candid review (MAJOR) showed that the root-peer rule only resolved a caller through a
`session-<id>.scope` component. GNOME ≥ 3.34 and KDE Plasma ≥ 5.25 run terminal emulators and the
shell's polkit agent under the systemd user manager, for example
`0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.gnome.Terminal.slice/vte-spawn-1234.scope`
and `0::/user.slice/user-1000.slice/user@1000.service/session.slice/org.gnome.Shell@wayland.service`.
`sudo` in a desktop terminal and polkit prompts therefore always fell back to the password, and the
original tests hid this by placing the terminal `sudo` inside a session scope.

### User decision (2026-09-30)

A root peer whose cgroup resolves to no session scope is also allowed when all of:

1. its cgroup is `/user.slice/user-<uid>.slice/user@<uid>.service/<child>...` with both `<uid>`
   parsed strictly (ASCII decimal, no sign, no leading zero, fits `u32`) and equal to each other and
   to the target UID;
2. the target owns at least one active, `REMOTE=0`, seat-attached, `CLASS=user` session;
3. the target owns no remote session at that moment, because an SSH session of the same account
   could otherwise reach the user manager through `systemd-run --user sudo ...`.

The session-scope and unprivileged-peer rules are unchanged. The ADR amendment records the
residual risk: malware already running as the user in the desktop can use face `sudo`, which is
inherent to face `sudo`.

### Design

- `LogindSource::cgroup_of_pid(pid)` returns the raw bounded cgroup content. The default method
  returns `Ok(None)`, so a source that does not implement it denies (fail closed). `SystemLogind`
  reads `/proc/<pid>/cgroup` under the same 16 KiB bound.
- `parse_user_manager_uid_from_cgroup(content) -> Result<Option<u32>, SessionDenial>` considers
  only the systemd-managed hierarchies (unified `0::` and v1 `name=systemd`). All considered lines
  must agree. A malformed user-manager path or disagreeing lines produce `UserManagerCgroupMalformed`.
- `LocalSessionPolicy::authorize_user_manager_caller` runs only when `session_id_of_pid` returns
  `None`. Its checks in order: UID match (`UserManagerUidMismatch`), `sessions()` readable
  (`LogindUnavailable`), no session of the target with `REMOTE != 0` (`UserManagerCallerRemoteSessionActive`),
  then a local seat session exists (`UserManagerNoLocalSeatSession`).
- The remote check is deliberately stricter than "active remote session". It counts any
  state, so a closing SSH session with a lingering `tmux` is also refused. It also counts an unknown
  `REMOTE=` value and a record without `UID=`. `SessionRecord` has no field for `STATE=online`
  versus `closing`, and its layout is part of the existing contract.
- Hardening found while writing the contract: `parse_session_id_from_cgroup` accepted a
  `session-<id>.scope` component anywhere in the path. A user can name a transient unit
  `session-2.scope` inside its own user manager (`systemd-run --user --scope --unit=session-2`),
  which would have tied `su <victim>` to the victim's seat session. The parser now applies the
  `sd_pid_get_session` rule: only the first unit below the slices counts. The existing parsing tests
  keep their expectations.

### Red → Green

New tests in `crates/daemon/tests/session_policy_tests.rs` (the `MockLogind` helper gained a
`pid_cgroups` map and a `cgroup_of_pid` implementation; no existing test was changed). Red run
against the API-only stubs (commit `test(daemon): specify user-manager caller contract for root
auth peers`):

```
test result: FAILED. 36 passed; 11 failed; 0 ignored
```

| Test | Red | Green |
|---|---|---|
| `test_user_manager_cgroup_real_desktop_layouts_are_resolved` | `left: Ok(None) right: Ok(Some(1000))` | ✅ |
| `test_user_manager_cgroup_malformed_uid_fails_closed` | `left: Ok(None) right: Err(UserManagerCgroupMalformed)` | ✅ |
| `test_cgroup_session_scope_nested_in_user_manager_is_not_a_session` | nested `session-2.scope` resolved to `Some("2")` | ✅ |
| `test_root_peer_sudo_in_gnome_terminal_user_manager_is_allowed` | `Err(CallerSessionUnresolved)` | ✅ |
| `test_root_peer_polkit_from_gnome_shell_user_manager_is_allowed` | `Err(CallerSessionUnresolved)` | ✅ |
| `test_root_peer_user_manager_denied_when_target_has_remote_session` | `Err(CallerSessionUnresolved)` instead of `UserManagerCallerRemoteSessionActive` | ✅ |
| `test_root_peer_su_other_user_from_user_manager_terminal_is_denied` | `Err(CallerSessionUnresolved)` instead of `UserManagerUidMismatch` | ✅ |
| `test_root_peer_user_manager_malformed_uid_is_denied` | `Err(CallerSessionUnresolved)` | ✅ |
| `test_root_peer_user_manager_without_local_seat_session_is_denied` | `Err(CallerSessionUnresolved)` | ✅ |
| `test_root_peer_user_manager_logind_unavailable_fails_closed` | `Err(CallerSessionUnresolved)` | ✅ |
| `test_system_logind_user_manager_caller_real_layout` | `cgroup_of_pid` returned `Ok(None)` | ✅ |
| `test_user_manager_cgroup_outside_user_manager_is_none` | passes on stub (negative case) | ✅ |
| `test_root_peer_session_scope_takes_precedence_over_user_manager` | passes on stub (unchanged rule) | ✅ |
| `test_denial_reason_codes_cover_user_manager_variants` | passes on stub (variants declared) | ✅ |
| `test_system_logind_user_manager_oversized_cgroup_fails_closed` | passes on stub (denies) | ✅ |

Green: `session_policy_tests` 47/47, with all 32 original tests unchanged.

### Review minors

- `Docs/IPC_PROTOCOL.md`: the second `## 10.` became `## 11.` (Per-Peer Connection Limits),
  and the reference in walkthrough 95 was updated. The allowed-callers table now lists the
  user-manager row.
- `tests/docker/test_suite.sh`: the stale `T11 artifacts` comment now reads `T12 artifacts`.
- Suggestions 4 (first-inference warm-up) and 5 (evidence-store global daily cap and
  `daily_counts` pruning) are recorded as follow-ups in walkthroughs 96 and 95.

### Verification (rework)

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean.
- `cargo test --locked --workspace --all-targets --all-features --no-fail-fast`: all green.
- `./scripts/candid_review.sh`: passed.
- LSB8 (hardware) stays pending. It now also covers checking the real cgroup of `sudo` in GNOME
  Terminal/Konsole and of the gnome-shell polkit agent on a logind host.
