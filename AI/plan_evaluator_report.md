# Plan Evaluation Report
- **Date**: 2026-10-13
- **Issue**: GitHub #376: Offline History and a Back-Online Push in `soos-remote` (tier 1 of #359 Part B; GitHub-only, no backlog id)
- **Branch**: `feat/remote-offline-alerts` (worktree HEAD `e86993a`, one commit behind `origin/main`; rebase deferred to the release step, see R2-5)
- **Base commit**: `6f32ef8`
- **Plan**: `AI/architect_spec_remote_offline_alerts.md` (round 2; §15 "Revision 2" is authoritative where it differs from §0–§14)
- **Owner decisions read**: #359 body (Part B tier 1 and its acceptance block), #376 body, #362 comments (2026-10-12, 2026-10-13)

## Round 2: resolution of the round-1 findings

| Finding | Round-1 severity | Revision | Status |
|---|---|---|---|
| F-1 (clock vs R3-1) | MAJOR | §15 F-1: `SystemOfflineClock::new(unix: UnixClock)`, wall time injected from `main.rs`; `offline.rs` reads only `CLOCK_BOOTTIME`/`CLOCK_MONOTONIC` through `nix::time::clock_gettime`, never `SystemTime` or `CLOCK_REALTIME`. Verified: `UnixClock = Arc<dyn Fn() -> u64 + Send + Sync>` (`server.rs:154`), already injected into `push.rs:1200` and `alerts.rs:821`; R3-1 needle is `SystemTime::now` outside `main.rs`/`server.rs` (`remote_companion_contract.rs:1342`) | **Resolved** |
| F-2 (premature release, lost notice) | MAJOR | Release only on `Online`; 12-tick (60 s) release only when NM was never observed in this run; notice retries `[5 s, 30 s, 120 s, 600 s]` (plus `Retry-After`, capped at `MAX_RETRY_AFTER_S` = 300 by `push-protocol/src/lib.rs:80`), at most 5 attempts over about 12.6 min. Scenarios (a) slow first read on an NM host: NM is observed as `Offline` → held until `Online`. (b) no NM: released after 60 s, then 12.6 min of retries cover a late network. (c) captive portal: still `Online` per D-4, but the retries now span 12.6 min instead of 35 s | **Resolved** (residual edge in R2-3) |
| F-3 (bus activation of a stopped NM) | MAJOR | D-15: `NameHasOwner("org.freedesktop.NetworkManager")` on the bus driver before the NM reads of each tick; `false` → `Unknown`, monitor `Unavailable`, no call to NM. The bus driver never activates a service. ROA-S2 allows exactly this one extra call (`call_method(` count 2). The literals do not collide with the RMC-S9/R3-2 needles (`remote_companion_contract.rs:1166–1188`: `::system(`, `request_name`, … are not touched by a `call_method` to `org.freedesktop.DBus`) | **Resolved** (test power of the gate in R2-2) |
| F-4 (exit 78 acceptance line) | MAJOR | D-9 now follows the acceptance line: `ConfigError::OfflineRequiresPush`, exit 78, checked after the push keys; no `effective`; `OfflineUnavailableReason` = `StoreFailed` only; test 17 asserts the refusal, test 23 becomes `test_roa_offline_without_push_is_inert`. `main.rs` already maps every `ConfigError` to `EXIT_CONFIG` (`main.rs:145–224`); the unit keeps `RestartPreventExitStatus=78`. No GUI writes `push_notifications` today (`crates/gui/src` has no hit), so no existing toggle can trigger the exit; the #362 requirement is recorded in the ADR | **Resolved** |
| F-5 (zero clock sample) | MINOR | `sample() -> Option<ClockSample>`; `None` skips the whole step, and the next valid sample is compared with the last valid one | **Resolved** |
| F-6 (budgets, stop race) | MINOR | `NETWORK_READ_DEADLINE_MS` = 3000 = `DBUS_CONNECT_TIMEOUT_MS` 1000 + 4 × `DBUS_CALL_TIMEOUT_MS` 500 (< tick 5000); `OFFLINE_SHUTDOWN_READ_MS` = 1500 ≥ 1000 + 500; the stop budget of 3000 leaves 1500 for a save including `STORE_LOCK_TIMEOUT_MS` 500; the stop step runs after `supervised.abort_all()` (`server.rs:845`) and before `serve` returns. Budgets and ordering are **resolved**; the save-serialization mechanism is not sound as worded (R2-1) | **Partly resolved** (R2-1, MINOR) |
| F-7 (dispatcher) | MINOR | `next_wake` includes `offline_due_at()` and notice retries; separate notice-retry list pruned in `after_load`; the oldest first note wins and the alert wins ties; the offline runtime starts after `push.start()`; the sink is called after the tracker guard drops | **Resolved** (budget accounting of notice retries to state, R2-6) |
| F-8 (needle scoping) | MINOR | property literals pinned in `offline.rs` only; comment-stripped production code; page check limited to `fetchOffline`/`renderOffline` | **Resolved** |
| F-9 (ADR departures, cost) | MINOR | the ADR names polling and the two read-only methods; 4 calls per 5 s ≈ 69 120/day, ≈ 1 440 writes/day | **Resolved** |
| F-10 (page lifecycle) | MINOR | `showLogin` hides `#offline` and clears the list; `renderOffline` returns early when `signedOut`; a non-OK answer is ignored | **Resolved** |
| F-11 (test power) | MINOR | test 6 rewritten (`Unknown` releases nothing once NM was observed; `Online` releases; 12 `Unknown` steps release on a never-observed host); new sleep-gap boundary, unmonitored-release, heartbeat-cadence and `NameHasOwner` tests; the missed-deadline stop is bounded by construction and recorded as an owner-hardware observation | **Resolved** except the `NameHasOwner` test, which cannot fail against a plausible wrong implementation (R2-2) |
| F-12 (housekeeping) | MINOR | rebase deferred to the release step; walkthrough 202 assigned by the coordinator (196–201 reserved); the ADR records the daily `shutdown_or_crash` push and the `sleep`-crowded history; `boot_id` deferred | **Recorded** (R2-5) |

