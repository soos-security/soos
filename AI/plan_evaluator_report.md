# Plan Evaluation Report

- **Date**: 2026-10-07
- **Issue**: GitHub #346 — feat(remote): live PC battery level in the soos-remote web app (GitHub-only issue, no `AI/BACKLOG.md` entry; acceptance lines taken from the issue body and the ADR)
- **Branch**: `feat/remote-battery-status`
- **Base commit**: `05001c9` (`origin/main` = `HEAD`, nothing committed)
- **Round**: 3 (re-evaluation of `AI/architect_spec_remote_battery.md` Round 3, 842 lines, after the round 2 `REVISION_REQUIRED`)
- **Inputs read**:
  - `AGENTS.md` and the Round 3 spec in full.
  - The ADR "[2026-10-07] Live Battery Level in `soos-remote`" (`git diff AI/DECISIONS.md`) and the round 2 report.
  - `crates/remote/src/{lib,main,server,routes}.rs` (the relevant parts), `crates/remote/assets/app.js`, `crates/remote/Cargo.toml`, the workspace `Cargo.toml` (dependency features, lints, `[profile.release]`), `packaging/soos-remote.service` and `scripts/install_remote.sh`.
  - The log-capture test `server_tests.rs:2620-2690` and every crate-wide scan in `tests/invariants/src/remote_companion_contract.rs`: `remote_sources`, `strip_comments`, `production_part`, RMC-S1, S2, S3, S6, S9, S10, and the panic and logging hygiene tests.
  - The #345 worktree (`feat/remote-live-camera`, read-only, nothing modified): `git status`, the hunk list of `server.rs`, the diffs of `app.js` and `remote_companion_contract.rs`, and the crate-wide scans of `tests/invariants/src/remote_camera_contract.rs` (RLC-S4, RLC-S10).

Scope note: the owner's relayed request also asks for the live camera stream. That is #345 (`feat/remote-live-camera`), a separate branch. This report evaluates only the battery spec and how it merges with #345.

## 1. Coverage Matrix

