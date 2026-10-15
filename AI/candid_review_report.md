# Candid Review Report

- **Date**: 2026-10-15
- **Target Branch**: `feat/remote-offline-alerts`
- **Base (merge-base)**: `af511eb`
- **Reviewed-Diff-Fingerprint**: `396018e73d046acb3b0303804de69c555bcfe89f226666b2fc3760ce960fa2d6`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_remote_offline_alerts.md`, `AI/auditor_constraints_remote_offline_alerts.md`, `AI/tester_contract_remote_offline_alerts.md`, `AI/walkthroughs/202_remote_offline_alerts.md`, `Docs/REMOTE_COMPANION.md`, `crates/remote/assets/app.js`, `crates/remote/assets/style.css`, `crates/remote/assets/sw.js`, `crates/remote/src/audit.rs`, `crates/remote/src/config.rs`, `crates/remote/src/lib.rs`, `crates/remote/src/logind.rs`, `crates/remote/src/main.rs`, `crates/remote/src/offline.rs`, `crates/remote/src/push.rs`, `crates/remote/src/routes.rs`, `crates/remote/src/server.rs`, `crates/remote/tests/alerts_server_tests.rs`, `crates/remote/tests/android_push_tests.rs`, `crates/remote/tests/battery_server_tests.rs`, `crates/remote/tests/common/camera.rs`, `crates/remote/tests/common/harness.rs`, `crates/remote/tests/offline_server_tests.rs`, `crates/remote/tests/offline_tests.rs`, `crates/remote/tests/push_server_tests.rs`, `crates/remote/tests/server_tests.rs`, `scripts/install_remote.sh`, `tests/invariants/src/lib.rs`, `tests/invariants/src/remote_offline_contract.rs`

## 1. Executive Summary

Re-review after the rebase of `feat/remote-offline-alerts` (GitHub #376, tier 1 of #359 Part B,
PR #386) from base `94a7cb3` onto `origin/main` `af511eb`. Main gained #383 (push delivery test
deflake: `settle_gone` in `crates/remote/tests/common/push.rs`, `push_server_tests.rs`,
`android_push_tests.rs`), #394 (actions bump), #384 (motion ADR, `mod ui_motion_spec_contract`)
and #388 (read-only lock-screen state request: daemon, protocol `RequestKind::LockState`,
`mod lock_state_contract`). The previous review (fingerprint `caf54570…`, APPROVED, one MINOR
resolved, one SUGGESTION open) is carried forward. The raw diff against the new merge base was
re-read independently (`offline.rs` runtime/watcher/stop/persist, `push.rs` scheduler,
notice delivery and retries, `server.rs` wiring and route, `config.rs`, `logind.rs`, `main.rs`,
`audit.rs`). No CRITICAL or MAJOR finding.

Rebase verification (this review):
- `git range-diff origin/feat/remote-offline-alerts...HEAD`: the single branch commit `b2dbc3e` →
  `290b8c1`; the only changes are context lines from main and the placement of the conflicted
  blocks.
- Per-file comparison of the sorted added/removed lines of `94a7cb3..origin/feat/remote-offline-alerts`
  against `af511eb..HEAD`, for all 35 files: identical for 32 files, including `AI/DECISIONS.md`
  and `tests/invariants/src/lib.rs`. Differences: `AI/VERIFICATION_MATRIX.md` gains exactly one
  blank line and one `---` separator before the `remote-offline-alerts` section (the file's
  convention); `AI/candid_review_report.md` (this file, rewritten) and
  `AI/plan_evaluator_report.md` differ only through their base side (main's versions are now the
  base); the branch's `AI/plan_evaluator_report.md` is byte-identical to the pre-rebase branch
  file (`git diff origin/feat/remote-offline-alerts HEAD -- AI/plan_evaluator_report.md` is empty).
- No loss or duplication: `AI/DECISIONS.md` holds the "Motion System", "Read-Only Lock-Screen State
  Channel" and "Offline History and Back-Online Push Notifications" ADRs once each, main's first;
  `git diff af511eb HEAD -- AI/DECISIONS.md` removes no line. `tests/invariants/src/lib.rs`
  declares `mod remote_offline_contract;` once, after `mod ui_motion_spec_contract;` and
  `mod lock_state_contract;` (76 `mod` lines = 75 at `af511eb` + 1). The matrix holds one
  `remote-offline-alerts` section after main's `rwp-delivery-deflake`, `ui-motion-spec` and
  `lockscreen-status` sections. Walkthrough 202 does not collide with main's 203, 204, 205.
- Semantic interaction: #388 does not touch `crates/remote`; `soos-remote` only constructs
  `RequestKind::PreviewFrame` and has no exhaustive match on `RequestKind`, so the new wire kind
  needs nothing here. #383 adds `Call.answer`, `last_gone` and `settle_gone` to the push test
  harness; this branch's harness edits are the setup-only `offline: OfflineConfig::default()`
  lines, and its new back-online delivery path reuses the same `remove_gone` store rewrite that
  `settle_gone` waits for, so the deflake applies to it unchanged. Both suites pass together.
- `cargo test --locked -p soos-invariants -p soos-remote -- --test-threads=4`: 1044 passed,
  0 failed, 0 ignored (soos-invariants 625; soos-remote 419, including `offline_tests` 23,
  `offline_server_tests` 10, `push_server_tests` 22, `android_push_tests` 5).

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Over `target/candid_diff.patch`:
- Test files touched: `alerts_server_tests.rs`, `android_push_tests.rs`, `battery_server_tests.rs`,
  `common/camera.rs`, `common/harness.rs`, `push_server_tests.rs`, `server_tests.rs` — each gains
  exactly one line `offline: soos_remote::config::OfflineConfig::default(),` in a `RemoteConfig`
  literal and nothing else (verified per file with `git diff af511eb HEAD`). Setup-only migration
  forced by the new `RemoteConfig.offline` field (ADR item 10). No assertion changed.
- `tests/invariants/src/lib.rs`: only adds `mod remote_offline_contract;`.
- New: `offline_tests.rs` (23 tests), `offline_server_tests.rs` (10 tests),
  `remote_offline_contract.rs` (ROA-S1..S5).
- Removed or changed assertions (`^-` with `assert`/`#[test]`/`proptest!`/`should_panic`): none.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): none in
  code; the only hit is a sentence in `AI/auditor_constraints_remote_offline_alerts.md` (#38)
  forbidding them.
- Inline `mod tests` changes: none.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Tracker (carried forward, re-read): sleep during an open outage closes the outage at the
  previous sample and records a non-pushed `sleep`; outages under 30 s are dropped; a backwards
  wall clock gives an empty span, never inverted; `now < last_alive` gives no interval. → PASS.
- Release: held notice released on `Online` or after 12 consecutive `Unknown` steps. → PASS.
- Ordering: `PushRuntime` exists before `OfflineRuntime::start`; the sink drops notices unless
  `is_active()`; the watcher is spawned only when the store was usable; the sink is invoked after
  the tracker guard is released (no nested lock with the scheduler). → PASS.
- Scheduler: `take_due` shares `budget_due` (coalescing, spacing, 20/h); earlier first-noted wins,
  alert wins ties; notice retries (5 s, 30 s, 2 min, 10 min, then `Failed`) are per endpoint,
  merged on a new notice, pruned with `known`, and included in `next_wake`. → PASS.
- Config: `offline_alerts` without push → `OfflineRequiresPush` (exit 78, no restart loop);
  `with_offline_alerts` ignores the wiring unless both flags are set. → PASS.
- `connect_system_bus` keeps the single pinned `Builder::address(`, `method_timeout`,
  `max_queued`, connect timeout. → PASS.
- Rebase: no main API used by this branch changed (§1). → PASS.

### PAM Concurrency & Deadlines
- No PAM code touched. Remote bounds: observation under `NETWORK_READ_DEADLINE_MS` (3 s < 5 s
  tick, compile-time assert covers connect + 4 calls); stop under `OFFLINE_STOP_DEADLINE_MS`, its
  shutdown read under 1.5 s; store lock retries bounded by `STORE_LOCK_TIMEOUT_MS`; no
  `std::sync::Mutex` guard across `.await` (`step`, `start`, `persist`, `stop`, sink). → PASS.

### Panic Safety & Fail-Closed
- No `unwrap`/`expect`/`panic!`/indexing in the new production paths (`offline.rs` grep clean;
  `main.rs` `unwrap_or(u64::MAX)` only). `spawn_blocking` join errors map to `Io`; poisoned
  mutexes are skipped. A refused store sets `failed`, the watcher is not spawned and `stop()`
  returns early, so the bad file is never rewritten. The watcher is supervised (panic →
  `TaskPanicked`). This feature has no unlock/allow path. → PASS.

### Test Integrity & Anti-Weakening
- See §2. New tests discriminate plausible wrong implementations (boottime-vs-wall duration,
  floor, notice merge, generic payload, shared budget, Funnel 403, never-rewritten bad store,
  heartbeat rate, "off installs nothing"). → PASS.

### Memory, Bounds & Secrets
- History ≤ 32 (evicting); store ≤ 8 KiB on read and write, compile-time assert on 32 serialized
  intervals; notice retries ≤ one per known endpoint. Store uses `read_owned_file`,
  `try_lock_file`, `check_parent_dir`, `write_atomic` (`O_NOFOLLOW`, owner/mode checks, `flock`,
  temp + `fsync` + `rename`, `0600`).
- Privacy: only `State`, `Connectivity`, `WirelessEnabled` (NetworkManager), `NameHasOwner`
  and `PreparingForShutdown` (logind) are read; no SSID, hardware address, IP config, active
  connection, access point or hostname; no signal match, proxy or `Inhibit`; no inet socket.
  Audit lines are fixed vocabulary; `generic` previews carry no reason text. `/api/offline` is
  not Funnel-public (403 `login_required`), GET/HEAD only. → PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, workflow or hook change; `scripts/install_remote.sh` adds a commented
  template key only. → PASS.

### English-Only Policy
- All code, comments, docs and UI strings are English. → PASS.

## 4. Detailed Findings & Action Items

- **[MINOR — RESOLVED, carried forward]** `AI/DECISIONS.md` — the earlier accidental edit to the
  2026-10-06 ADR ("rMC23") stays fixed; `git diff af511eb HEAD -- AI/DECISIONS.md` removes no line.
- **[SUGGESTION — open, carried forward]** `crates/remote/src/main.rs` (`offline_wiring`) — a
  refused network store path logs through `warn!` instead of the fixed audit vocabulary of ADR
  item (6), and leaves `/api/offline` at `disabled` rather than `unavailable`. Follows the push
  precedent and leaks nothing (`%err` prints only the byte limit). Consider documenting it or
  reporting `unavailable`.

## 5. Final Verdict

**VERDICT: APPROVED**