All four round-1 MAJOR findings are resolved soundly. Six new or residual MINOR findings (R2-1 to R2-6) are listed in §4. None of them blocks Phase 2.

## 1. Coverage Matrix

| Acceptance line / TDD test | Spec element | Status |
|---|---|---|
| #359 B1-a: follows NM and logind signals read-only | D-1 (polling `Properties.Get` + BOOTTIME/MONOTONIC gap + `PreparingForShutdown` at SIGTERM), ADR departure note (§15 F-9) | Mapped, with a documented deviation forced by the immutable RMC-S9 |
| #359 B1-a: no method call on NetworkManager, no `Inhibit` | D-2, D-15 (`NameHasOwner` goes to the bus driver, not to NM), ROA-S2 | Mapped; the ADR states the read-only-method reading |
| #359 B1-a: static invariant, no `AF_INET`/`AF_INET6`, no NM mutating method | ROA-S1, ROA-S2 (tests 31, 32) | Mapped |
| #359 B1-b: bounded intervals (≤ 32), monotonic + wall time, reason enum | §5.1, §5.2, tests 1–8, 10, sleep-gap boundary | Mapped |
| #359 B1-b: `remote-network.json` 0600, ≤ 8 KiB, `O_NOFOLLOW`, owner-checked, flock + temp + fsync + rename | §5.3, tests 13–16, 27 | Mapped |
| #359 B1-b: unclean previous stop → `shutdown_or_crash`, fake clock and fake bus | D-7, §15 F-6, tests 9, 25, 26 | Mapped (stop-save race in R2-1) |
| #359 B1-c: fixed vocabulary; the builder takes only enums and times | §5.6, ROA-S3 struct scan, tests 12, 19, 33 | Mapped |
| #359 B1-c: `push_previews = "generic"` honoured | §5.6, tests 19, 24 | Mapped |
| #359 B1-c: coalescing and hourly limit shared | D-10, §5.7, §15 F-7, tests 20, 21, 30 | Mapped (R2-6 clarification) |
| #359 B1-c: on reconnect, one Web Push goes out | §15 F-2 (release on `Online`, unmonitored release, bounded retries), tests 6, 24, 25, `test_roa_tracker_unmonitored_release` | Mapped |
| #359 B1-d: opt-in `offline_alerts`, default `false` | D-8, test 17 | Mapped |
| #359 B1-d: requires `push_notifications = true`, exit 78 otherwise | D-9 (revised), §15 F-4, test 17 | Mapped |
| #359 B1-d: while off, no bus match is installed and no file is created | D-1 (no match ever), tests 22, 23 (`..._inert`) | Mapped |
| Web app "Offline history" card | §8, §15 F-10, ROA-S4, test 34 | Mapped |
| Privacy: no SSID/BSSID/IP/location | D-13, ROA-S3, tests 24, 33 | Mapped |
| ADR amending Web Push ADR item (1) | §12, §15 F-9 | Mapped |
| Owner hardware checks | ROA12–ROA14 pending, plus the missed-deadline stop observation | Mapped (owner rows) |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `UnixClock` is an injected closure | `crates/remote/src/server.rs:154` | `Arc<dyn Fn() -> u64 + Send + Sync>` | Yes |
| R3-1: `SystemTime::now` only in `main.rs`/`server.rs` | `tests/invariants/src/remote_companion_contract.rs:1342` | as stated | Yes; `offline.rs` complies per §15 F-1 |
| RMC-S9 / R3-2 needles | `remote_companion_contract.rs:1166–1188` | `Connection::system`, `::system(`, `request_name`, `MessageStream`, `SignalStream`, `#[proxy`, `zbus::blocking`, `Address::system`, … | A `call_method` to `org.freedesktop.DBus`/`NameHasOwner` hits none of them |
| Single `Builder::address(` | `remote_companion_contract.rs:1154` | count 1 | Kept (moved into `logind::connect_system_bus`) |
| `DBUS_CONNECT_TIMEOUT_MS` / `DBUS_CALL_TIMEOUT_MS` | `crates/remote/src/lib.rs:110`, `:108` | 1000 / 500 | Yes. 1000 + 4 × 500 = 3000 = `NETWORK_READ_DEADLINE_MS` (≤ holds with equality), and 1000 + 500 ≤ 1500 |
| `STORE_LOCK_TIMEOUT_MS` | `lib.rs:259` | 500 | Fits the 1500 ms save sub-budget |
| `supervised.abort_all()`, then join, then connections | `server.rs:845–851` | as stated | Yes; the stop step slots in before `serve` returns |
| `RestartPreventExitStatus=78`, `RestrictAddressFamilies=AF_UNIX` | `packaging/soos-remote.service` | present; no `TimeoutStopSec` (systemd default 90 s) | The 3000 ms stop step plus `RUNTIME_SHUTDOWN_TIMEOUT_MS` 1000 fit |
| Every `ConfigError` → `EXIT_CONFIG` | `crates/remote/src/main.rs:145–224` | 78 | Yes |
| `Retry-After` bound | `crates/push-protocol/src/lib.rs:80, 472` | `MAX_RETRY_AFTER_S` = 300, 0 refused | Notice retries stay bounded |
| Alert retries: `retry_or_fail` replaces the retry per endpoint and folds a carry after the last delay | `crates/remote/src/push.rs:1676–1700` | as stated | The separate notice-retry list of §15 F-7 is required and specified |
| No GUI writes `push_notifications` | `crates/gui/src` | 0 hits | Exit 78 cannot be triggered by an existing toggle |
| `PUSH_RETRY_DELAYS_MS` vs `OFFLINE_RETRY_DELAYS_MS` | `lib.rs:413`; §15 | `[5000, 30000]` vs `[5000, 30000, 120000, 600000]` | Sum 755 s ≈ 12.6 min, as stated |
| Upstream overlap | `git diff --name-only HEAD origin/main` | `tests/invariants/src/lib.rs`, plus `AI/DECISIONS.md` and `AI/VERIFICATION_MATRIX.md` (which Phase 6 appends to here) | Today only `lib.rs` overlaps; Phase 6 will add two more (R2-5) |

