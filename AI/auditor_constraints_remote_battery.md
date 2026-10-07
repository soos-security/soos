# Auditor Constraints — GitHub #346: Live PC Battery Level in `soos-remote`

- **Date**: 2026-10-07
- **Round**: 1
- **Branch**: `feat/remote-battery-status` (from `origin/main` 05001c9; nothing committed, pushed, rebased or stashed)
- **Audited**:
  - the ADR "[2026-10-07] Live Battery Level in `soos-remote`" (`AI/DECISIONS.md`);
  - `AI/architect_spec_remote_battery.md` (round 3);
  - `AI/plan_evaluator_report.md` (round 3, APPROVED, MINOR R3-1, R3-2, R3-3);
  - `AI/tester_contract_remote_battery.md` and its tests:
    - `crates/remote/tests/{battery,battery_config,battery_routes,battery_server}_tests.rs`;
    - `crates/remote/tests/common/battery.rs`;
    - `tests/invariants/src/remote_battery_contract.rs`;
    - the four setup migrations and the `mod` line in `tests/invariants/src/lib.rs`;
  - the code about to change: `crates/remote/src/{lib,config,routes,http,server,main}.rs` (in particular
    `server.rs::deadline` :111, `reserve_seq` :361, `session_still_valid` :426, `handle_connection` Funnel gate
    :1111-1140, `serve_stream` :2054-2240, `main.rs::run_service` :441-456), `crates/remote/assets/app.js`,
    `crates/remote/Cargo.toml`.
- **Out of bounds**: the #345 worktree (`feat/remote-live-camera`) was not opened by this audit. No host change was
  made.

## Audit checklist results

| # | Check | Command / evidence | Result |
|---|---|---|---|
| 1 | Panic paths | `grep -rnE '\.unwrap\(\)\|\.expect\(\|panic!\|todo!\|unimplemented!\|unreachable!' crates/remote/src` | 0 hits today. The workspace lints apply to the new `battery.rs`: `unwrap_used` deny, `indexing_slicing` and `arithmetic_side_effects` warn, which `-D warnings` turns into errors. `test_rmc_production_code_never_panics_or_prints` scans every file of `crates/remote/src`, so it covers `battery.rs` with no change. A source panic stays inside `spawn_blocking` and becomes a `JoinError`: release `panic = "unwind"` (test 26). |
| 2 | Unsafe | `grep -rn unsafe crates/remote/src` | Only `#![forbid(unsafe_code)]` (`lib.rs:25`, `main.rs:25`). `O_NOFOLLOW`/`O_NONBLOCK` come through the safe `OpenOptionsExt::custom_flags` with `nix::fcntl::OFlag` bits, as `config.rs` does. The new test files contain no `unsafe` keyword, so the `candid_review.sh` keyword gate stays clean. |
| 3 | Output isolation | print macros in `crates/remote/src` | None; RMC and test 32 forbid them. |
| 4 | Bounded I/O | spec §3, §5.4, §5.5 | Entries ≤ 64, batteries ≤ 8, value ≤ 32 bytes. Regular files only (`O_NOFOLLOW \| O_NONBLOCK` + `fstat`). One 500 ms deadline covers the gate wait and the read. At most 1 blocking thread on sysfs; a still-stuck read short-circuits to `unavailable`. Runtime shutdown bounded at 1000 ms. HTTP-driven reads ≤ 1 per 5 s (cache), sampler reads only while a stream exists. `read_dir` on kernfs does not call drivers. The only unbounded kernel wait is the driver `show()` in `read`, which runs on the blocking pool behind the timeout and `in_flight`. |
| 5 | Arithmetic | spec §5.3, §5.5, R3-3 | Needs checked forms: deadline, stream counter, digit accumulation, energy sums, mean, `u64`→`u8`. See C-7 to C-10. |
| 6 | Filesystem | spec §5.4 | Read-only. No create, write, chmod or rename (test 32). Entry symlinks are followed on purpose (class layout). Attribute symlinks are refused (`O_NOFOLLOW`, test 11). No secret file is involved. |
| 7 | Secrets & privacy | RBS-I2, RBS-I5 | View = 4 keys. Names, serial, model and manufacturer are never read (tests 14, 33). No tracing in `battery.rs` (test 32). Test 31 has field-aware markers. The existing `debug!(?resolved, "request")` logs the route name `Battery`, never a value (acceptable). |
| 8 | Fail-closed | B-10, RBS-I4, B-5 | `no_battery` only after a complete scan. Errors become `unavailable`/unknown. `Route::Battery` is outside the `is_funnel_public` `matches!` allowlist (`routes.rs:306`), so anonymous Funnel gets `403 login_required` before routing (`server.rs:1119`). The stream re-validates the session before every battery event. No path reaches PAM, lock or unlock. |
| 9 | Supply chain | `crates/remote/Cargo.toml` | No new dependency. Every tokio (`sync`, `time`, `rt`) and nix (`fs`) feature is already enabled. Test 37 pins the post-#345 key set (see C-30). |
| 10 | CI / scripts | — | Only `scripts/install_remote.sh` gains one comment line and `# battery_status = true` (B-14). No workflow is touched. |

