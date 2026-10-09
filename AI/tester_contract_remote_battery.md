# Tester Contract — GitHub #346: Live PC Battery Level in `soos-remote`

- **Date**: 2026-10-07
- **Branch**: `feat/remote-battery-status` (Phase 2, TDD Red; nothing committed)
- **Inputs**: `AI/architect_spec_remote_battery.md` (round 3), `AI/plan_evaluator_report.md` (round 3, APPROVED,
  MINOR R3-1, R3-2 and R3-3 folded in below), ADR "[2026-10-07] Live Battery Level in `soos-remote`"
  (`AI/DECISIONS.md`).
- **Matrix rows**: RBS1–RBS12. RBS12 is the owner's manual hardware check and has no automated test.
- Every test below is a contract for `developer-agent`. It may not be modified, weakened or deleted.

## 0. Precondition gap: #345 is not merged yet

The spec header ("Merge order") requires Phase 2 to run on a `main` that already contains #345
(`feat/remote-live-camera`). That is not the case today: `origin/main` = `05001c9`, and `crates/remote/Cargo.toml`
has neither `soos-protocol` nor `jpeg-encoder`. The workflow orchestrator asked for Phase 2 anyway. So:

- The branch was **not** rebased. Nothing was rebased onto the #345 worktree, and that worktree was only read
  (its `crates/remote/Cargo.toml` diff).
- Test 37 (`test_rbs_s6_no_new_dependency`) pins the post-#345 key set, as the spec requires. On this branch it
  fails today for the precondition reason, in addition to being Red. It passes once #345 is merged and the branch is
  rebased. A script check against the #345 worktree manifest gives an exact match: 23 keys, no difference.