## 3. Pillar Analysis

### Pillar 1: Architecture and threat model
- Failure scenario considered: NetworkManager is enabled but was stopped by the administrator. With the `NameHasOwner` gate, the bus driver answers `false` and no message is addressed to `org.freedesktop.NetworkManager`, so bus activation never happens. `NameHasOwner` itself is answered by `dbus-daemon`/`dbus-broker` and never activates a service.
- Failure scenario 2: the gate is implemented but its answer is ignored, or the reads run before it. The service is then started again by bus activation, and every listed test still passes (R2-2).
- RC-1 unchanged (the system bus is `AF_UNIX`, the unit is unchanged); RMC-S9 unchanged.
- Result: PASS, with FINDING R2-2 (MINOR, test power).

### Pillar 2: Deadlines and concurrency
- No PAM, daemon or IPC change; N/A for `crates/pam`.
- Budget arithmetic re-checked:
  - observation: 1000 + 4 × 500 = 3000 ≤ 3000 < 5000;
  - shutdown read: 1000 + 500 ≤ 1500;
  - save: 1500 ≥ 500 (lock) + write;
  - whole stop: 3000, then the existing 1000 ms runtime bound, under the default 90 s stop timeout.
  
  PASS.
- Failure scenario: a heartbeat save is in flight on the blocking pool when SIGTERM arrives.
  - `abort_all()` cancels the watcher at its `spawn_blocking` await. That drops the future, and with it the `tokio::sync::MutexGuard` held "across the blocking-pool await". The blocking closure keeps running.
  - The stop step then acquires the now-free mutex while the heartbeat closure may still be in its `Locked` retry.
  - If that retry loop is inside the closure, the heartbeat can write `run: running` after the stop wrote `stopped`, and the next start pushes a false `shutdown_or_crash`.
  
  The mutex as worded does not close the race it was introduced for → R2-1 (MINOR; fail-safe direction, rare).