| Acceptance line (GitHub #346 / ADR) | Spec element | Status |
|---|---|---|
| `battery_status` key, boolean, default `true`; `false` gives `disabled`, no event and no sysfs read | §4, §6.5, §6.6, RBS-I8; tests 18, 22 | Covered |
| Reader limited to the 10-attribute allowlist under an injectable root; it never reads names, serial, model or manufacturer | §5.1, §5.4; tests 14, 33, 34 | Covered (R3-2 affects how test 33 is scoped) |
| Bounds: ≤ 64 entries, ≤ 8 batteries, regular files `O_NOFOLLOW|O_NONBLOCK` + `fstat`, ≤ 32 bytes, 500 ms, ≤ 1 in flight, single-flight | §3, §5.4, §5.5; tests 10, 11, 25, 26, 32, 38 | Covered; test 25 now reaches step 5 |
| Fail closed: a missing root or a timeout gives `unavailable`; a malformed value is unknown; `no_battery` only after a complete scan; `scope=Device` and `present=0` are ignored; several batteries are aggregated | §5.2–§5.4, B-9, B-10; tests 3–10, 12, 13, 15, 16 | Covered |
| `GET|HEAD /api/battery` with a 4-key body; visibility equal to `/api/status`; anonymous Funnel gets `403 login_required` | §6.1, §6.2, RBS-I1; tests 19–21 | Covered (`is_funnel_public` is a `matches!` allowlist, `routes.rs:306-317`, so `Battery` is non-public with no code change) |
| Live update on `/api/events`: first `battery` frame after status (and alerts), change-only, 5 s sampler only while streaming, cache ≤ 5 s, Funnel re-validation; status unchanged | §5.5, §6.3, B-6; tests 23, 24, 27–30, 38 | Covered. The cache boundary and the subscription point are now pinned. |
| Page line built at run time: no new id, `textContent` only, CSP unchanged, fed only by `event: battery`, hidden when unreachable, stream down, signed out or disabled | §8, B-11; test 35, existing `test_rmc_s8_*`, `test_rmc_s45`–`s50` | Covered |
| No logging of values, no new dependency, `forbid(unsafe_code)`, no panic paths | RBS-I5, RBS-I6; tests 31, 32, 37 | Covered |
| Bounded runtime shutdown | B-13, §3; test 39 | Covered |
| Tempdir fake sysfs + scripted source; static invariants; docs; RBS1–RBS12; walkthrough | §10, §11, §13 | Covered |
| Owner check on the iPhone | RBS12 manual | Covered |

Every acceptance line maps to a spec element. The authoritative test count is §10.6: 17 + 1 + 1 + 13 + 7 = 39. The orchestrator summary ("37 tests … 12 in battery_server_tests and 6 static invariants") predates tests 38 and 39 and is stale.

### Round 2 findings — resolution check

| Round 2 finding | Resolution in the Round 3 spec | Verified |
|---|---|---|
| R2-1 (CRITICAL) `SysfsBattery::system()` matches the RMC-S9 needle `::system(` | Renamed `SysfsBattery::kernel()` (B-4, §1.1, §5.1); test 34 pins `kernel()` and forbids `SysfsBattery::system`; RMC-S9 is added to §7 with its full needle list | **Resolved.** I checked `kernel()` against every RMC-S9 needle (`remote_companion_contract.rs:1148-1171`) and the env-var rule (main.rs is exempt; `battery.rs` has no `env`). `register_stream`, `subscribe`, `session_still_valid` and `with_battery` contain no needle. |
| R2-2 (MAJOR) cache boundary contradicts test 28; test 25 never reaches step 5 | `CacheRule::MaxAge` (inclusive `age <= interval`, `saturating_duration_since`) and `CacheRule::StoredSince(start)`. Test 28 asserts both sides. Test 25 uses `+ 1` advances and closes its stream before step 4. | **Resolved.** I re-traced test 25 in paused time below. |
| R2-3 subscription point | §6.3: `subscribe()`, then `current().await` with no `.await` in between, then the first write | Resolved |
| R2-4 test 31 marker `capacity` | Field-aware markers only; the tester greps the fixed log text before freezing the list | Resolved, re-checked below (§2) |
| R2-5 `shutdown_timeout` side effects | Exact B-13 tail; other blocking tasks named; Docs §8 | Resolved. `Runtime::shutdown_timeout(self, ..)` consumes the runtime after `block_on` returns, so the tail compiles as written, and RMC-S10 (`remote_companion_contract.rs:1218-1253`) is unaffected. |
| R2-6 stale §12 `app.js` row | Row corrected | Resolved. The #345 `app.js` diff only adds `renderCamera`/camera code and one `showLogin(..)` call site; it does not touch `openStream`, `closeStream`, `onerror` or `render`. |
| R2-7 redundant CSS | `style.css` unchanged | Resolved |

#### Test 25 trace (Round 3 steps)

Paused clock, gated source; T is the virtual time at the first GET.

1. **First GET.** The read blocks; auto-advance is inhibited while the blocking task runs. `advance(500)` fires the deadline: the GET gets `unavailable`, which is cached at T+500, and `in_flight` stays set.
2. **Second GET.** `advance(5001)` makes the entry 5001 ms old, so the GET misses the cache and takes the gate. `in_flight` is still set, so step 5 returns `unavailable`, cached at T+5501. `calls == 1`, and this result can only come from step 5.
3. **Stream open.** The stream's `current()` is a cache hit (age 0). The sampler, woken by the registration, gets `StoredSince(T+5501)` and the entry stored at T+5501, so it reuses it without a read. `advance(15000)` wakes the sampler once (at T+20501). It misses the cache, hits step 5 again, and the equal view is filtered, so no new frame is sent. The status keep-alive arrives, and `calls == 1`. After the close, the sampler's next sleep ends at T+25501.
4. **After the release.** Releasing the gate clears `in_flight`. `advance(5001)` brings the clock to T+25502. The sampler wakes, sees `streams == 0` and goes idle without reading. The GET finds the entry from T+20501, now 5001 ms old, misses the cache, reads, and returns `present` with `calls == 2`.

The test now fails against an implementation that spawns a second read while the first is stuck (step 2), and against one that keeps sampling with no stream (step 4).

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| RMC-S9 needles | `remote_companion_contract.rs:1148-1171` | 21 needles, as §7 lists them; env reads only in `main.rs` | Yes |
| `"/sys` appears exactly once in the crate after the change (test 34) | `lib.rs:100` `SYSTEM_BUS_ADDRESS = "unix:path=/run/dbus/system_bus_socket"` | contains the **bare** substring `/sys` (in `/system_bus`), but not `"/sys` with the opening quote | Holds only if the needle keeps its opening quote (R3-1) |
| `serial_number`, `model_name`, `manufacturer`, `uevent` never appear in `crates/remote/src` (test 33) | `grep` over `crates/remote/src` (both trees) | none today | Yes, but the §5 `//!` doc the spec prescribes contains `manufacturer` (R3-2) |
| Tokio `sync`, `time` and `rt` features (`Mutex`, `Notify`, `watch`, `timeout_at`, `spawn_blocking`) | workspace `Cargo.toml:43` | `rt-multi-thread, net, sync, time, macros, signal, io-util` (+ `process` in the crate) | Yes |
| `nix` `fcntl::OFlag` and `unistd::mkfifo` (tests) | workspace `Cargo.toml:44` | `socket, fs, user` (both items are gated on `fs`) | Yes |
| `proptest`, `tempfile`, `tokio/test-util` dev-dependencies | `crates/remote/Cargo.toml:44-47` | present | Yes |
| A panicking source yields `JoinError`, not an abort | `[profile.release]` `panic = "unwind"` (`Cargo.toml:160`) | unwind in both profiles | Yes (test 26 matches production) |
| `run_service` tail | `main.rs:441-456` | `runtime.block_on(run(config, credentials_path, uid))` is the tail expression | Yes (B-13 shape applies cleanly) |
| Existing deadline and counter patterns that satisfy `arithmetic_side_effects` | `server.rs:110-113` `deadline()` (`checked_add(..).unwrap_or(from)`), `server.rs:363` `fetch_update(.., checked_add(1))` | as cited | Yes (R3-3 asks the spec to name them) |
| Log capture format for test 31 | `server_tests.rs:2623-2627` | `tracing_subscriber::fmt()`, TRACE, no ANSI, default wall-clock timer | Yes |
| Field-carrying log calls that could match test 31 markers | `grep` of every tracing macro with a field in `crates/remote/src` | `%err`, `kind = ?..`, `socket_path = %..`, `status = response.status` (2xx–5xx), `?resolved` (a `Route` `Debug`, e.g. `Battery`) | No collision: no `=87`/`=86`, `charge=`, `percent`, `discharging`; RFC 3339 timestamps contain no `: 8x`, `=8x` or `%` |
| `RemoteConfig` struct literals to migrate (§9) | `harness.rs:771`, `server_tests.rs:762`, `alerts_server_tests.rs:207`, `push_server_tests.rs:192`; #345 adds `tests/common/camera.rs` | exactly these, in both trees (the #345 hits in `src/server.rs` are only the `&RemoteConfig` accessor) | Yes |
| Stream EOF detection (test 25 step 3, test 27) | `server.rs:2232-2235` | `Ok(0) | Err(_)` breaks | Yes |
| `is_funnel_public` | `routes.rs:306-317` | `matches!` allowlist | Yes |
| `app.js` anchors | `app.js:122` `updatedNode`, `:157` `signedOut`, `:247` `showLogin`, `:270` `render`, `:361` `source.onerror`, `:368` `closeStream` | as §8.1 assumes | Yes |
| #345 leaves `serve_stream` alone | #345 `server.rs` hunks end at `@@ -2024 +2073` (camera functions); `serve_stream` at #345 line 2464 has no hunk | as cited | Yes |
| #345 crate-wide scans that would see `battery.rs` | `remote_camera_contract.rs:621-650` (RLC-S4: only `camera*` files and functions whose names contain `camera`), `:1061-1085` (RLC-S10: `jpeg_decoder`, `zune`, `image::`, `/dev/video`, `v4l`, `Decoder`) | no battery identifier matches | Yes |
| #345 `push.rs` line numbers cited in B-13 | #345 tree | `spawn_blocking` at `push.rs:483` and `push.rs:1323` (was 1293 on `main`) | Drift only (observation) |
| Installer template invariants | RMC-S6 (`remote_companion_contract.rs:716-790`), `remote_push_contract.rs:819-840` | required-substring and command checks only; a commented `# battery_status = true` line is unaffected (`#` lines are excluded from the command scan) | Yes |
| Unit sandbox does not hide `/sys` | `packaging/soos-remote.service` | `NoNewPrivileges`, `RestrictAddressFamilies=AF_UNIX`, …; no `ProtectKernel*`, `InaccessiblePaths` or `TemporaryFileSystem` | Yes (B-1 needs no unit change) |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- **Scenario: an anonymous Funnel visitor probes `/api/battery` or the stream.** `Route::Battery` falls outside the `matches!` allowlist, so it gets `403 login_required` before any source read (test 21 asserts zero reads).
- **Scenario: a revoked Funnel session keeps receiving values.** The session is re-validated before each battery event. Test 29 forces the change to land before `session_check_at`, so a missing re-check fails the test.
- **Scenario: "on battery" reveals that the laptop is away from mains.** That is visible only to the callers who already see active/idle/locked, which is less sensitive than the existing card. On by default is justified.
- There is no root, no daemon change and no new trust boundary. The user unit's sandbox does not hide `/sys`.
- Result: PASS.

### Pillar 2 — PAM deadline & concurrency
- `crates/pam` is untouched.
- **Scenario: concurrency inside `soos-remote`, cache boundary under paused time.** Test 28's hit at exactly `BATTERY_SAMPLE_INTERVAL_MS` needs the server's fast-path `Instant::now()` to equal the test's clock after `advance`. Auto-advance happens only when the runtime is idle with pending timers. The loopback request makes the connection task ready at once, and the only pending timers are the far harness and handler bounds. The same assumption underlies every existing `advance_ms`-based server test, so it holds.
- **Scenario: the sampler and the stream open in the same virtual instant.** `StoredSince(start)` uses `>=`, so an entry stored at the same instant counts as fresh. This coalesces the sampler with the stream's first read in paused time, and it is harmless in real time.
- Result: PASS.

### Pillar 3 — Panic safety & fail-closed
- **Scenario: the deadline arithmetic overflows (`start + 500 ms`).** It cannot in practice. If the code were written literally as §5.5 step 1 shows, clippy would flag it (`arithmetic_side_effects` is a workspace lint), so it needs the existing checked pattern (R3-3).
- **Scenario: the source panics.** With release `panic = "unwind"` this gives `JoinError` → `unavailable`, and the `in_flight` guard clears.
- Result: PASS (MINOR R3-3 is a wording issue).

### Pillar 4 — Dependencies
- No new crate. Every required tokio and nix feature is already enabled (§2). Test 37 pins the post-#345 key set, and Phase 2 is gated on the rebase.
- Result: PASS.

### Pillar 5 — Data confidentiality
- **Scenario: a supply name such as `hid-<MAC>-battery` or a serial leaks.** Names never leave `read_power_supplies`, and identifying attributes are never opened (tests 14, 31, 33).
- Test 31's markers no longer collide with any fixed or field-carrying log line (§2).
- Result: PASS.

### Pillar 6 — Test integrity
- **Scenario: a spec-compliant implementation fails a new static test.**
  - Test 34: a tester who drops the opening quote of the `"/sys` needle counts 2 occurrences in `lib.rs`, the second being `/run/dbus/system_bus_socket`, which RMC (`remote_companion_contract.rs:1115`) pins byte for byte. That test could then never pass (R3-1).
  - Test 33: if the tester scans raw text, it fails on the `//!` doc that §5 prescribes, which mentions "manufacturer" (R3-2).
  - Both traps are cheap to remove in the spec text, and neither forces a weakening of an existing test.
- The §9 setup-only migrations are complete in both trees.
- Result: FINDING (MINOR R3-1, MINOR R3-2).

## 4. Findings

- **[MINOR] R3-1: Test 34's needle must keep its opening quote, and the spec should say so explicitly.**
  - **What happens.** `lib.rs:100` already contains the bare substring `/sys` (`"unix:path=/run/dbus/system_bus_socket"`), and RMC pins that literal byte for byte (`remote_companion_contract.rs:1115`). With a bare `/sys` needle, test 34 counts 2 and can never pass.
  - **Required spec text.** Add to test 34: "the needle is `"/sys` (opening double quote included) or `"/sys/`; a bare `/sys` also matches `SYSTEM_BUS_ADDRESS` (`lib.rs:100`) and must not be used". The ADR phrase "the only `/sys` literal" stays correct with this reading.

- **[MINOR] R3-2: Test 33 conflicts with the prescribed `battery.rs` module doc.**
  - **What happens.** §5 prescribes a `//!` doc containing "never reads names/serial/model/manufacturer". Test 33 says `manufacturer` (and the other three needles) "never appear in `crates/remote/src`", without saying whether comments are excluded.
  - **Required change.** Either pin test 33 to `strip_comments(production_part(..))`, like test 34 (preferred, because the doc is useful), or reword the doc to "never reads identifying attributes". State the choice in the test text.

- **[MINOR] R3-3: Name the checked forms for the two arithmetic spots in §5.5.**
  - **What happens.** Step 1 writes `deadline = start + BATTERY_READ_TIMEOUT_MS`. Under the workspace lint `arithmetic_side_effects` (and the "no unchecked arithmetic" rule of §5), the developer needs `start.checked_add(Duration::from_millis(BATTERY_READ_TIMEOUT_MS)).unwrap_or(start)`, the `server.rs:110-113` `deadline()` pattern. That fallback fails closed: the wait expires at once and the result is `GateTimeout` / `unavailable`.
  - **`register_stream`.** The spec gives a saturating `fetch_update` only for the decrement. The increment should use `fetch_update(.., |n| n.checked_add(1))`, as at `server.rs:363`. A wrapped atomic cannot panic, but saturation keeps the count meaningful.
  - **Test.** None is required; this is a one-line clarification for the auditor and the developer.

Observations (no change required):
- The orchestrator summary's test count (37: 12 server and 6 static tests) is stale. The spec has 39, adding test 38 (server) and test 39 (static). Downstream agents should use §10.6.
- In the #345 tree, the second push `spawn_blocking` cited in B-13 is at `push.rs:1323` (`1293` on `main`). Refresh it at the rebase.
- §8.1 `renderBattery` calls `showBattery(true)` regardless of status staleness. A battery frame can only arrive on a live stream, and the next `TICK_MS` render applies the stale rule, so at worst there is a one-tick display. No change is needed.
- A stream that ends with a non-200 reconnect (`EventSource` CLOSED) keeps the battery line hidden until `openStream` runs again. This matches B-11 ("no value the stream is not keeping current").

## 5. Verdict

Round 3 resolves every round 2 finding:
- **R2-1:** the constructor `kernel()` clears all 21 RMC-S9 needles.
- **R2-2:** the inclusive `MaxAge` rule is pinned. Test 28 checks both sides of the boundary, and test 25 provably reaches step 5. I traced it step by step in paused time.

The MINOR items R2-3 to R2-7 are folded in correctly. Facts re-checked against both trees hold:
- dependency features, the release panic strategy and the unit sandbox;
- the log capture format and the field-carrying log lines;
- the migration literals;
- #345's crate-wide scans, and #345 leaving `serve_stream` unchanged.

Three new items are MINOR spec-text clarifications that keep two static tests (33, 34) from being written in a form that cannot pass, and name the checked-arithmetic pattern. They are not design defects and can go into the tester contract and the auditor constraints without another architect round.

VALIDATION_VERDICT: APPROVED
