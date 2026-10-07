# Walkthrough 192 — Live PC Battery Level in `soos-remote`

- **Date**: 2026-10-07
- **Issue**: GitHub #346 (GitHub-only, no backlog id, like #339 and #345; not registered in `BRANCH_TO_ISSUE` of
  `scripts/sync_issue.py`, the commit says `Refs #346` and the PR body says `Closes #346`). **Branch**:
  `feat/remote-battery-status`, stacked on `origin/feat/remote-live-camera` (#345, PR #347, `df4798e`). Nothing is
  pushed, deployed, installed or restarted by the agents.
- **ADR**: "[2026-10-07] Live Battery Level in `soos-remote`" (`AI/DECISIONS.md`).
- **Documents**: spec `AI/architect_spec_remote_battery.md` (round 3); plan evaluation `AI/plan_evaluator_report.md`
  (round 3); tester contract `AI/tester_contract_remote_battery.md`; auditor constraints
  `AI/auditor_constraints_remote_battery.md` (C-1–C-34, CLEARED); candid review `AI/candid_review_report.md`.
- **Matrix criteria**: RBS1–RBS12 (RBS12 is the owner's hardware check, pending).

## 1. Context & Objectives

The owner wants to see the PC's battery level and charging state, live, in the `soos-remote` home-screen web app on
the iPhone. The companion is a user-level service that already runs as the owner, and the kernel power-supply class
attributes under `/sys/class/power_supply` are world-readable, so the companion reads them itself: no daemon, D-Bus,
UPower or dependency change.

Goals: on by default with a total off switch, read-only and bounded sysfs access that a hung ACPI/EC driver cannot
turn into a stalled service, fail closed (never a guessed level, never a false "No battery"), no identity (no supply
names, serial, model or manufacturer), no logging of values, visibility exactly equal to `/api/status`, and
`/api/status` itself unchanged.

## 2. Architect Design

- **Decisions B-1..B-14** (spec §0). Key ones: B-2 a separate `BatteryView`, `GET|HEAD /api/battery` and
  `event: battery` while `StatusView`, `/api/status` and `event: status` stay byte-for-byte unchanged; B-5 the route
  is not Funnel-public and a Funnel session is re-validated before each battery event; B-6 one sampler every 5 s only
  while a stream is open, a 500 ms read deadline, at most one read in flight and single-flight for every caller; B-9
  single battery = firmware `capacity`, several = energy-weighted, else floor mean, else unknown; B-10 `no_battery`
  only after a complete readable scan; B-11 the page takes data only from `event: battery`; B-12 `online` 0/1/2 per
  the kernel ABI; B-13 a bounded runtime shutdown in `main.rs`; B-14 one commented installer line.
- **Constants** (`crates/remote/src/lib.rs`): `POWER_SUPPLY_ROOT = "/sys/class/power_supply"`,
  `MAX_POWER_SUPPLIES = 64`, `MAX_BATTERIES = 8`, `MAX_POWER_SUPPLY_NAME_LEN = 64`, `MAX_SYSFS_VALUE_BYTES = 32`,
  `MAX_SYSFS_MICRO_DIGITS = 19`, `BATTERY_READ_TIMEOUT_MS = 500`, `BATTERY_SAMPLE_INTERVAL_MS = 5000`,
  `RUNTIME_SHUTDOWN_TIMEOUT_MS = 1000`, with compile-time asserts on their ordering.
- **Module** `crates/remote/src/battery.rs`: pure parsers (`parse_capacity`, `parse_status`, `parse_online`,
  `parse_present`, `parse_scope_included`, `parse_micro`, `valid_supply_name`), pure `aggregate`, blocking
  `read_power_supplies(root)`, the `BatterySource` trait with `SysfsBattery::{new, kernel}` and the async
  `BatteryRuntime` (read gate, cache, `in_flight` flag, `watch` channel, stream counter, sampler).
- **Config**: `BatteryConfig { enabled }` (`battery_status`, default `true`) in `RemoteConfig.battery`.
- **New invariants** RBS-I1..RBS-I9 (spec §7): visibility, no identity, bounded reads, fail closed, no logging,
  read-only and no new dependency, status unchanged, total off switch, page hygiene.

## 3. Plan Evaluation

Three rounds. Round 1 and round 2 were `REVISION_REQUIRED`: round 2 found a CRITICAL clash of the constructor name
`SysfsBattery::system()` with the RMC-S9 needle `::system(` (renamed `kernel()`, R2-1) and a MAJOR contradiction
between the cache boundary and test 28 (inclusive `MaxAge` rule, R2-2). Round 3 was `APPROVED` with three MINOR
clarifications folded into the tester contract and the auditor constraints: R3-1 the `"/sys` needle keeps its opening
quote, R3-2 test 33 strips comments, R3-3 the checked deadline and counter patterns.

## 4. Tester Contract

- 39 spec tests in 40 functions: `crates/remote/tests/battery_tests.rs` (18, pure and tempdir sysfs, one proptest of
  512 cases split in two functions), `battery_config_tests.rs` (1), `battery_routes_tests.rs` (1),
  `battery_server_tests.rs` (13, paused virtual time, scripted and gated sources) and
  `tests/invariants/src/remote_battery_contract.rs` (7 static tests). Shared fixture
  `crates/remote/tests/common/battery.rs` (`FakeSysfs`, `ScriptedBattery`, `Tracked`/`Probe`).
- Mapping to rows: RBS1 tests 18, 22; RBS2 1, 2, 17; RBS3 3, 7, 12; RBS4 4, 5, 6, 10, 13, 15; RBS5 8, 9, 16; RBS6
  10, 11, 25, 26, 32, 38, 39; RBS7 14, 31, 32, 33; RBS8 19, 20, 21; RBS9 23, 24, 27, 28, 29, 38; RBS10 30; RBS11 34,
  35, 36, 37 plus the existing `test_rmc_s8_*` and `test_rmc_s45`–`s50`.
- Red evidence: 33 behavioural functions failed to compile on the missing API only (E0432/E0433/E0599/E0560/E0609);
  the 7 static tests failed on their assertions (519 passed, 7 failed in the invariant suite).
- Power check: a scratch reference implementation killed ten listed mutants (inclusive cache boundary, missing
  `in_flight`, sampler ignoring the stream count, no cache re-check under the gate, invalid name not marking the scan
  incomplete, single battery preferring energy, missing `O_NONBLOCK`, `online = 2` unknown, no Funnel re-validation,
  no change filter). Ten plus five (two-core) runs of the server and pure suites were green.
- Contract migrations (setup only, no assertion touched): M1–M4 add `battery: BatteryConfig::default()` to the
  `RemoteConfig` literals of `common/harness.rs`, `server_tests.rs`, `alerts_server_tests.rs` and
  `push_server_tests.rs`; M5 adds the same line to `common/camera.rs` after stacking on #345.
- Test 37 (`test_rbs_s6_no_new_dependency`) pins the post-#345 dependency set and was legitimately Red on
  `05001c9`; it is green now that the branch is stacked on `origin/feat/remote-live-camera`.

## 5. Auditor Constraints

CLEARED with C-1–C-34. How the main ones are met:

- C-1, C-3, C-9, C-10 (panics, arithmetic): no `unwrap`/`expect`/panic macro or indexing in production code;
  digit accumulation and energy sums use `checked_*`, conversions use `try_from`; clippy `-D warnings` clean.
- C-2, C-11, C-12, C-13 (bounded reads): `read_attr` opens with `O_NOFOLLOW | O_NONBLOCK`, requires a regular file
  by `fstat`, reads through `take(MAX_SYSFS_VALUE_BYTES + 1)`; the directory walk stops at the 65th entry.
- C-5, C-7, C-15–C-19 (runtime): one checked deadline covers the gate wait and the join; `in_flight` is cleared by a
  guard moved into the blocking closure; a gate timeout is never cached or published; the sampler parks on `Notify`
  with `enable()` and a re-check.
- C-20, C-22, C-23, C-24 (server): `with_battery` stores the source only when enabled; `subscribe()` then
  `current()` with no `.await` between them; the battery arm re-validates the session with `Touch::Keep`;
  `Route::Battery` is outside `is_funnel_public`.
- C-21 (`main.rs`): `block_on`, then `shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT_MS)`.
- C-25–C-29 (confidentiality and page): four-key view, no tracing in `battery.rs`, fixed-text server lines, a single
  `"/sys` literal, `textContent` only on the page.
- C-30 (no dependency on this branch): no `Cargo.toml`/`Cargo.lock` change; test 37 turned green only by stacking on
  #345, never by adding a dependency.
- C-31 (test integrity): only M1–M5 touch pre-existing tests. C-32: no test reads the real `/sys`.
- C-33, C-34: no `unsafe` keyword in new files; docs state the bounds and the shutdown side effect.

## 6. Implementation

- New: `crates/remote/src/battery.rs`.
- Changed: `crates/remote/src/{lib,config,routes,http,server,main}.rs` (constants, `BatteryConfig`, `Route::Battery`
  and `BATTERY_PATH`, the SSE battery encoder, `ServerState::with_battery`, the `/api/battery` handler, the battery
  arm of `serve_stream`, the supervised sampler, the bounded runtime shutdown), `crates/remote/assets/app.js` (the
  run-time battery line with `batteryText`, `renderBattery`, `showBattery`, `clearBattery`),
  `scripts/install_remote.sh` (commented `# battery_status = true`).
- Docs: `Docs/REMOTE_COMPANION.md` §2g (and the config and route tables), `AI/ARCHITECTURE.md` §13,
  `AI/MOCK_STRATEGY.md` "Remote Companion Doubles", the ADR in `AI/DECISIONS.md`, `project-facts.md`.
- Notable decision: the branch was stacked on `origin/feat/remote-live-camera` instead of waiting for #345 to merge,
  so test 37 and the §12 conflicts (`server.rs`, `app.js`, `lib.rs`, docs) were resolved against the camera tree.
  The owner merges #347 first; this branch is then rebased onto `main` with an identical camera tree.

## 7. Candid Review

- Rounds 1–3 against `origin/main` (`05001c9`): `CHANGES_REQUESTED`, one MAJOR (test 37 fails on that base because
  #345 is not merged; resolved by stacking, never by editing the test or adding a dependency), one MINOR (a single
  battery takes its percent only from `capacity`, as specified) and one SUGGESTION (`HEAD /api/battery` also reads,
  harmless through the cache).
- Stacked rounds, base frozen with `CANDID_BASE_REF=origin/feat/remote-live-camera ./scripts/candid_subagent.sh
  --prepare` so the fingerprint binds only the battery diff: `APPROVED`, no CRITICAL or MAJOR finding. Remaining
  MINOR: the pre-push hook and CI compute the fingerprint against `origin/main`, so it matches only after #347 is
  merged and this branch is rebased with an identical camera tree. The final fingerprint, recorded in
  `AI/candid_review_report.md`, covers this walkthrough and the RBS matrix rows.

## 8. Verification Results

All on 2026-10-07, on top of `origin/feat/remote-live-camera` (`df4798e`):

- `cargo test --locked --all-features -p soos-remote -p soos-invariants --no-fail-fast`: 888 passed, 0 failed
  (`soos-remote` 351: `battery_tests` 18, `battery_server_tests` 13, `battery_config_tests` 1,
  `battery_routes_tests` 1, every pre-existing suite green; `soos-invariants` 537, including the 7
  `remote_battery_contract` tests and `test_matrix_claimed_rows_cite_only_existing_evidence`).
- `cargo clippy --locked -p soos-remote -p soos-invariants --all-features --all-targets -- -D warnings`: clean.
- `cargo fmt --all -- --check`: clean.
- `cargo deny --locked check licenses bans sources`: bans ok, licenses ok, sources ok.
- `python3 scripts/sync_issue.py --check`: OK (GitHub-only issue, no backlog sub-issue to tick).

## 9. Known Limitations / Follow-ups

- RBS12 (owner hardware check on the iPhone home-screen app) is pending a reinstall of this build.
- A single battery that exposes energy attributes but no readable `capacity` shows "Battery level unknown" (B-9, by design).
- The level is only as fine as the kernel and firmware report; external power is unknown when no non-battery supply
  exposes a valid `online` attribute.
- Merge order: #347 (#345) first, then rebase this branch onto `main` and re-run the candid review gate against
  `origin/main`.