Test integrity:

- `git diff -- crates/remote/tests tests/invariants/src/lib.rs` shows **additions only**:
  - one `battery: soos_remote::config::BatteryConfig::default(),` line in each of the four `RemoteConfig` literals
    (`harness.rs:784`, `server_tests.rs:775`, `alerts_server_tests.rs:220`, `push_server_tests.rs:205`);
  - one `mod remote_battery_contract;` block.
- No assertion is changed. These are the setup-only migrations of spec §9, which the memory rule allows.
- Red evidence was re-checked: `cargo test -p soos-remote --test battery_tests --no-run` fails only with E0432 for
  `soos_remote::battery` and the §3 constants.

## Audit Constraints — Issue #346

### A. Panic safety and arithmetic

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C-1 | No `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!`, no `[i]` indexing or slicing, no `#[allow(clippy::…)]` of the D12 lints anywhere in production code. Use `.get()`, `split_last()`, `strip_suffix(b"\n")` and iterators only. | `battery.rs` (all), the battery lines of `server.rs`, `main.rs`, `config.rs`, `routes.rs`, `http.rs` | `test_rmc_production_code_never_panics_or_prints`; `cargo clippy -p soos-remote --all-targets --all-features -- -D warnings` |
| C-2 | `trim_value` strips **one** trailing `\n` with `strip_suffix`. It checks `raw.len() > MAX_SYSFS_VALUE_BYTES` **before** any other work and rejects an empty remainder. Bytes must be `0x21..=0x7E`, or an inner `0x20` (a leading or trailing space is malformed). | `battery.rs::trim_value` and every `parse_*` | tests 5, 6, 13, 15 (proptest, 512 cases) |
| C-3 | Every parser is total: it returns `None`/`Unknown`/`false`/`true` per §5.2 for every byte input and never allocates beyond its input. | `battery.rs::parse_*`, `valid_supply_name` | test 15 |
| C-4 | `aggregate` returns `unavailable()` for `len() > MAX_BATTERIES` before any arithmetic. | `battery.rs::aggregate` | tests 10, 16 |
| C-5 | A panic in `BatterySource::read` must never cross the runtime. It is caught by `spawn_blocking`'s `JoinError` → `unavailable()` (cached as an actual attempt). The `in_flight` guard is cleared by `Drop`. | `battery.rs::read_coalesced` | test 26 |
| C-6 | `serde_json::to_vec` failure of `BatteryView` → `Response::json(503, "unavailable")`. A stream serialisation failure ends the stream or skips the frame, never panics. Use compact serialisation (`to_string`/`to_vec`, never `_pretty`), so the SSE `data:` line has no newline. | `server.rs::battery_response`, `serve_stream` battery arm, `http.rs::encode_sse_battery_event` | tests 20, 23; review |
| C-7 | **R3-3, deadline.** `let deadline = start.checked_add(Duration::from_millis(BATTERY_READ_TIMEOUT_MS)).unwrap_or(start);` (or reuse the `server.rs::deadline` helper by moving it to a shared place). An unrepresentable deadline fails closed (`GateTimeout` / `unavailable`). No `Instant + Duration` operator anywhere in `battery.rs`; the sampler sleep uses `tokio::time::sleep(Duration::from_millis(..))`. | `battery.rs::read_coalesced`, `run_sampler` | `grep -nE 'Instant::now\(\) \+\|start \+\|\+ Duration' crates/remote/src/battery.rs` = 0; clippy `arithmetic_side_effects` |
| C-8 | **R3-3, counter.** `register_stream` increments with `streams.fetch_update(SeqCst, SeqCst, \|n\| n.checked_add(1))`. On `Err` (overflow, impossible below `MAX_SSE_STREAMS`) it still returns a guard that decrements nothing, or saturates, but never wraps. `BatteryStreamGuard::drop` decrements with `fetch_update(.., \|n\| Some(n.saturating_sub(1)))`. `wake.notify_one()` is called after the increment. | `battery.rs::BatteryRuntime::register_stream`, `BatteryStreamGuard::drop` | review against `server.rs:361-366`; test 27 |
| C-9 | Digit accumulation in `parse_capacity`/`parse_micro` uses `checked_mul(10)` + `checked_add(digit)` (`u64`/`u16`). The digit comes from `b.checked_sub(b'0')` or `char::to_digit`, never `b - b'0'`. The length bound (≤ 3 or ≤ `MAX_SYSFS_MICRO_DIGITS`) is checked first. | `battery.rs::parse_capacity`, `parse_micro` | tests 5, 15, 16; clippy |
| C-10 | Energy-weighted percent: `checked_add` sums, `checked_mul(100)`, `checked_div`. Any `None`, or `sum_full == 0`, falls through to the mean. The result goes `min(100)` then `u8::try_from(..).ok()`. The mean uses a `u32`/`u64` sum with `checked_add` and divides by `u64::try_from(len)` / `u32::try_from(len)` (no `as` casts that truncate or wrap). Several batteries with mixed kinds (`Energy` and `Charge`) never take the energy path. | `battery.rs::aggregate` | tests 8, 16; clippy `cast_*` lints |