- Result: FINDING R2-1 (MINOR).

### Pillar 3: Panic safety and fail-closed
- Failure scenario: `clock_gettime(CLOCK_BOOTTIME)` fails once. `sample()` returns `None`, the step is skipped, and the next valid sample is compared with the last valid one, so no spurious `Sleep` is computed. PASS.
- Failure scenario: `offline_alerts = true` with `push_notifications = false` exits 78 and is not restarted. There is no partial wiring (`with_offline_alerts` also refuses defensively). PASS.
- Result: PASS.

### Pillar 4: Dependencies
- No new crate; `nix` `time` is already enabled. `CLOCK_BOOTTIME` in the Linux-only `soos-remote` raises no portability issue with the upstream `portable_crates_contract.rs`. PASS.
- Result: PASS.

### Pillar 5: Data confidentiality
- Failure scenario: the gate adds a bus-driver call whose argument is a bus name; no SSID, address or identifier is read. The read set stays three scalar NM properties plus one logind boolean, and `OfflineNotice` stays `Copy` with enum and integer fields. The page lifecycle change (F-10) closes the shared-phone leak. PASS.
- Result: PASS.

### Pillar 6: Test integrity
- Failure scenario 1: the revised test 6 fails against "release on `Unknown`" and against a counter that ignores whether NM was observed. PASS.
- Failure scenario 2: `nm_reads_allowed(has_owner: bool) -> bool` is the identity function, so its test cannot fail against an `observe` that never calls it (R2-2).
- Failure scenario 3: Red-phase files and stubs already exist in the worktree and are being aligned with §15:
  - test files: `crates/remote/tests/offline_tests.rs`, `offline_server_tests.rs`, `tests/invariants/src/remote_offline_contract.rs`;
  - stubs: `crates/remote/src/offline.rs`, the constants in `lib.rs` (still 1500/1000), `OfflineConfig::effective` in `config.rs:106`.

  Because they predate this gate's approval, they are not yet immutable contracts, so aligning them is not test weakening, but the walkthrough must say so. The spec body (§3, §4, §5.4, §5.5, §6, §8, §10) also still states the round-1 text (R2-4).