- After the rebase, the §12 conflicts apply. In particular, `crates/remote/tests/common/camera.rs` (#345) needs the
  setup-only `battery: soos_remote::config::BatteryConfig::default(),` line (spec §9, row 5). Every other test
  here is independent of #345.

## 1. Placement decisions

- **Tests 18 and 19** are mapped by the spec to `config_tests.rs` and `routes_tests.rs`. They use only new API, so
  they live in new files with the spec names unchanged: `crates/remote/tests/battery_config_tests.rs` and
  `crates/remote/tests/battery_routes_tests.rs`. This follows the #345 precedent. The existing `config_tests` and
  `routes_tests` suites therefore keep compiling and stay green during Red.
- **Test 15** (`test_rbs_parsers_never_panic`) is split into two proptest functions that share the spec name as a
  prefix:
  - `test_rbs_parsers_never_panic`: 512 cases of arbitrary bytes into every parser, with the strict acceptance
    rules;
  - `test_rbs_parsers_never_panic_canonical_capacity`: every value 0..=100, with or without a newline, is
    accepted.

  `grep test_rbs_parsers_never_panic` finds both.
- **Shared fixture** `crates/remote/tests/common/battery.rs` (tester-owned) provides:
  - `FakeSysfs`: a tempdir root with `battery`, `supply`, `mains`, `set`, `remove`, `fifo` (via
    `nix::unistd::mkfifo`), `dir_attr`, `symlink_attr` (the target lives in a second tempdir outside the root),
    `raw_entry` (raw-byte names) and `chmod_root`;
  - `ScriptedBattery`: a settable view; `calls`, `entered` and `returned` counters; four modes (immediate, delayed,
    gated, panic); and a `GateRelease` guard that opens the gate on `Drop`, with a 10 s real-time fallback;
  - `Tracked<S>` / `Probe`: counts started and finished reads of any source, so a test can wait in real time until
    no read runs before it moves the virtual clock.
- **Server tests** use their own `start` set-up, like `alerts_server_tests.rs`:
  - Funnel and passkeys are configured;
  - alerts are optional;
  - the source is injected through `ServerState::with_battery`;
  - their own frame reader accepts `status`, `alerts` and `battery`. The shared `harness::parse_event`, which
    asserts `status`, is untouched.
- **Time model** (spec §10):
  - virtual time moves only through `tokio::time::advance`, via `Harness::advance_ms`;
  - real-time waits (`wait_until`, `quiesce`, `next_frame`) cover only blocking-pool completion and never move the
    clock.

## 2. Test-facing API the developer must provide (beyond or pinned from the spec text)

| Item | Fixed by | Note |
|---|---|---|
| `soos_remote::battery::{aggregate, parse_capacity, parse_micro, parse_online, parse_present, parse_scope_included, parse_status, read_power_supplies, valid_supply_name, BatteryReading, BatterySource, BatteryState, BatteryView, ChargeStatus, EnergyPair, SysfsBattery, BatteryRuntime}` | tests 1–17, 38 | §5.1–§5.5 signatures, `pub` |
| `BatteryRuntime::new(Arc<dyn BatterySource>)`, `current(&self)`, `sample(&self)`, `subscribe(&self)` | test 38 (i) | the runtime is used directly, without a server |
| `ChargeStatus` and `BatteryState` serialize in `snake_case`; `BatteryView` serializes exactly the four keys, in the order `state, percent, charge, external_power` | tests 14, 20, 22, 23, 25 | the byte-exact §6.7 JSON (serde's field order) |
| `config::BatteryConfig { enabled }`, deriving `Debug, Clone, Copy, PartialEq, Eq, Default` (default `true`); `RemoteConfig.battery` | test 18, §9 migrations | |
| `ServerState::with_battery(self, Arc<dyn BatterySource>) -> Self` | tests 20–31, 38 | ignored when `battery_status = false` |
| `routes::{Route::Battery, BATTERY_PATH}` | test 19 | `Route` derives `Debug`, `PartialEq` already |
| `HEAD /api/battery` answers `200` with `Content-Type: application/json` and an empty body | test 20 | the existing `answer` path |
| Trimming: only one trailing `\n` is stripped; a leading or trailing space, an inner NUL or `\n`, a non-ASCII byte or more than 32 bytes is malformed. `"Not  charging"` (two spaces) is `Unknown`. `parse_present(Some(b"00"))` is `true` (malformed means present) | tests 5, 6, 7, 13, 15 | §5.2 |
| An invalid entry name, or an unreadable or malformed `type`, makes the scan incomplete (`unavailable` without a battery). A symlinked `type` attribute counts as unreadable | tests 10, 11, 12 | §5.4 steps 3, 4, 5 |
| `battery.rs` defines `fn read_attr(` and calls it with **string literals**: at least `"type"`, `"capacity"`, `"status"` and `"online"`, all in the §5.4 allowlist | test 33 | makes the allowlist check meaningful (constants would make it vacuous) |
| `battery.rs` contains `custom_flags`, `O_NOFOLLOW` and `O_NONBLOCK`, and none of: `SystemTime`, `.write(true)`, `.append(true)`, `.create(true)`, `.truncate(true)`, `create_dir`, `remove_dir`, `rename(`, `log::`, any print macro | test 32 | §5, §5.4, RBS-I6 |
| `main.rs` contains `config.battery.enabled`, `with_battery(` and `SysfsBattery::kernel()`, and never `SysfsBattery::new(`. `run_service` contains `block_on(` before `shutdown_timeout(` and names `RUNTIME_SHUTDOWN_TIMEOUT_MS` | tests 34, 39 | §1.1, B-13 |
| `lib.rs` defines each §3 constant once, in the exact form `pub const NAME: TYPE = VALUE;`, plus the three compile-time asserts `BATTERY_READ_TIMEOUT_MS < BATTERY_SAMPLE_INTERVAL_MS`, `BATTERY_SAMPLE_INTERVAL_MS < SSE_KEEPALIVE_MS` and `BATTERY_READ_TIMEOUT_MS < RUNTIME_SHUTDOWN_TIMEOUT_MS` | test 39 | §3 |
| `app.js`: `batteryNode.className = "detail battery"`; the functions `batteryText`, `renderBattery`, `showBattery` and `clearBattery`; `render(` calls `showBattery(`; `showLogin`, `closeStream` and `source.onerror` call `clearBattery()`; `", plugged in"` and `", on battery"` appear exactly once each; no `innerHTML` in `batteryText` | test 35 | §8.1 |
| `Docs/REMOTE_COMPANION.md`: a heading containing `Battery level`; a §5 row starting with `` | `battery_status` | `` whose default cell is `` `true` `` or `true`; a §6 table row containing `GET /api/battery`. `AI/ARCHITECTURE.md` §13 and `AI/MOCK_STRATEGY.md` "Remote Companion Doubles" mention `battery`. `scripts/install_remote.sh` has a line that trims to `# battery_status = true` | test 36 | §11, B-14 |

Plan-evaluator findings folded in:

- **R3-1.** Test 34 searches for `"/sys` with its opening double quote, over `strip_comments(production_part(..))` of
  every file. A bare `/sys` would also match `SYSTEM_BUS_ADDRESS` in `lib.rs:100`.
- **R3-2.** Test 33 scans `strip_comments(production_part(..))`, so the `//!` module doc of `battery.rs` may say it
  never reads "names/serial/model/manufacturer".
- **R3-3 (no test, binding on auditor and developer).** The read deadline must be
  `start.checked_add(Duration::from_millis(BATTERY_READ_TIMEOUT_MS)).unwrap_or(start)` (the `server.rs:110-113`
  `deadline()` pattern, which fails closed). `register_stream` must increment with
  `fetch_update(.., |n| n.checked_add(1))` (`server.rs:363`). The guard decrements saturating.

## 3. Test table

Red evidence comes from `cargo test --locked -p soos-remote --all-features --test <target> --no-run` (compile) and
`cargo test --locked -q -p soos-invariants remote_` (assertions). E0432 means an unresolved import, E0433 an
unresolved path, E0599 a missing variant, E0560 or E0609 a missing field. In every case the missing item is exactly
the specified API.

### 3.1 `crates/remote/tests/battery_tests.rs` (17 spec tests, 18 functions)

| Test | Matrix | Red evidence |
|---|---|---|
| `test_rbs_single_battery_discharging_on_battery` (1) | RBS2 | E0432 `soos_remote::battery`, `soos_remote::{MAX_BATTERIES, MAX_POWER_SUPPLIES, MAX_POWER_SUPPLY_NAME_LEN, MAX_SYSFS_MICRO_DIGITS, MAX_SYSFS_VALUE_BYTES, POWER_SUPPLY_ROOT}` |
| `test_rbs_single_battery_charging_on_mains` (2) | RBS2 | same |
| `test_rbs_no_battery_desktop` (3) | RBS3 | same |
| `test_rbs_unreadable_root_is_unavailable` (4) | RBS4 | same |
| `test_rbs_malformed_capacity_is_unknown` (5) | RBS4 | same |
| `test_rbs_status_strings_are_exact` (6) | RBS4 | same |
| `test_rbs_peripheral_and_empty_bays_are_ignored` (7) | RBS3 | same |
| `test_rbs_multiple_batteries_are_aggregated` (8) | RBS5 | same |
| `test_rbs_charge_aggregation_table` (9) | RBS5 | same |
| `test_rbs_entry_and_battery_bounds` (10) | RBS4, RBS6 | same |
| `test_rbs_non_regular_attributes_are_not_read` (11) | RBS6 | same |
| `test_rbs_type_unreadable_never_claims_no_battery` (12) | RBS3 | same |
| `test_rbs_external_power_rules` (13) | RBS4 | same |
| `test_rbs_view_json_shape_has_no_identity` (14) | RBS7 | same |
| `test_rbs_parsers_never_panic` and `test_rbs_parsers_never_panic_canonical_capacity` (15, proptest 512 cases) | RBS4 | same |
| `test_rbs_aggregate_never_overflows` (16) | RBS5 | same |
| `test_rbs_sysfs_source_rereads_live` (17) | RBS2 | same |

### 3.2 New single-test files (tests 18, 19)

| Test | Matrix | Red evidence |
|---|---|---|
| `battery_config_tests.rs::test_rbs_battery_config_key` (18) | RBS1 | E0432 `soos_remote::config::BatteryConfig`; E0609 no field `battery` on `RemoteConfig` |
| `battery_routes_tests.rs::test_rbs_battery_route` (19) | RBS8 | E0432 `soos_remote::routes::BATTERY_PATH`; E0599 no variant `Route::Battery` |

### 3.3 `crates/remote/tests/battery_server_tests.rs` (13 tests)

| Test | Matrix | Red evidence |
|---|---|---|
| `test_rbs_get_battery_tailnet` (20) | RBS8 | E0432 `soos_remote::battery`, `soos_remote::{BATTERY_READ_TIMEOUT_MS, BATTERY_SAMPLE_INTERVAL_MS}`, `config::BatteryConfig`; E0433 `BatteryConfig` |
| `test_rbs_battery_visibility_matches_status` (21) | RBS8 | same |
| `test_rbs_disabled_or_unwired` (22) | RBS1 | same |
| `test_rbs_stream_first_frames_order` (23) | RBS9 | same |
| `test_rbs_stream_live_update_on_change_only` (24) | RBS9 | same |
| `test_rbs_hung_source_is_bounded` (25) | RBS6 | same |
| `test_rbs_panicking_source_is_unavailable` (26) | RBS6 | same |
| `test_rbs_sampler_idle_without_streams` (27) | RBS9 | same |
| `test_rbs_get_uses_cache_within_interval` (28) | RBS9 | same |
| `test_rbs_funnel_session_revalidated_before_battery_event` (29) | RBS9 | same |
| `test_rbs_status_contract_unchanged_with_battery` (30) | RBS10 | same |
| `test_rbs_never_logs_battery_values` (31) | RBS7 | same |
| `test_rbs_concurrent_readers_share_one_read` (38) | RBS6, RBS9 | same |

Test 31's markers were frozen after the spec's grep over `crates/remote/src` on `main`.
No fixed log text and no field-carrying tracing call contains any marker. The only `capacity:` hits are struct
fields (`credentials.rs:227`, `server.rs:312`, `server.rs:346`), never log text. The test adds `ADPMARKER1` and the
generic `MARKER` to the spec's list (the supply names it creates).

### 3.4 `tests/invariants/src/remote_battery_contract.rs` (7 tests; `mod remote_battery_contract;` added in `tests/invariants/src/lib.rs`)

| Test | Matrix | Red evidence (assertion) |
|---|---|---|
| `test_rbs_s1_battery_module_never_logs_and_never_writes` (32) | RBS6, RBS7 | `crates/remote/src/battery.rs must exist (spec §1.1)` |
| `test_rbs_s2_attribute_allowlist_and_no_identifiers` (33, R3-2) | RBS7 | same |
| `test_rbs_s3_sysfs_root_single_source` (34, R3-1) | RBS11 | same |
| `test_rbs_s4_page_battery_line` (35) | RBS11 | `app.js must contain "addEventListener(\"battery\""` |
| `test_rbs_s5_documented` (36) | RBS11 | `no heading containing "Battery level"` |
| `test_rbs_s6_no_new_dependency` (37) | RBS11 | `crates/remote [dependencies] must equal the post-#345 set` (see §0) |
| `test_rbs_s7_runtime_shutdown_is_bounded` (39) | RBS6 | `run_service bounds the runtime shutdown (B-13)` |

The full invariant suite reports 519 passed and 7 failed: exactly the seven above.

### 3.5 Counts

- **39 spec tests in 40 functions:** 18 in `battery_tests`, 1 in `battery_config_tests`, 1 in
  `battery_routes_tests`, 13 in `battery_server_tests`, and 7 static.
- **33 behavioural functions** are Red at compile time, for the missing specified API.
- **7 static tests** are Red on their assertions.

## 4. Power check (outside the repository)

A reference implementation of §3–§8 was written in a scratch copy of the tree (session scratchpad, never in the
repository). It covers `battery.rs`, the constants, the config, the route, the encoder, the server wiring, `main.rs`,
the `app.js` line, minimal docs and the installer line.

- **Passing.** All 39 tests (40 functions) pass, except test 37, which needs #345's manifest. Every existing
  `soos-remote` test and every remote invariant also pass with the §9 migrations. Clippy is clean: `cargo clippy -p
  soos-remote -p soos-invariants --all-features --all-targets -- -D warnings`.
- **Mutants.** Each mutant is killed by the named test:

  | Mutant | Killed by |
  |---|---|
  | `<` instead of `<=` in `MaxAge` | 28 |
  | No `in_flight` check | 25 |
  | Sampler ignores `streams == 0` | 27 |
  | No cache re-check under the gate | 38 |
  | Invalid name not marking the scan incomplete | 10 |
  | Single battery preferring energy | 1 |
  | Missing `O_NONBLOCK` (FIFO blocks, failure after 5 s) | 11 |
  | `online = 2` treated as unknown | 13 |
  | Funnel session not re-validated before a battery event | 29 |
  | No change filter | 23, 24, 25, 29, 31 |

- **Fixes made before hand-off.** The power check caught and fixed three tester mistakes:
  - a HEAD assertion on a body;
  - a missing `clippy::cast_possible_wrap` allow (needed by the included harness);
  - a `useless_vec`.

## 5. Contract Migrations (setup-only, applied by the tester; no assertion touched)

| # | Test / file:line | Old | New | Kind / mandate |
|---|---|---|---|---|
| M1 | `crates/remote/tests/common/harness.rs:783` (`RemoteConfig` literal of `Harness::start_with`) | `push: …PushConfig::default(),` then `};` | adds `battery: soos_remote::config::BatteryConfig::default(),` | Setup-only (compile); spec §9 |
| M2 | `crates/remote/tests/server_tests.rs:774` | same | same | Setup-only (compile); spec §9 |
| M3 | `crates/remote/tests/alerts_server_tests.rs:219` | same | same | Setup-only (compile); spec §9 |
| M4 | `crates/remote/tests/push_server_tests.rs:204` | `push,` then `};` | same | Setup-only (compile); spec §9 |
| M5 (pending the #345 rebase) | `crates/remote/tests/common/camera.rs` (#345) | `RemoteConfig` literal | same line | Setup-only (compile); spec §9 row 5 |

Until the developer adds `RemoteConfig.battery`, these suites fail to compile (E0433/E0560), for the specified field
only: `server_tests`, `alerts_server_tests`, `push_server_tests`, `auth_server_tests` and `auth_capacity_tests` (the
last two include the shared harness). Every other existing `soos-remote` suite still compiles and stays green. No
other existing test was touched. An existing test that fails after the implementation must be reported, not edited.

## 6. Flakiness check

- **Battery tests, 10 runs.** `battery_server_tests` and `battery_tests` ran 10 times in a row against the reference
  implementation (default parallelism). Every run was green: 13/13 and 18/18.
- **Battery tests on two cores, 5 runs.** The same pair ran 5 more times pinned to two cores (`taskset -c 0,1`, the
  CI runner shape). Every run was green.
- **Wall time.** About 0.4 s for the server tests and 0.02 s for the pure tests, per run.
- **Gated source.** It releases on `Drop`. A wrong implementation that spawns a second read while one is stuck fails
  test 25 after the 10 s gate fallback instead of hanging.