### B. Bounded I/O, deadlines and concurrency

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C-11 | `read_attr` opens with `OpenOptions::new().read(true).custom_flags((OFlag::O_NOFOLLOW \| OFlag::O_NONBLOCK).bits())` (std adds `O_CLOEXEC`). It then calls `file.metadata()` and requires `is_file()`: a FIFO, directory, socket or device returns `None` **before** any `read`. A symlink fails at `open` (`ELOOP`) and returns `None`. | `battery.rs::read_attr` | tests 11, 32 |
| C-12 | The value read is bounded **by construction**: `file.take(u64::try_from(MAX_SYSFS_VALUE_BYTES + 1))` → `read_to_end` into a `Vec::with_capacity(MAX_SYSFS_VALUE_BYTES + 1)` (checked/const add). More than `MAX_SYSFS_VALUE_BYTES` bytes → `None`. Never `fs::read`, `read_to_string` or an unbounded `read_to_end`. Any `io::Error` (EIO, ENODEV, ENODATA, EAGAIN) → `None`. | `battery.rs::read_attr` | test 5 (33-byte value); `grep -nE 'fs::read\(\|read_to_string' crates/remote/src/battery.rs` = 0; review (no behavioural test pins a megabyte file; sysfs values are ≤ PAGE_SIZE anyway) |
| C-13 | `read_dir` iteration stops at the `MAX_POWER_SUPPLIES + 1`-th entry and returns `unavailable()` at once. It never collects all names first. Invalid entries count toward the bound. An iteration error → `unavailable()`. | `battery.rs::read_power_supplies` | test 10; review ("stop at once" is not observable in a test) |
| C-14 | Every attribute is read through `read_attr(&dir, "<literal>")`. Only the ten §5.4 literals are used. The four identifying names never appear in production code. | `battery.rs` | test 33 (R3-2 scoping applied) |
| C-15 | The single-flight protocol is exactly §5.5 steps 1-6, with one deadline for the gate wait and the join (`timeout_at(deadline, read_gate.lock())`, then `timeout_at(deadline, handle)`). Under the gate the cache is re-checked with the caller's rule. `in_flight` is tested and set with `compare_exchange(false, true, SeqCst, SeqCst)` (or `swap`) under the gate. If it is already set → `unavailable()`, no spawn. The clearing guard is **moved into** the `spawn_blocking` closure, so a cancelled or never-run task (runtime shutdown) also clears it. On timeout the `JoinHandle` is dropped (detached), never awaited again. | `battery.rs::read_coalesced` | tests 25, 26, 28, 38 |
| C-16 | `GateTimeout` is never cached and never published: `current()` returns `unavailable()` without storing it, and `sample()` returns `None`. Only actual attempts are cached (value, timeout, `JoinError`, still-stuck). | `battery.rs::current`, `sample` | test 38 (iii), test 23 |
| C-17 | Cache rules: `MaxAge` reuses iff `Instant::now().saturating_duration_since(stored_at) <= Duration::from_millis(BATTERY_SAMPLE_INTERVAL_MS)` (inclusive). `StoredSince(start)` reuses iff `stored_at >= start`. The cache mutex (`tokio::sync::Mutex`, no poisoning) is held only to copy or replace the `Copy` tuple, never across an `.await` other than its own `lock()`. | `battery.rs::read_coalesced` | test 28 (both sides of the boundary), test 25 |
| C-18 | The read gate is never held across a socket write, a `watch` send or a `Notify` wait. The cache is written **before** `views.send_replace(Some(view))` (the R2-3 argument depends on it). | `battery.rs::run_sampler`, `read_coalesced` | review |
| C-19 | `run_sampler` is an endless `loop`. While `streams == 0` it publishes `None`, then `let notified = wake.notified(); pin!(notified); notified.as_mut().enable();`, re-checks `streams`, then awaits, so a registration between the check and the await is never missed. Otherwise: `sample().await` → `Some(v)` → `send_replace(Some(v))`; then `sleep(BATTERY_SAMPLE_INTERVAL_MS)`. It never busy-loops. `serve` spawns it in the `supervised` `JoinSet` as `async move { run_sampler(rt).await; std::future::pending().await }` (a panic → `ServeError::TaskPanicked`). Shutdown aborts it. It is spawned only when `state.battery` is `Some`. | `battery.rs::run_sampler`, `server.rs::serve` | test 27; review |
| C-20 | `with_battery` stores the source only when `config.battery.enabled`. Otherwise `state.battery` stays `None`: no runtime, no sampler, zero sysfs access, `disabled` view, no battery frame. `ServerState::new` keeps `battery: None`. | `server.rs::ServerState::with_battery`, `new` | test 22 (`calls == 0`, no frame over 2 intervals) |
| C-21 | `run_service` ends exactly with `let code = runtime.block_on(run(config, credentials_path, uid)); runtime.shutdown_timeout(Duration::from_millis(RUNTIME_SHUTDOWN_TIMEOUT_MS)); code`. The early `EXIT_RUNTIME` return and `new_current_thread()` are kept. | `main.rs::run_service` | test 39; RMC-S10 |
| C-22 | The stream's first battery frame: register (`register_stream`) after the slot/subscriber registration and before the SSE head. After the first status and alerts frames: re-validate the Funnel session, then `let mut views = runtime.subscribe();` immediately followed by `let view = runtime.current().await;` (**no `.await` between them**), then send, `last_battery = Some(view)`. Never `subscribe()` or `borrow_and_update()` after the first battery write. (R2-3: static review only; no test pins it.) | `server.rs::serve_stream` | candid review (diff reading); tests 23, 24 indirectly |
| C-23 | The battery `select!` arm (`battery_views.changed()`, pending when `None`) goes after the alerts arms and before `session_check_at`. `Err` → break. `Some(v) != last_battery` → `session_still_valid(hash)` (Touch::Keep; never `validate_session` with `Touch::Refresh`, so battery events never extend a session) → send → update. A write failure ends the stream. It never touches `last_sent`, `last_sent_at` or `keepalive_pending`. | `server.rs::serve_stream` | tests 24, 29, 30 |

