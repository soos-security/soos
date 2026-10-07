# Walkthrough 186 — Opt-In Remote Unlock in `soos-remote`

- **Date**: 2026-10-06
- **Issue**: GitHub #339 (GitHub-only, no backlog id; commits carry `Refs #339`) — **Branch**:
  `feat/remote-companion` (draft PR #340). **Not merged** until the owner gives an explicit go.
- **ADR**: 2026-10-06 "Remote Unlock in `soos-remote`, Tailscale Identity Only, Opt-In"
  (supersedes, for this crate only, the "status and lock only" scope of the 2026-10-05 ADR and
  the original invariant RMC-S3).
- **Matrix criteria**: RMC22–RMC24 verified, RMC25 automated part verified (phone check pending);
  RMC21 annotated with the owner's 2026-10-06 report (web app and *Lock now* work on the iPhone).

## 1. Context & Objectives

After confirming on the iPhone that the status page and *Lock now* work (walkthrough 185), the
owner asked for the reverse action: unlock the PC from the phone. The owner chose two settings
explicitly when offered a passkey / Face ID step and an automatic re-lock:

1. **Authentication**: the Tailscale identity alone (no passkey, PIN or Face ID step);
2. **Re-lock**: none (the PC stays unlocked until it locks again by itself).

The ADR records the resulting risk: an unlocked phone (or any device logged in to an allowed
Tailscale login) on the tailnet can unlock the PC.

## 2. Design (Phase 1)

- **Mechanism**: `Manager.UnlockSession(id)` on the session `select_session` already picks for
  the lock (`Class=user`, `Remote=false`, on a seat, the active one first). logind emits
  `Unlock`; GNOME and Plasma react natively, and the sway family through the `swayidle`
  `unlock 'pkill -USR1 …'` hook already used by the daemon's presence auto-unlock. The owner's
  host already runs that hook (driftwm + `swaylock-plugin`). logind lets a user unlock their own
  session without a polkit rule, so the service gains no new right.
- **Opt-in**: `allow_unlock` (TOML boolean, default `false`) in `remote.toml`. Disabled ⇒
  `403 {"result":"unlock_disabled"}` and no logind call. This keeps the merged default
  behaviour identical to walkthrough 185 for any user who does not ask for it.
- **Route and CSRF**: `POST /api/unlock`, `405 Allow: POST` otherwise. `check_unlock_csrf` runs
  the same rules as the lock with its own action value (`X-Soos-Action: unlock`); both routes
  share one private `check_action_csrf`, so neither accepts the other's value. Dispatch order:
  host (`421`) → identity (`403`) → route → CSRF (`403 forbidden`) → `allow_unlock`
  (`403 unlock_disabled`) → flow. The CSRF check runs first, so a cross-site caller never learns
  whether unlock is enabled.
- **Flow** (`unlock_flow`, mirror of `lock_flow`): own gate and own rate limit
  (`MIN_UNLOCK_INTERVAL_MS` = 2000, independent of the lock), fresh snapshot, `409 no_session` /
  `409 already_unlocked`, record the interval, one `info` audit line
  `remote unlock requested` (no identity, no session id), `UnlockSession`, `202
  unlock_requested`; any failure ⇒ `503 unavailable`; the whole flow is bounded by
  `UNLOCK_FLOW_DEADLINE_MS` = 2000.
- **logind adapter**: `SessionSource` gains `unlock_session`; `ZbusSessionSource` shares one
  `session_action(id, method)` helper between `LockSession` and `UnlockSession`, so each method
  name appears exactly once as a string literal.
- **Page**: an outlined *Unlock now* button under *Lock now*, enabled only while the state is
  `Locked`, behind a `window.confirm("Unlock the PC now?")` tap (protection against a stray tap,
  not an authentication factor); confirmation through the stream (`Unlocked`) or, after 5 s,
  "The desktop did not confirm the unlock (LockedHint unchanged)".

## 3. Tests (Phase 2) and the superseded contract lines

New tests (red before the implementation: they failed on the missing API only):

| Test | Proves |
|---|---|
| `config_tests::test_rmc_unlock_constants_match_the_adr` | the three new constants |
| `config_tests::test_rmc_parse_config_allow_unlock_is_opt_in` | default `false`, boolean only, allowlist still required |
| `routes_tests::test_rmc_unlock_route`, `test_rmc_check_unlock_csrf` | route table, `Allow: POST`, CSRF, no action confusion |
| `server_tests::test_rmc_unlock_disabled_by_default_never_reaches_logind` | `403 unlock_disabled`, zero logind reads |
| `server_tests::test_rmc_unlock_flow_and_rate_limit` | 202 / 409 / 503 / 429, interval rules, one attempt |
| `server_tests::test_rmc_unlock_and_lock_rate_limits_are_independent` | independent gates |
| `server_tests::test_rmc_unlock_requires_identity_host_and_csrf` | every refusal before logind |
| `server_tests::test_rmc_disabled_unlock_checks_csrf_first` | CSRF before `unlock_disabled` |
| `server_tests::test_rmc_unlock_route_is_post_only` | `405` for GET/HEAD/PUT/DELETE |
| `server_tests::test_rmc_unlock_flow_deadline` | `503` at exactly 2000 ms, healthy afterwards |
| `server_tests::test_rmc_unlock_is_audited_without_identity` | one `INFO` audit line, no identity in logs |
| `remote_companion_contract::test_rmc_unlock_is_opt_in_and_documented` | doc, page and ADR needles |

Three assertions of the walkthrough-185 contract encoded the old "never unlock" scope and are
**superseded by the ADR**, not weakened to make code pass:

- `routes_tests` and `server_tests` listed `/api/unlock` among the `404` paths; it is now a real
  `POST` route, so the entries became `/api/unlock/` (and `/API/UNLOCK`), which still prove exact
  path matching;
- `test_rmc_s3_no_unlock_or_locked_hint_literal` forbade the `"UnlockSession"` literal; it now
  requires exactly one, in `logind.rs`, and still forbids `"UnlockSessions"`, `"Unlock"`,
  `"SetLockedHint"`, `"TerminateSession"`, `"KillSession"` and `"ActivateSession"`.

Harness changes are setup only: `MockSource` gained an `unlock_session` result, spy and hold
gate; `Options` gained `allow_unlock`; the `RemoteConfig` literal gained `allow_unlock`.

## 4. Audit (Phase 3)

- No `unwrap`/`expect`/indexing in production code; no new `unsafe`; `#![forbid(unsafe_code)]`
  unchanged.
- Every new await is bounded (`UNLOCK_FLOW_DEADLINE_MS` around snapshot and call; the snapshot
  keeps its own 1500 ms bound; the D-Bus call keeps its 500 ms bound).
- No identity, session id, host, path or header value in any new log line (tested at `TRACE`).
- Fail closed: a disabled flag, a CSRF failure, a logind error or a timeout never calls
  `UnlockSession` or answers `202`.
- No new dependency.

## 5. Host Deployment

```sh
./scripts/install_remote.sh && systemctl --user restart soos-remote
# then, to enable the feature:
echo 'allow_unlock = true' >> ~/.config/soos/remote.toml
systemctl --user restart soos-remote
```

## 6. Remaining Work

- RMC25 phone check: with `allow_unlock = true`, lock the PC, tap *Unlock now* on the iPhone,
  confirm, and observe the lock screen closing and the page showing `Unlocked`.
- RMC21 Shortcuts steps ("Lock PC", and optionally "Unlock PC").
- Future ADRs: passkey / Face ID step-up for the unlock, push notifications, live camera.
