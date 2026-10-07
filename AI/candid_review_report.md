# Candid Review Report

- **Date**: 2026-10-07
- **Target Branch**: `feat/remote-battery-status` (GitHub #346), stacked on `origin/feat/remote-live-camera` (PR #347)
- **Base (merge-base)**: `df4798e` (`origin/feat/remote-live-camera`; frozen with `CANDID_BASE_REF=origin/feat/remote-live-camera ./scripts/candid_subagent.sh --prepare`, the documented base override, so the fingerprint binds only the battery diff and not the unmerged #345 camera diff)
- **Reviewed-Diff-Fingerprint**: `3d71593f471906f3b7ceff21f8f63ee130427337f187a8a1b0fdb233ae7e0280`
- **Re-review (Phase 6 delta)**: the previous APPROVED round was bound to `4ce2a95e…a30491c`. Phase 6 then added only `AI/VERIFICATION_MATRIX.md` (component `remote-battery-status`, rows RBS1–RBS12) and `AI/walkthroughs/192_remote_battery_status.md`; no code, test, script or dependency changed. The patch was re-frozen with the same base override and re-reviewed (see §3 "Phase 6 documentation delta").
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/MOCK_STRATEGY.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/192_remote_battery_status.md`, `AI/architect_spec_remote_battery.md`, `AI/auditor_constraints_remote_battery.md`, `AI/tester_contract_remote_battery.md`, `Docs/REMOTE_COMPANION.md`, `crates/remote/assets/app.js`, `crates/remote/src/battery.rs`, `crates/remote/src/config.rs`, `crates/remote/src/http.rs`, `crates/remote/src/lib.rs`, `crates/remote/src/main.rs`, `crates/remote/src/routes.rs`, `crates/remote/src/server.rs`, `crates/remote/tests/alerts_server_tests.rs`, `crates/remote/tests/battery_config_tests.rs`, `crates/remote/tests/battery_routes_tests.rs`, `crates/remote/tests/battery_server_tests.rs`, `crates/remote/tests/battery_tests.rs`, `crates/remote/tests/common/battery.rs`, `crates/remote/tests/common/camera.rs`, `crates/remote/tests/common/harness.rs`, `crates/remote/tests/push_server_tests.rs`, `crates/remote/tests/server_tests.rs`, `scripts/install_remote.sh`, `tests/invariants/src/lib.rs`, `tests/invariants/src/remote_battery_contract.rs`

## 1. Executive Summary

The diff adds a read-only, bounded sysfs battery reader (`crates/remote/src/battery.rs`), a single-flight cached runtime with a sampler that runs only while an event stream is open, `GET|HEAD /api/battery` with the exact visibility of `/api/status` (not Funnel-public), an `event: battery` frame on the status stream (first frame after status/alerts, then only on change, Funnel session re-validated before each frame), a `battery_status` config key (default on), a bounded runtime shutdown in `main.rs`, the page line in `app.js` (textContent only, hidden when unreachable/stream down/signed out), docs, installer template comment and contract tests. No PAM, daemon, protocol or dependency change. `cargo test -p soos-remote` (all suites), `cargo test -p soos-invariants remote_` (89 passed) and `cargo clippy -p soos-remote --all-targets -- -D warnings` are green locally. No CRITICAL or MAJOR finding.

## 2. Test Changes (mechanical listing from step 3)

- Test files touched: `crates/remote/tests/{alerts_server_tests,battery_config_tests,battery_routes_tests,battery_server_tests,battery_tests,push_server_tests,server_tests}.rs`, `crates/remote/tests/common/{battery,camera,harness}.rs`, `tests/invariants/src/{lib,remote_battery_contract}.rs`.
- Removed/changed assertions (`^-` with assert/#[test]/proptest/should_panic): **none**. Removed lines in test files: **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none** (the single grep hit is the text of constraint C-31 inside the auditor document).
- Inline `mod tests` changes: **none**.
- Pre-existing tests edited: exactly five one-line additions `battery: soos_remote::config::BatteryConfig::default(),` to `RemoteConfig` literals in `alerts_server_tests.rs`, `common/camera.rs`, `common/harness.rs`, `push_server_tests.rs`, `server_tests.rs`. These are the spec setup migrations M1-M5 (new required struct field); no assertion touched. Justified.
- `tests/invariants/src/lib.rs`: only adds `mod remote_battery_contract;` with its doc comment.
- All other test files are new Phase 2 contract tests.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Parsers: tried `capacity` = `"100\n"`, `"101"`, `"007"`, `" 50"`, 33-byte value, `"5\r"` → only canonical 0..=100 accepted, others `None`. `status` other than the four kernel strings → `Unknown`. `online` `2` → true (kernel ABI). `energy_*` 20 digits → rejected; 19 digits fit `u64` (const assert). PASS.
- Aggregation: tried mixed energy/charge pairs, overflow of `sum_now * 100`, a battery without a pair, `full = 0` → falls back to floor mean of capacities, else `percent: None`; never guesses. `external_power` is `None` when no non-battery supply reported a valid `online`. Charge precedence Charging > Discharging > all Full > Not charging > Unknown. PASS.
- Scan: unreadable root or a failing `read_dir` entry → `unavailable`; 65th entry → `unavailable`; 9th included battery → `unavailable`; invalid name or unreadable `type` with no battery found → `unavailable` (never a false `no_battery`); `scope = Device` (peripheral batteries) excluded; `present = 0` excluded. Identifying attributes are never opened. PASS.
- Runtime: tried concurrent HTTP + sampler + first-frame callers → one gate, cache re-checked under the gate, one blocking read; a timed-out read leaves `in_flight` set so no second thread is spawned until it returns (`InFlightGuard` clears it even on panic or if the task never runs); gate wait and read share one 500 ms deadline; gate-wait timeout is never cached or published. PASS.
- Sampler: stream count 0 → publishes `None` and parks on a permit-storing `Notify` (lost-wakeup closed by `enable()` + re-check). Supervised in `serve` and never ends it on its own. PASS.
- Stream: receiver subscribed before the first `current()` with no `.await` in between; `None` and unchanged views filtered; sender lives in the shared runtime so `changed()` cannot spuriously error while a stream is open. PASS.
- Config: `battery_status` absent → true, `false` → `with_battery` ignored and `/api/battery` answers `disabled`. PASS.
- Single source of truth: all constants in `lib.rs`, `/sys` literal only in `POWER_SUPPLY_ROOT`. PASS.

### PAM Concurrency & Deadlines
- No file under `crates/pam` changed. The remote crate is a user-level Tokio service; the only blocking I/O runs in `spawn_blocking` under `BATTERY_READ_TIMEOUT_MS`, and service shutdown is bounded by `RUNTIME_SHUTDOWN_TIMEOUT_MS` so a read stuck in a hung driver cannot block stop. Attributes are opened `O_NONBLOCK | O_NOFOLLOW | O_CLOEXEC`. PASS.

### Panic Safety & Fail-Closed
- Grep of added production lines: no `unwrap()`, `expect(`, `panic!`, `todo!`, `unreachable!`, no indexing. A source panic becomes a `JoinError` → `unavailable`. Every failure maps to `unavailable` / unknown field, never to a guessed level; the feature does not touch any authorization path (status, lock or unlock decisions). PASS.

### Test Integrity & Anti-Weakening
- See section 2: zero assertion removed, only M1-M5 setup migrations. New tests exercise wrong-implementation cases (non-canonical capacity, false `no_battery` on unreadable entries, second thread after timeout, Funnel visibility). PASS.

### Memory, Bounds & Secrets
- Every read is capped (`take(MAX_SYSFS_VALUE_BYTES + 1)`), entry and battery counts capped, `Vec` capacities bounded. The JSON view has exactly four keys and no device names, serial, model or manufacturer; entry names never leave the module; the module never logs; server only logs the existing fixed `"stream session ended"` debug line. `/api/battery` is refused on Funnel without a session (not in `is_funnel_public`). Page uses `textContent` only. PASS.

### Supply Chain & Automation
- No `Cargo.toml`/`Cargo.lock`/`deny.toml`/`.github/` change; `nix` and `serde` were already dependencies. `scripts/install_remote.sh` only gains two commented template lines. PASS.

### English-Only Policy
- French-marker grep over the added lines: no hit. Code, comments, docs and test names are English. PASS.

### Phase 6 documentation delta
- Frozen patch: 31 files (the 29 code/test/doc files of the previous round, byte-identical, plus the two Phase 6 files). Mechanical listing re-run: 0 removed or changed assertions, 0 new escape hatches (the single hit is again the C-31 text of the auditor document), 0 inline `mod tests` changes.
- Matrix rows RBS1–RBS11 are `✅ Verified` and cite only `test_rbs_*` / `test_rmc_*` functions that exist; tried a misspelled and a moved test name mentally against `test_matrix_claimed_rows_cite_only_existing_evidence`, which resolves every citation by file stem and package and passes (`cargo test --locked -p soos-invariants`: 537 passed). RBS12 (owner hardware check) is `⬜ Pending`, not claimed. The row texts match the spec §13 table and the tests they cite (e.g. RBS8 cites the anonymous-Funnel test 21, RBS10 cites the RC-5 shape test). PASS.
- Walkthrough 192: number follows 191 (camera), repository-relative paths only (`grep /home/` empty), every section filled; the counts (888 = 351 + 537), the review history (rounds 1–3 `CHANGES_REQUESTED` on `05001c9` for test 37, then APPROVED when stacked) and the limitation text ("Battery level unknown", matching `app.js:344`) were checked against the tree. PASS.
- `./scripts/candid_review.sh` (keyword, unsafe, shell syntax, PAM output, English audits): PASSED. English-only: PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** Process — this report is bound to the diff against `origin/feat/remote-live-camera`. The pre-push hook and CI run `scripts/candid_subagent.sh` against `origin/main`, where the merge-base still includes the unmerged #345 diff, so the fingerprint will only match once PR #347 is merged and this branch is rebased onto `main` with an identical camera tree. Push after #347 merges (or re-run `--prepare` and re-review if the rebase changes any content).
- **[SUGGESTION]** `crates/remote/src/battery.rs` `read_power_supplies` — an entry whose `type` is unreadable is ignored when at least one battery was found; this matches the spec (a partial scan still reports the batteries it saw) but could under-report a second battery. Acceptable as specified.

## 5. Final Verdict

**VERDICT: APPROVED**