### C. Visibility, confidentiality and logging

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C-24 | `Route::Battery` is **not** added to `is_funnel_public`. Its handler runs only after the existing host (`421`), classification (`403`) and Funnel session (`403 login_required`) gates. No new early return before those gates, and no source read before them. | `routes.rs::is_funnel_public`, `server.rs::handle_connection` | tests 19, 21 (`calls == 0` after anonymous, foreign-login and wrong-host requests) |
| C-25 | `BatteryView` has exactly the four keys, in the order `state, percent, charge, external_power`. No `serde(flatten)`, no extra field, no timestamp (B-8). Entry names, the root path and `BatteryReading` never leave `battery.rs` (not in the view, an error or a log). | `battery.rs::BatteryView` | tests 14, 20, 23 |
| C-26 | No `tracing`/`log` macro in `battery.rs`. The battery lines of `server.rs` use fixed text without fields (for example `debug!("stream session ended")`). Never `?view`, `%percent`, `?reading`, `?source` or a `Debug` of `SysfsBattery` (which carries the root). | `battery.rs`, `server.rs` | tests 31, 32; `test_rmc_logging_never_names_identity_header_or_session_fields` |
| C-27 | Read-only: no `File::create`, `.write(true)`, `.create(true)`, `set_permissions`, `remove_*`, `rename(` or `create_dir` in `battery.rs`. No `SystemTime`. No D-Bus or env needle of RMC-S9 (`::system(`, `::session(`, `std::env`, …) in any battery line. The constructor is `SysfsBattery::kernel()`. | `battery.rs`, `main.rs` | tests 32, 34; `test_rmc_s9_bus_rules_match_the_presence_worker` |
| C-28 | `"/sys` (opening quote included, R3-1) occurs once in the production code of `crates/remote/src`: `lib.rs` `POWER_SUPPLY_ROOT`. `main.rs` wires `SysfsBattery::kernel()` only inside `if config.battery.enabled { … }`. | `lib.rs`, `main.rs` | test 34 |
| C-29 | The page writes the battery line with `textContent` (`setText`) only: no `innerHTML`, no new id, no storage, no `fetch` of `/api/battery`. Data comes only from `event: battery`. `JSON.parse` sits in a `try`, so a malformed frame is ignored. `clearBattery()` is called in `closeStream`, `source.onerror` and `showLogin`. `render` calls `showBattery(reachable)`. Unknown `state`/`charge` values give `null` or an empty suffix (fail closed). CSP, `index.html` and `sw.js` are unchanged. | `crates/remote/assets/app.js` | test 35; `test_rmc_s8_*`, `test_rmc_s45`–`s50` |

