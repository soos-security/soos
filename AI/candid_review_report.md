# Candid Review Report

- **Date**: 2026-10-07
- **Target Branch**: `feat/remote-battery-status` (GitHub #346, PR #348), stacked on `origin/feat/remote-live-camera` (PR #347)
- **Base (merge-base)**: `59be5ce` (`origin/feat/remote-live-camera`; frozen with `CANDID_BASE_REF=origin/feat/remote-live-camera ./scripts/candid_subagent.sh --prepare`, the documented base override, so the fingerprint binds only the battery diff and not the unmerged camera diff)
- **Reviewed-Diff-Fingerprint**: `f0cd7f90914b45305bfcf0c94ada8789363461c87d616b9830a8171648c4415a`
- **Rebase note**: the camera branch gained `59be5ce` (its own candid report, `AI/candid_review_report.md` only). The single battery commit was rebased onto it with `git rebase --onto origin/feat/remote-live-camera bb1d19a`; the only conflict was this report singleton (excluded from the fingerprint). The rebased tree is byte-identical to the previously reviewed battery commit `ca4285e` (`git diff ca4285e HEAD` is empty), so the battery diff itself is unchanged; it was re-frozen against the new merge-base and re-reviewed from the raw patch. A line-level interdiff of the changed lines of the old patch (`df4798e..de0e534`) and the new patch (`59be5ce..HEAD`) is empty. The interaction with the camera follow-ups (`fee8480`, `0a34872`, `1168206`, `bb1d19a`: stream content type, fullscreen modes, cooldown and camera-start push removal) was checked in the shared files `app.js`, `server.rs`, `http.rs`, `lib.rs` and `tests/common/camera.rs`: the battery hunks touch no camera code path, and the camera changes touch no battery path.
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/MOCK_STRATEGY.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_remote_battery.md`, `AI/auditor_constraints_remote_battery.md`, `AI/tester_contract_remote_battery.md`, `AI/walkthroughs/192_remote_battery_status.md`, `Docs/REMOTE_COMPANION.md`, `crates/remote/assets/app.js`, `crates/remote/src/battery.rs`, `crates/remote/src/config.rs`, `crates/remote/src/http.rs`, `crates/remote/src/lib.rs`, `crates/remote/src/main.rs`, `crates/remote/src/routes.rs`, `crates/remote/src/server.rs`, `crates/remote/tests/alerts_server_tests.rs`, `crates/remote/tests/battery_config_tests.rs`, `crates/remote/tests/battery_routes_tests.rs`, `crates/remote/tests/battery_server_tests.rs`, `crates/remote/tests/battery_tests.rs`, `crates/remote/tests/common/battery.rs`, `crates/remote/tests/common/camera.rs`, `crates/remote/tests/common/harness.rs`, `crates/remote/tests/push_server_tests.rs`, `crates/remote/tests/server_tests.rs`, `scripts/install_remote.sh`, `tests/invariants/src/lib.rs`, `tests/invariants/src/remote_battery_contract.rs`

## 1. Executive Summary

The diff adds a read-only, bounded sysfs battery reader (`crates/remote/src/battery.rs`), a single-flight cached runtime with a sampler that runs only while an event stream is open, `GET|HEAD /api/battery` with the visibility of `/api/status` (not in `is_funnel_public`, so a Funnel caller without a session gets `403 login_required`), an `event: battery` frame on the status stream (first after the status/alerts frames, then only on change, Funnel session re-validated before each frame), a `battery_status` config key (default on), a bounded runtime shutdown in `main.rs`, the page line in `app.js` (textContent only, hidden when unreachable, stream down or signed out), docs, an installer template comment and contract tests. No PAM, daemon, protocol or dependency change. Gates after the rebase: `cargo fmt --all --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` and `cargo test --locked --workspace --all-targets --all-features --no-fail-fast` (exit 0, no failed test) are green. No CRITICAL or MAJOR finding.

## 2. Test Changes (mechanical listing from step 3)

- Test files touched: `crates/remote/tests/{alerts_server_tests,battery_config_tests,battery_routes_tests,battery_server_tests,battery_tests,push_server_tests,server_tests}.rs`, `crates/remote/tests/common/{battery,camera,harness}.rs`, `tests/invariants/src/{lib,remote_battery_contract}.rs`.
- Removed/changed assertions (`^-` with assert/#[test]/proptest/should_panic): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none** (the single grep hit is the text of constraint C-31 inside the auditor document).
- Inline `mod tests` changes: **none**.
- Pre-existing tests edited: exactly five one-line additions `battery: soos_remote::config::BatteryConfig::default(),` to `RemoteConfig` literals in `alerts_server_tests.rs`, `common/camera.rs`, `common/harness.rs`, `push_server_tests.rs`, `server_tests.rs` (setup migrations for the new required struct field; no assertion touched). Justified.
- `tests/invariants/src/lib.rs`: only adds `mod remote_battery_contract;` with its doc comment.
- All other test files are new contract tests.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Parsers: tried `capacity` = `"100\n"`, `"101"`, `"007"`, `" 50"`, a 33-byte value, `"5\r"` → only canonical 0..=100 accepted. `status` outside the four kernel strings → `Unknown`. `online` `1`/`2` → true, `0` → false, anything else unknown. `energy_*` with 20 digits rejected; 19 digits fit `u64` (const assert). PASS.
- Aggregation: tried mixed energy/charge kinds, overflow of `sum_now * 100`, a battery without a pair, `full = 0` → falls back to the floor mean of capacities, else `percent: None`; `now > full` clamps to 100; never a guessed value. `external_power` is `None` when no non-battery supply reported a valid `online`. PASS.
- Scan: unreadable root or a failing `read_dir` entry → `unavailable`; the 65th entry → `unavailable`; the 9th included battery → `unavailable`; an invalid name or unreadable `type` with no battery found → `unavailable` (never a false `no_battery`); `scope = Device` excluded; `present = 0` excluded. Entry names are validated before `root.join` (no `/`, no leading `.`), so no traversal. Identifying attributes are never opened. PASS.
- Runtime: concurrent HTTP + sampler + first-frame callers → one gate, the cache is re-checked under the gate, one blocking read; a timed-out read leaves `in_flight` set so no second thread is spawned until it returns (`InFlightGuard` clears it on panic or when the task never runs); gate wait and read share one 500 ms deadline; a gate-wait timeout is never cached or published. PASS.
- Stream: the guard is registered before the first frames and dropped on every early return; `subscribe()` happens before `current()`, so a later publication always fires `changed()`, and an equal view is filtered against `last_battery`. The `watch` sender lives in `Shared`, so `changed()` cannot error while the stream runs. PASS.
- Page: `batteryText` fails closed on unknown states and out-of-range percent; the line is cleared on `onerror`, `closeStream` and `showLogin`. PASS.

### PAM Concurrency & Deadlines
- No file under `crates/pam` changed. N/A, PASS.

### Panic Safety & Fail-Closed
- `battery.rs` has no `unwrap`/`expect`/indexing (`unwrap_or` only); a panicking source is `JoinError` → `unavailable`. Serialization failure → `503 unavailable` / stream closed. The battery view grants nothing: no path reaches lock or unlock. PASS.

### Test Integrity & Anti-Weakening
- See §2. The new tests cover the parsers, aggregation, the bounded scan over tempdir sysfs fixtures, single-flight/timeout behavior, route visibility and stream framing; a wrong implementation (for example a guessed `no_battery` after an unreadable entry, or a public Funnel route) fails them. PASS.

### Memory, Bounds & Secrets
- Every attribute read is `take(33)` into a 33-byte buffer, regular files only (`O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC`, `fstat`); at most 64 entries and 8 batteries; the JSON body has exactly four bounded fields. The module never logs; no battery value, entry name or path is logged. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/` or `.githooks/` change; `nix::fcntl::OFlag` is an existing dependency. `scripts/install_remote.sh` only adds a commented template line. PASS.

### English-Only Policy
- Code, comments, docs and the commit message are in English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `crates/remote/assets/app.js` `renderBattery` — it calls `showBattery(true)` regardless of the last rendered reachability. Battery frames only arrive over the open stream from the PC, so this cannot show a stale line while the PC is unreachable; no change required.

## 5. Final Verdict

**VERDICT: APPROVED**
