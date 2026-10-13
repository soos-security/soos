# Candid Review Report

- **Date**: 2026-10-13
- **Target Branch**: `feat/remote-offline-alerts`
- **Base (merge-base)**: `6f32ef8`
- **Reviewed-Diff-Fingerprint**: `caf54570623eb8f380cf412883eeebbdef827fa183443c8989afba24199ae8e9`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_remote_offline_alerts.md`, `AI/auditor_constraints_remote_offline_alerts.md`, `AI/tester_contract_remote_offline_alerts.md`, `AI/walkthroughs/202_remote_offline_alerts.md`, `Docs/REMOTE_COMPANION.md`, `crates/remote/assets/app.js`, `crates/remote/assets/style.css`, `crates/remote/assets/sw.js`, `crates/remote/src/audit.rs`, `crates/remote/src/config.rs`, `crates/remote/src/lib.rs`, `crates/remote/src/logind.rs`, `crates/remote/src/main.rs`, `crates/remote/src/offline.rs`, `crates/remote/src/push.rs`, `crates/remote/src/routes.rs`, `crates/remote/src/server.rs`, `crates/remote/tests/alerts_server_tests.rs`, `crates/remote/tests/android_push_tests.rs`, `crates/remote/tests/battery_server_tests.rs`, `crates/remote/tests/common/camera.rs`, `crates/remote/tests/common/harness.rs`, `crates/remote/tests/offline_server_tests.rs`, `crates/remote/tests/offline_tests.rs`, `crates/remote/tests/push_server_tests.rs`, `crates/remote/tests/server_tests.rs`, `scripts/install_remote.sh`, `tests/invariants/src/lib.rs`, `tests/invariants/src/remote_offline_contract.rs`

## 1. Executive Summary

GitHub #376 (tier 1 of #359 Part B) adds an opt-in offline history and a back-online Web Push to
`soos-remote`. The diff was reviewed against the 2026-10-13 ADR, RC-1, RMC-S9, the Web Push ADR,
`Docs/REMOTE_COMPANION.md` and the security guidelines. RC-1 holds: no inet socket and no unit
change. The bus stays read-only: `NameHasOwner` plus four `Properties.Get` reads, no match and no
mutating call. No network identifier is read. The store follows the passkey and push store rules.
The new route is never Funnel-public. Existing tests only gained the setup line. The full
workspace suite passes (`cargo test --locked --workspace --all-targets --all-features`: 3587
passed, 0 failed, 6 ignored that were already ignored before this change).
The first review (fingerprint `586b0661…`) found one MINOR documentation defect and one SUGGESTION. The MINOR one is now fixed: re-review of the current diff confirms that the only change since then is the `AI/DECISIONS.md` correction. The SUGGESTION remains open. Nothing is CRITICAL or MAJOR.

## 2. Test Changes

Mechanical listing (skill step 3):

- Files touched: `alerts_server_tests.rs`, `android_push_tests.rs`, `battery_server_tests.rs`,
  `common/camera.rs`, `common/harness.rs`, `push_server_tests.rs`, `server_tests.rs` — each gains
  exactly one line `offline: soos_remote::config::OfflineConfig::default(),` in a `RemoteConfig`
  struct literal (verified with `git diff`, no other `+`/`-` line). This is a setup-only migration
  forced by the new `RemoteConfig.offline` field and stated in ADR item (10). No assertion changed.
- `tests/invariants/src/lib.rs`: only adds `mod remote_offline_contract;`.
- New: `offline_tests.rs` (22 tests), `offline_server_tests.rs` (10 tests),
  `remote_offline_contract.rs` (ROA-S1..S5).
- Removed or changed assertions: none. The first review's only `^-` hit, `AI/DECISIONS.md:123`, is gone after the fix to finding 1. `AI/DECISIONS.md` is now append-only: one line added and none removed.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance): none in code; the only hit is a
  sentence in the auditor constraints document.
- Inline `mod tests` changes: none.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Tracker: I checked a sleep during an open outage. The outage is closed at the previous sample, then the
  `sleep` interval is recorded and never pushed (`pushes()` is false only for `Sleep`). An
  outage under 30 s is dropped. A `wifi_disabled` reading upgrades an open interval. A clock that
  steps backwards gives an empty wall span, never an inverted one (`OfflineInterval::new`). A
  `Running` marker with `now < last_alive` gives no interval. → PASS.
- Release: a held notice is released only on `Online` or after 12 consecutive `Unknown` steps.
  The streak resets on any real reading. A `shutdown_or_crash` from `start` is held until then. → PASS.
- Push ordering: `PushRuntime::start` runs before `OfflineRuntime::start`, and the sink checks
  `is_active()`. The watcher is spawned only when the store was usable. → PASS.
- Scheduler: `take_due` shares `budget_due` (30 s coalescing and spacing, 20/h). The older
  first-noted message wins and an alert wins a tie. Notice retries (5 s, 30 s, 2 min, 10 min,
  then dropped as `Failed`) bypass the budget and are merged per endpoint. Topic `soosonline` is
  checked distinct at compile time. → PASS.
- Config: `offline_alerts` without push gives `OfflineRequiresPush`. The `load_config` failure
  path returns `EXIT_CONFIG` (78), and `packaging/soos-remote.service` has
  `RestartPreventExitStatus=78`, so there is no restart loop. `with_offline_alerts` also ignores the wiring unless both
  flags are set. → PASS.
- `logind.rs` refactor into `connect_system_bus` keeps the single pinned `Builder::address(`,
  `method_timeout`, `max_queued` and connect timeout. → PASS.

### PAM Concurrency & Deadlines
- No PAM code touched. Remote deadlines: each NM observation runs under `NETWORK_READ_DEADLINE_MS`
  (3 s, which is less than the 5 s tick; a compile-time assert covers connect plus 4 calls). The stop step
  runs under `OFFLINE_STOP_DEADLINE_MS`, and the shutdown read under 1.5 s. Store lock retries are bounded
  by `STORE_LOCK_TIMEOUT_MS`. No `std::sync::Mutex` guard is held across an `.await`. I checked
  `step`, `start`, `persist` and `stop`. → PASS.

### Panic Safety & Fail-Closed
- No `unwrap`/`expect`/indexing in production paths. `spawn_blocking` join errors map to `Io`, and
  poisoned mutexes are skipped. A malformed or refused store at start sets `failed`. The watcher is
  then not spawned and `stop()` returns early because the runtime is not active, so the bad file is never rewritten (test
  `test_roa_store_failed_is_unavailable_and_never_rewritten`). The watcher task is supervised:
  a panic becomes `TaskPanicked`. This feature has no unlock/allow path. → PASS.

### Test Integrity & Anti-Weakening
- See §2. The new tests cover outcomes a plausible wrong implementation would get wrong:
  boottime-vs-wall duration, floor, notice merge, generic payload text, shared budget, Funnel 403,
  never-rewritten bad store, heartbeat rate and the "off installs nothing" case. → PASS.

### Memory, Bounds & Secrets
- History ≤ 32 (evicting); store ≤ 8 KiB enforced on read (`read_owned_file`) and write. A
  compile-time assert bounds 32 serialized intervals. Notice retries are at most one per known endpoint.
- The store reuses `read_owned_file`/`try_lock_file`/`check_parent_dir`/`write_atomic`
  (`O_NOFOLLOW`, owner and mode checks, `flock`, temp file, `fsync`, `rename`, `0600`).
- Privacy: I searched `offline.rs`, `push.rs` and `logind.rs` for `Ssid`, `HwAddress`, `Ip4Config`,
  `ActiveConnections`, `Devices`, `AccessPoint` and hostname. There are no hits. Only `State`, `Connectivity`,
  `WirelessEnabled` and `PreparingForShutdown` are read. No `MessageStream`, `SignalStream`,
  `AddMatch`, proxy or `Inhibit` is used, and there is no inet socket. Logs are three fixed audit lines, plus the `main.rs` path-refusal
  `warn!` (see SUGGESTION). Its `%err` prints only the byte limit, never the path.
- `push_previews = "generic"` gives "Update from your PC — Open soos for details" with
  `reason: null`. Times and counts stay in the encrypted `soos` member, as alerts already do. The page
  uses `setText` only, the CSP in `http.rs` is unchanged and `index.html` is unchanged.
- `/api/offline` is not in `is_funnel_public`, so a Funnel caller without a session gets `403 login_required`
  (`test_roa_view_over_funnel_requires_session`). It is a GET/HEAD read route and POST gives `405`. → PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml` or workflow change. `scripts/install_remote.sh` only adds a commented
  template key. → PASS.

### English-Only Policy
- All code, comments, docs and UI strings are in English. → PASS.

## 4. Detailed Findings & Action Items

- **[MINOR — RESOLVED]** `AI/DECISIONS.md:123` — the immutable 2026-10-06 ADR "Tailscale Funnel Access and
  In-House Passkey Authentication" was changed by accident: "RMC23–RMC25 amended" became
  "rMC23–RMC25 amended". Restore the original capital `R`. The ADR register is append-only. *Resolved*: the text is back to "RMC23–RMC25 amended", and `git diff origin/main -- AI/DECISIONS.md` removes no existing line.
- **[SUGGESTION]** `crates/remote/src/main.rs:233` — a refused network store path logs through
  `warn!` instead of the fixed audit vocabulary that ADR item (6) describes. It also leaves
  `/api/offline` at `disabled` rather than `unavailable`. This follows the push precedent
  (`main.rs:464`) and leaks nothing. Consider documenting it or reporting `unavailable`.

## 5. Final Verdict

**VERDICT: APPROVED**