### D. Dependencies, test integrity and process

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C-30 | **No dependency change on this branch.** The developer must **not** add `soos-protocol` or `jpeg-encoder` (or anything else) to `crates/remote/Cargo.toml` to turn test 37 green. Test 37 stays Red, for the documented precondition only, until #345 is merged to `main` and this branch is rebased (tester contract §0). The final developer gate, the candid review and the push run only after that rebase, with the §12 conflicts resolved and migration M5 (`tests/common/camera.rs`) applied. The developer never opens or edits the #345 worktree. `cargo deny --locked check` runs after the rebase. | `crates/remote/Cargo.toml`, `Cargo.lock` | test 37; `git diff origin/main -- crates/remote/Cargo.toml Cargo.lock` empty before the rebase |
| C-31 | No test, fixture or assertion written in Phase 2 is modified, weakened, deleted or `#[ignore]`d. The only allowed test edits are spec §9 / contract §5 setup migrations (M1–M5). Any existing test that fails after the implementation is reported, never edited. | `crates/remote/tests/**`, `tests/invariants/**` | `git diff --numstat` of those paths after Phase 4 = the Phase 2 state plus M5 only |
| C-32 | No test reads the real `/sys`. `SysfsBattery::kernel()` may be constructed in tests but never `read()`. The battery tests do no host change (no `systemctl`, `sudo`, `/etc`, `~/.config`). | all battery tests | `grep -n 'kernel()' crates/remote/tests` (one construct-only hit, `battery_tests.rs:1069`) |
| C-33 | Every `.rs` file added under `crates/remote` is free of the bare `unsafe` keyword (the `candid_review.sh` gate counts added lines, string literals included, outside `//` lines). The `//!` doc of `battery.rs` says "never reads identifying attributes (names, serial, model, manufacturer)" only in comments. | `battery.rs` | `scripts/candid_review.sh` audit 1; test 33 (comments stripped) |
| C-34 | Docs (Phase 6) state the RBS-I3 bounds and the B-13 shutdown side effect, without inventing behaviour. The `Docs/REMOTE_COMPANION.md` "Battery level" section, §5 row, §6 row, `ARCHITECTURE.md` §13, `MOCK_STRATEGY.md` and the installer line are present. | `Docs/`, `AI/`, `scripts/install_remote.sh` | test 36 |