- Result: FINDING R2-2, R2-4 (MINOR).

## 4. Findings

- **R2-1 [MINOR]**: The save-serialization mutex (§15 F-6) does not survive a task abort. A `tokio::sync::MutexGuard` held across a `spawn_blocking(...).await` is released when `abort_all()` drops the watcher future, while the blocking closure keeps running. **Required plan change**:
  - Take the guard with `Arc<tokio::sync::Mutex<()>>::lock_owned()` and move the `OwnedMutexGuard` **into** the `spawn_blocking` closure, so it is released only when the write really ends. Alternatively, use a `std::sync::Mutex` acquired inside the closure by every save, the stop save included.
  - Keep the `Locked` retry loop outside the closure (an async sleep between attempts), so an aborted heartbeat cannot retry after the stop.

- **R2-2 [MINOR]**: The `NameHasOwner` gate has no test that fails against a plausible wrong implementation (the gate is called but ignored, or called after the reads). `nm_reads_allowed` is an identity function, and ROA-S2 only counts `call_method(` sites. **Required plan change**, one of:
  - (a) make ROA-S2 assert, in the `function_text` of the production `observe`, that `nm_reads_allowed(` and `"NameHasOwner"` occur before the first `"State"` literal and that the `false` branch returns before any `Properties.Get`;
  - (b) split `ZbusNetworkSource` over a small raw-call seam (`name_has_owner()`, `get_u32`/`get_bool`) with a fake that counts NM `Get` calls, and assert zero `Get` calls when the name has no owner.

- **R2-3 [MINOR]**: The unmonitored release keys on "NetworkManager never observed during this run". If NM is observed and then stopped mid-run (a switch to iwd), a held notice is never released and is lost at the next stop. **Change**: count consecutive steps where `NameHasOwner` returned `false` (NM absent now) instead of "never observed", or record the case in the §13 accepted risks.

- **R2-4 [MINOR]**: The spec body still has stale text that contradicts §15:
  - §3 table: 1500/1000, and the new constants are missing;
  - §4: `effective`, and "no new `ConfigError` variant";
  - §5.4: `SystemTime`, and `0` on a failed read;
  - §5.5 / §6: `PushDisabled`, `push_disabled()`;
  - §8: the "turn on push notifications" string;
  - §10: the descriptions of tests 6, 17 and 23;
  - §1.1: the "D-9 warning" in `main.rs`.

  §15 is declared authoritative, so nothing is ambiguous, but the tester and the developer read the body first. **Change**: fold §15 into the body, or strike the superseded sentences, before the Red files are frozen. The walkthrough records that the pre-gate Red files and stubs were aligned with round 2 before this gate approved them (no contract weakening).

- **R2-5 [MINOR]**: Housekeeping. The rebase onto `6f32ef8` is deferred to the release step (recorded, not a blocker). `origin/main` also changed `AI/DECISIONS.md` and `AI/VERIFICATION_MATRIX.md`, which Phase 6 appends to, so rebase **before Phase 6** to avoid append conflicts. Walkthrough 202 is assigned by the coordinator.

- **R2-6 [MINOR]**: §15 F-7 does not say whether notice retries draw from the shared 30 s / 20-per-hour budget. Existing alert retries do not, because only `take` records a send. **Change**: state "retries do not consume the shared budget (parity with alert retries)" and pin it in test 20 or test 30.

## 5. Verdict

All four round-1 MAJOR findings are resolved soundly:
- **F-1**: the wall clock is injected, so R3-1 holds.
- **F-2**: the notice is released only on `Online`, with a bounded unmonitored release and bounded retries.
- **F-3**: the `NameHasOwner` gate removes bus activation.
- **F-4**: exit 78, per the acceptance line.

The round-1 MINOR findings are resolved or recorded. The revision introduces no CRITICAL or MAJOR defect, and R2-1 to R2-6 are MINOR. R2-1 and R2-2 should be folded into the spec and the Red tests during Phase 2, and the auditor-agent should carry R2-1 as a constraint.

VALIDATION_VERDICT: APPROVED
