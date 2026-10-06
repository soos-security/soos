# Candid Review Report

- **Date**: 2026-10-06
- **Target Branch**: `feat/remote-companion`
- **Base (merge-base)**: `222665f`
- **Reviewed-Diff-Fingerprint**: `500c66533e50203330088166c49e2d9450770aa031743976c443f4468941282f`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `AGENTS.md`,
  `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/MOCK_STRATEGY.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/architect_spec_remote_companion.md`, `AI/auditor_constraints_remote_companion.md`,
  `AI/tester_contract_remote_companion.md`, `AI/walkthroughs/183_remote_companion.md`,
  `AI/walkthroughs/184_remote_unlock.md`, `Cargo.lock`, `Cargo.toml`, `Docs/README.md`,
  `Docs/REMOTE_COMPANION.md`, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`,
  `crates/remote/Cargo.toml`, `crates/remote/assets/{app.js,apple-touch-icon.png,icon.svg,index.html,manifest.webmanifest,style.css}`,
  `crates/remote/src/{assets,config,http,identity,lib,logind,main,routes,server,session,socket,status}.rs`,
  `crates/remote/tests/{config,http,identity,routes,server,session,socket}_tests.rs`,
  `packaging/soos-remote.service`, `scripts/candid_review.sh`, `scripts/install_remote.sh`,
  `tests/invariants/src/{lib,presence_unlock_contract,remote_companion_contract}.rs`

## 1. Executive Summary

The frozen diff contains the whole `soos-remote` companion branch (status + lock, committed in
`0b0b50c` / `e3a897b`) plus the uncommitted opt-in remote unlock (ADR 2026-10-06). The review
focused on the new unlock surface, since it is the only state-changing capability added and it
deliberately widens invariant RMC-S3. The unlock is opt-in (`allow_unlock` defaults to `false`,
only a TOML boolean is accepted), passes through every existing check (effective host, identity
allowlist, CSRF with its own action value) before any logind access, is rate limited and bounded
by its own 2 s deadline, never calls logind while disabled, and logs a single identity-free audit
line. The crate still names no session-ending method and is a leaf (`pam_soos.so`, the daemon and
the IPC protocol are untouched). `cargo test -p soos-remote -p soos-invariants` passes
(469 invariants + all remote suites) and `cargo clippy -p soos-remote --all-targets -D warnings`
is clean. The two MINOR documentation findings and the suggestion of the first pass are resolved; no CRITICAL/MAJOR/MINOR finding remains.

## 2. Test Changes

Mechanical listing (`target/candid_diff.patch`):

- Test files touched: all `crates/remote/tests/*` (new crate, new files), and in
  `tests/invariants/src/`: `lib.rs`, `presence_unlock_contract.rs`, `remote_companion_contract.rs`.
- Removed/changed assertions (`^-` with assert): one hit, patch line 11698,
  `presence_unlock_contract.rs::test_pau_zbus_is_used_only_by_the_daemon`: the single daemon
  assertion becomes a loop over `crates/daemon` and `crates/remote`, each still required to declare
  `zbus = { workspace = true }` and name `zbus` exactly once; every other manifest still must not
  mention `zbus`, the PAM assertion is unchanged. Justified by ADR 2026-10-05 item (7) (owner
  decision, documented "Contract migration" comment, matrix PAU17). Not a weakening: it is
  stricter for the daemon (exact count) and keeps the PAM exclusion.
- Other changes to pre-existing test content (not caught by the assert grep, found by reading the
  working-tree diff):
  - `remote_companion_contract.rs::test_rmc_s3_no_unlock_or_locked_hint_literal`: `"UnlockSession"`
    removed from the forbidden list and replaced by "exactly one `"UnlockSession"` literal, in
    `logind.rs`". Justified by ADR 2026-10-06 item (5) (owner decision explicitly superseding
    RMC-S3 for this crate). `"Unlock"`, `"UnlockSessions"`, `"SetLockedHint"`,
    `"TerminateSession"`, `"KillSession"`, `"ActivateSession"` remain forbidden.
  - `routes_tests.rs` / `server_tests.rs`: `/api/unlock` removed from the "unknown path ⇒ 404"
    lists and replaced by `/api/unlock/` and `/API/UNLOCK` (which stay 404). Justified by the same
    ADR (the route now exists); exact-match routing is still pinned.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): none.
- Inline `mod tests` changes: none in production files of the diff other than the invariants
  crate's own test module (`lib.rs` adds `"remote"` to the forbid-unsafe business-crate list: a
  strengthening).

## 3. Deep Reasoning Audit

### Logic & Architecture

- Scenario: unlock route reached with `allow_unlock = false` → `403 unlock_disabled`, zero logind
  reads (`test_rmc_unlock_disabled_by_default_never_reaches_logind`). PASS.
- Scenario: action confusion (`X-Soos-Action: lock` on `/api/unlock`, `unlock` on `/api/lock`,
  `UNLOCK`, duplicated header) → `403` on both routes; shared `check_action_csrf` compares the exact
  bytes per route. PASS.
- Scenario: unlock of a remote, greeter or absent session → `409 no_session`; of an already
  unlocked session → `409 already_unlocked`; neither records the interval. Same `select_session`
  as the lock (own uid, `Class=user`, `Remote=false`, seat). PASS.
- Scenario: failed `UnlockSession` → `503`, single attempt, interval still recorded (no rapid
  retry amplification). PASS.
- `allow_header` now strips the query string, aligning it with `route()` (previously
  `/api/lock?x` returned `Allow: GET, HEAD`): correct small fix.
- Lock and unlock gates are independent mutexes; each flow is serialized under its own gate and
  bounded by its own `timeout`. PASS.
- logind's `UnlockSession` on the caller's own session needs no polkit rule (same check as
  `LockSession`), consistent with the "no new right" claim. PASS.

### PAM Concurrency & Deadlines

- `crates/pam` is not touched; `pam_soos.so` still never links `zbus`
  (`test_pau_zbus_is_used_only_by_the_daemon`). The companion is a leaf (RMC-S4). PASS.

### Panic Safety & Fail-Closed

- No `unwrap`/`expect`/indexing added in `crates/remote/src` by the unlock change; `split('?').next()`
  uses `unwrap_or`. Every logind error or deadline maps to `503 unavailable`; a logind failure is
  never reported as `unlocked` (RC-3 unchanged). The disabled state cannot be bypassed: the check
  sits after CSRF and before `unlock_flow`, the only caller of `unlock_session`. PASS.

### Test Integrity & Anti-Weakening

- See §2. New unlock tests would fail against plausible wrong implementations: default `true`,
  string `"true"` accepted, shared lock/unlock rate limit, logind read before CSRF, retry on
  failure, missing deadline (exact `UNLOCK_FLOW_DEADLINE_MS - 1` / `+1` boundary), audit line
  leaking login/session id/host/uid. PASS.

### Memory, Bounds & Secrets

- No new allocation path; request bounds unchanged; unlock adds a 2 s flow bound and a 2 s rate
  limit. The audit line `remote unlock requested` carries no identity, session id, host or path
  (asserted by `test_rmc_unlock_is_audited_without_identity`). No credentials are involved. PASS.

### Supply Chain & Automation

- `zbus` scope widened to exactly two crates by ADR, same workspace pin, `tokio` feature only;
  `scripts/install_remote.sh` refuses root and runs no `sudo`/`tailscale` (RMC-S6);
  `packaging/soos-remote.service` is sandboxed with `RestrictAddressFamilies=AF_UNIX` and
  `RestartPreventExitStatus=78`. No workflow changes. PASS.

### English-Only Policy

- Code, comments, docs, walkthrough 184 and UI strings are English; no non-English characters in
  the unlock files. PASS.

## 4. Detailed Findings & Action Items

Re-review (second pass): the previous pass (fingerprint `b36cf6f1…`) raised two MINOR findings and
one SUGGESTION. A patch-to-patch diff of the frozen target confirms the delta consists solely of
these three text edits; no code path, test or other file changed.

- **[MINOR — RESOLVED]** `Docs/SECURITY_AND_QUALITY_GUIDELINES.md:120` now states that
  `soos-remote` calls `UnlockSession` only when `allow_unlock = true`, names it exactly once and
  never names `SetLockedHint` or a session-ending method (consistent with ADR 2026-10-06 and the
  amended RMC-S3).
- **[MINOR — RESOLVED]** `crates/remote/src/server.rs:645` — the `unlock_flow` doc comment now
  says every accepted unlock (one that reaches `unlock_session`) leaves one `info` audit line,
  matching the behaviour and the tests (comment-only change).
- **[SUGGESTION — APPLIED]** `Docs/REMOTE_COMPANION.md` — the "Unlock PC" shortcut now documents a
  mandatory *Choose from Menu* confirmation step before *Get Contents of URL*; the documentation
  needles checked by `test_rmc_unlock_is_opt_in_and_documented` are unaffected.

No open findings.

## 5. Final Verdict

**VERDICT: APPROVED**