## Test-power review (tests that could stay green with a wrong implementation)

The contract is strong overall. The tester's mutation table (contract §4) shows 10 mutants killed. Residual gaps are
covered by static constraints above, not by new tests:

| Gap | Wrong implementation that would pass | Covered by |
|---|---|---|
| G-1 | Subscribing to `views` after the first battery write (R2-3 race: a change published during a slow first write is lost until the next change) | C-22, candid review only |
| G-2 | `read_to_end` without `take` (unbounded read of a huge regular file, then rejection by length) | C-12, grep |
| G-3 | Collecting every `read_dir` entry before checking the 64 bound | C-13, review |
| G-4 | Test 38 (ii) can be served by a cache hit if the GET is scheduled after the 50 ms read finished, so it does not prove coalescing on its own. Part (i) (`join!` of three callers, `calls == 1`) kills the "no re-check under the gate" mutant. | test 38 (i) |
| G-5 | `Instant + Duration` with the operator (panics only on overflow, which never happens in practice) | C-7, grep |
| G-6 | `Touch::Refresh` on battery re-validation would extend Funnel sessions every 5 s without failing a test | C-23, review |
| G-7 | The in-flight guard created outside the blocking closure: a task cancelled at shutdown leaves `in_flight` set. This does not matter at process exit, but is wrong in principle. | C-15, review |

None of these gaps allows a security bypass that a test misses and no constraint closes: Funnel visibility (test 21),
fail-closed scanning (tests 4, 10, 12), the off switch (test 22), revalidation (test 29), logging (test 31) and the
hung-driver bound (test 25) are all behaviourally pinned.

### Pre-existing violations found (not introduced by this change)

- None in `crates/remote/src`: 0 panic-path hits, no `unsafe`, no print macro.
- Observation only: `main.rs::run_service` today drops the runtime without a bound (`main.rs:455`). A blocking-pool
  task stuck at stop (push store write) could already delay shutdown until `TimeoutStopSec`. C-21 fixes this for
  every blocking task, as B-13 states.

### Clearance: CLEARED

Phase 4 may start. Conditions:

- Constraints C-1 to C-34 are binding.
- C-30 is a gate condition, not a defect: test 37 legitimately stays Red until #345 is merged and this branch is
  rebased. The developer's final green gate, the candid review and the push wait for that rebase.
- The developer must not add a dependency to satisfy test 37.
