# Walkthrough 202 — Offline History and Back-Online Push in `soos-remote`

- **Date**: 2026-10-13
- **Issue**: GitHub #376 (tier 1 of #359 Part B; GitHub-only, no backlog id). **Branch**: `feat/remote-offline-alerts`.
  The branch is not registered in `scripts/sync_issue.py`. The commit carries `Closes #376` and `Refs #359`.
- **Matrix criteria**: ROA1–ROA14.

## 1. Context & Objectives

The owner wants the phone told when the PC goes offline (#359, 2026-10-12). The PC cannot report
its own disappearance, so tier 1 reports it after the fact. `soos-remote` records offline
intervals (`network_lost`, `wifi_disabled`, `sleep`, `shutdown_or_crash`). When the PC is back
online, it sends one fixed-vocabulary Web Push through the existing sender, and it shows an
"Offline history" card in the web app. The constraints were:

- RC-1 kept: no `AF_INET` socket;
- read-only bus: no NetworkManager write;
- no SSID, BSSID, IP, MAC or location anywhere;
- the failed-password alert budget shared;
- `push_previews = "generic"` honoured.

## 2. Architect Design

Spec: `AI/architect_spec_remote_offline_alerts.md`. Sections §15 and §16 are authoritative over
the earlier sections.

- **Polling instead of signal subscriptions (D-1).** The companion's immutable bus rules (RMC-S9)
  forbid `MessageStream`, `SignalStream`, `receive_signal` and proxies, and allow a single
  connection builder. Every 5 s the watcher calls the bus driver's `NameHasOwner`, which never
  activates a stopped NetworkManager. Then, only while NetworkManager is running, it reads three
  manager properties with `Properties.Get`. Sleep shows as the gap between `CLOCK_BOOTTIME` and
  `CLOCK_MONOTONIC`. A clean stop reads logind's `PreparingForShutdown` once and writes a stop
  marker. No match rule is ever installed.
- **Store.** `remote-network.json` follows the `remote-push.json` rules (`0600`, ≤ 8 KiB,
  `O_NOFOLLOW`, owner-checked, `flock` + temp + `fsync` + `rename`) and holds at most 32
  intervals. A refused file makes the feature `unavailable` and is never rewritten.
- **Default and dependency.** `offline_alerts` defaults to `false` (consistent with
  `push_notifications`). It requires `push_notifications = true`, otherwise exit 78 (no failure
  loop: `RestartPreventExitStatus=78`). The #362 setup will write both keys.
- **Shared budget.** The push scheduler gains a second pending slot that uses the same spacing
  and the same hourly deque. Notice retries (5 s, 30 s, 2 min, 10 min) bypass the budget.
- **Privacy.** The payload builder takes only the `Copy`, enum-and-integer `OfflineNotice`,
  `push_previews` and `rp_id`. The body gives a duration instead of clock times; the clock times
  stay in the encrypted member, and the page renders them in the phone's time zone.
- **ADR.** "Offline History and Back-Online Push Notifications in `soos-remote`" amends item (1)
  of the 2026-10-06 Web Push ADR.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`.

**Round 1: REVISION_REQUIRED**, with four MAJOR findings:
- F-1: `SystemTime` outside `main.rs`/`server.rs` breaks R3-1.
- F-2: the notice could be released before the reconnect and then lost.
- F-3: a read of a stopped-but-enabled NetworkManager would start it.
- F-4: the spec's degrade-on-missing-push contradicted the acceptance line.

The fixes:
- injected Unix clock;
- release only on `Online` (or after 12 unreadable steps), with bounded notice retries;
- `NameHasOwner` gate;
- exit 78.

**Round 2: APPROVED**, with MINOR items R2-1 to R2-6:
- R2-1: the owned save guard moves into the blocking task.
- R2-2: an ordering invariant for the gate.
- R2-3: the `Unknown` streak counts whatever NetworkManager state was observed before.
- R2-4: a reading-order banner at the top of the spec.
- R2-5: the rebase was refused by the harness's safety rule; it is left to the release step.
- R2-6: notice retries do not use the budget.

## 4. Tester Contract

`AI/tester_contract_remote_offline_alerts.md`.

- New tests: 23 pure tests in `crates/remote/tests/offline_tests.rs`, 10 served tests in
  `crates/remote/tests/offline_server_tests.rs` (fake bus `FakeNetwork`, fake clock `FakeClock`,
  `FakeTransport`), and 5 invariants in `tests/invariants/src/remote_offline_contract.rs`.
- Red: every new test failed against signature stubs, except ROA-S1, a regression guard that was
  already green on the base tree.
- Setup-only migration: seven `RemoteConfig` literals gained `offline: OfflineConfig::default()`.
- One Phase 2 expectation error in test 6 was corrected before Green. The test had forgotten the
  outage that the final `Online` step closes. The new expectation is the exact merged notice, so
  no assertion was weakened.

## 5. Auditor Constraints

`AI/auditor_constraints_remote_offline_alerts.md`: 38 constraints, CLEARED. How the key ones are
met:

- No panics, indexing or lossy casts. Poisoned mutexes are handled without panic.
- Every load and save runs on the blocking pool behind the async save gate, and the owned guard
  moves into the blocking closure (R2-1).
- The network read is bounded at 3000 ms, the shutdown read at 1500 ms and the whole stop at
  3000 ms. The stop runs only after a clean shutdown and after `abort_all()`.
- `OfflineInterval::new` clamps `end >= start`, so a backwards NTP step can never produce a store
  that the loader refuses (auditor finding 13).
- The sink is called after the tracker guard is dropped, and no std mutex is held across an
  `.await`.
- There is one bus builder (`logind::connect_system_bus`) and two call sites (`NameHasOwner`,
  `Properties.Get`).
- `offline.rs` and `push.rs` have no tracing. The only logs are three fixed audit lines.

## 6. Implementation

- `crates/remote/src/offline.rs` (new): types, classifier, pure tracker, store, the bus and clock
  seams, `ZbusNetworkSource`, `SystemOfflineClock`, the runtime and the view.
- `crates/remote/src/logind.rs`: `connect_system_bus()` extracted; it is still the only
  `Builder::address(`.
- `crates/remote/src/push.rs`:
  - shared scheduler slot (`note_offline`, `offline_due_at`, `take_due`, `DueMessage`);
  - `back_online_payload`;
  - notice dispatch with its own retry list;
  - `offline_sink`.
- `crates/remote/src/config.rs`: key `offline_alerts`, `OfflineConfig`,
  `ConfigError::OfflineRequiresPush`, `resolve_network_store_path`.
- `crates/remote/src/routes.rs`: `GET|HEAD /api/offline`.
- `crates/remote/src/server.rs`: `with_offline_alerts`, start after the push store, the supervised
  watcher, the bounded clean-stop step, the handler.
- `crates/remote/src/audit.rs`: `offline alerts active`, `offline alerts unavailable`,
  `offline history not persisted`.
- `crates/remote/src/main.rs`: wiring, with the wall clock injected from `main.rs`.
- `crates/remote/src/lib.rs`: constants and compile-time relations.
- Assets:
    - `app.js`: the "Offline history" card, built at run time without markup ids (the page id set
    is pinned by RMC-S50, so `index.html` is unchanged), `fetchOffline` and `renderOffline` with
    fixed strings through `setText`, the card hidden at sign-out;
  - `style.css`: one row rule;
  - `sw.js`: tag `soos-online`.
- `scripts/install_remote.sh`: commented template line.
- Docs: `Docs/REMOTE_COMPANION.md` §2j, §5, §6, §8 and §9; the ADR; matrix rows ROA1–ROA14;
  `AI/ARCHITECTURE.md`; the project-facts row.

## 7. Candid Review

Not run in this phase. The coordinator runs Phase 5 (`./scripts/candid_subagent.sh --prepare`,
fresh reviewer) and the release step.

## 8. Verification Results

All commands ran with a private target directory,
`CARGO_TARGET_DIR=$HOME/.cache/soos-targets/remote-offline-alerts`, `CARGO_INCREMENTAL=0` and
the dev and test debug profiles disabled. The shared target directory used during Phase 2 was
found unsafe, so every gate was re-run.

- `cargo fmt --all -- --check`: pass.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: pass. After
  the last edits, which touched only `soos-remote` and `soos-invariants`, it was re-run on those
  two packages.
- `cargo test --locked --workspace --all-targets --all-features --no-fail-fast -- --test-threads=4`
  (`CARGO_BUILD_JOBS=4`):
  - first full run: 3562 passed and 3 invariants failed; all three were fixed (tester contract,
    "Corrections found by the first full workspace run");
  - final re-run of `soos-remote` and `soos-invariants` (every package touched by those fixes):
    1015 passed, 0 failed;
  - the new suites: `offline_tests` 23/23, `offline_server_tests` 10/10, `remote_offline_contract`
    5/5.
- `./scripts/candid_review.sh` (layer 1): PASSED.

## 9. Known Limitations / Follow-ups

- **Owner hardware rows**:
  - ROA12: Wi-Fi off for more than a minute, then on → one back-online notification and a
    `wifi_disabled` row.
  - ROA13: suspend, then resume → a `sleep` row and no notification.
  - ROA14: hard power-off, then boot → a `shutdown_or_crash` row and one notification.
- **Accepted risks** (ADR item 8):
  - the notice comes after the fact;
  - outages under 30 s are missed;
  - a nightly shutdown gives a daily notification;
  - `sleep` rows can crowd the 32 slots;
  - a notice not yet sent at a stop is lost;
  - the history can be forged by an owner-uid process (bounded).
- **Later work**:
  - tier 2 watchdog and tier 3 Android heartbeat (each with its own ADR; tier 2 must amend RMC-S9
    for signals and a delay inhibitor);
  - `alert_on_sleep`;
  - a "network changed" push without SSID;
  - `boot_id` binding of the stop marker;
  - the #362 setup must write `offline_alerts = true` together with `push_notifications = true`,
    and turn it off with push.
- **Housekeeping**: the branch is one commit behind `origin/main` (`6f32ef8`, #373). The rebase is
  left to the release step. Expected conflicts are only the append-only lists
  (`tests/invariants/src/lib.rs`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`).
