# Candid Review Report

- **Date**: 2026-10-16
- **Target Branch**: `feat/lockscreen-adapters`
- **Base (merge-base)**: `2025738`
- **Reviewed-Diff-Fingerprint**: `7db27bb19b70f1d398379f41c85b5a2b1c73dd5af803292bc0cdd0aa844d82d4`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `AGENTS.md`, `AI/ARCHITECTURE.md`,
  `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_lockscreen_adapters.md`,
  `AI/auditor_constraints_lockscreen_adapters.md`, `AI/tester_contract_lockscreen_adapters.md`,
  `AI/walkthroughs/214_lockscreen_adapters.md`, `Cargo.lock`, `Cargo.toml`, `Docs/DISTRIBUTION_DEPLOYMENT.md`,
  `Docs/IPC_PROTOCOL.md`, `Docs/LOCKSCREEN_WIDGET.md`, `Docs/README.md`, `crates/lock-widget/Cargo.toml`,
  `crates/lock-widget/src/{args,font,layout,lib,main,palette,raster,render,scene,wayland}.rs`,
  `crates/lock-widget/tests/{lock_widget_cli_tests,lock_widget_render_tests,lock_widget_scene_tests}.rs`,
  `crates/lockstatus/Cargo.toml`, `crates/lockstatus/src/{args,client,format,lib,main,motion,poll,shared,view}.rs`,
  `crates/lockstatus/tests/common/mod.rs`,
  `crates/lockstatus/tests/{lockstatus_cli_tests,lockstatus_client_tests,lockstatus_poll_tests,lockstatus_shared_tests,lockstatus_view_format_tests}.rs`,
  `deny.toml`, `packaging/lockscreen/hyprlock-soos.conf`, `scripts/candid_review.sh`, `tests/invariants/src/lib.rs`,
  `tests/invariants/src/lockscreen_adapters_contract.rs`

## 1. Executive Summary

Independent review of the frozen patch (7 672 lines, 50 files), done without author context. The diff adds two
user-level `#![forbid(unsafe_code)]` crates: `soos-lockstatus` (bounded blocking `RequestKind::LockState` client,
per-user `flock` coordination in `$XDG_RUNTIME_DIR` with a 250 ms cache and a shared 1/2/4 s failure back-off,
`--once` for hyprlock labels, `--watch` JSON lines) and `soos-lock-widget` (wlr-layer-shell background for
`swaylock-plugin --command`, no seat, CPU rasterizer, bounded shared memory). The diff does not touch PAM, daemon or
protocol code. No new crate enters `Cargo.lock` (two path packages only). No existing test or assertion is modified.
The only change to an existing test file adds lines and removes none: two list entries and one module declaration.

Re-run locally with the shared-RAM environment: `cargo test -p soos-lockstatus -p soos-lock-widget` (103 tests,
all green), `cargo clippy -p soos-lockstatus -p soos-lock-widget --all-targets -- -D warnings` (clean),
`cargo test -p soos-invariants lockscreen forbid_unsafe` (15 green, including
`test_business_crates_forbid_unsafe_code`), `cargo fmt --all -- --check` (clean).

No CRITICAL, MAJOR or MINOR finding. Two SUGGESTIONS.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched (`grep '^diff --git' | grep 'tests?[/_.]|/tests/|fixtures'`):
  - New: `crates/lock-widget/tests/lock_widget_{cli,render,scene}_tests.rs`,
    `crates/lockstatus/tests/common/mod.rs`, `crates/lockstatus/tests/lockstatus_{cli,client,poll,shared,view_format}_tests.rs`,
    `tests/invariants/src/lockscreen_adapters_contract.rs`.
  - Modified: `tests/invariants/src/lib.rs`. Additions only: a `#[cfg(all(test, unix))] mod lockscreen_adapters_contract;`
    declaration, plus `"lockstatus"` and `"lock-widget"` appended to the `test_business_crates_forbid_unsafe_code` list.
    This makes the invariant stricter.
- Removed or changed checks (`^-[^-].*(assert|#[test]|#[tokio::test|proptest!|#[should_panic)`): **none**.
- New escape hatches (`#[ignore`, `#[cfg(any())]`, `should_panic`, `tolerance`, `epsilon`): **none**.
- Inline `mod tests` added or removed: **none**.
- Conclusion: no test is weakened. The `scripts/candid_review.sh` `BUSINESS_CRATES` list gains the same two crates
  and stays in sync with the invariant (checked by `test_lka12_both_crate_roots_forbid_unsafe`).

## 3. Deep Reasoning Audit

### Logic & Architecture

- *Can a refusal be shown as anything but Idle?* `query_lock_state` decodes `LockStateResponse` first. Any decoded
  `Response`, an `Allow` verdict included, then becomes `Refused`. A `Response` can never decode as a
  `LockStateResponse`: the codec is strict, rejects trailing bytes, and a `Response` payload is longer than 12 bytes.
  `fetch_view` maps every error to `UNAVAILABLE`, and `presented()` collapses that into Idle. Neither `ViewState` nor
  `Presented` has a success state. **PASS**.
- *Other protocol version*: `is_current()` is checked both in the client and in `LockView::from_response`. **PASS**.
- *Coordination and quota*: every client takes `LOCK_EX|LOCK_NB` on the private lock file and re-reads the record under
  the lock. A client that waited for another client's exchange therefore gets a `Cache` hit. A client contacts the
  daemon only when the record is older than 250 ms (or than the failure back-off step), and keeps one connection at a
  time. That gives at most about 4 requests/s against the daemon's 8/s quota (`LOCK_STATE_MAX_REQUESTS_PER_SEC`).
  Record writes and reads both happen under the lock, so no read is ever torn. **PASS**.
- *Starved exchange*: with less than 50 ms left after taking the lock, the result is `LockBusy`. No failure record is
  written and the poller keeps its last view, so nothing flickers. **PASS**.
- *Cache decoding*: the format is strict: six fields, a leading `v1`, decimal-only numbers and a 16-character lowercase
  hex hash. A record is consistent only if `unavailable` comes with sequence 0 and failures 1..=3, and vice versa. At
  most 129 bytes are read. The 79-byte maximum stated in the constant's comment is correct
  (3+14+1+20+1+1+1+20+1+16+1). **PASS**.
- *Scene state machine*: traced these sequences:
  - Scanning(1) → transient Unavailable → Scanning(1): the scan start is kept, and the stop is bounded by
    `scan_start + 4800` rounded down to the half-turn grid.
  - A new scan joining a running turn: `max(old_stop, wanted)` never exceeds the new scan's own bound, because
    `old_stop ≤ old_start + 4800 ≤ now + 4800`.
  - NotRecognized(2) → Unavailable → NotRecognized(2): no second shake.
  - Daemon restart: a lower non-zero sequence resets the shake and the scan start.
  - The daemon's neutral sequence-0 answer: ignored for epochs.

  This is consistent with the §14 sequence rule (`2 × scans − 1` while hidden, `2 × scans` once visible). **PASS**.
- *Brand mirror*: `star_contains` matches `STAR_SPIKES`. The spikes are quarter-circle Bézier approximations centred
  on the outer corners, with a radius of half the box. The palette values equal those in `crates/gui/src/theme.rs`.
  **PASS**.
- *CLI*: `--format` has `requires = "once"` and a default value, which does not break `--watch` (covered by the CLI
  tests). `--once` swallows usage errors and always exits 0. The widget delays every exit except help and version by
  1000 ms. **PASS**.

### PAM Concurrency & Deadlines

- `crates/pam` is not in the diff. Neither new crate has Tokio, zbus or an async runtime (checked by the dependency
  allow-list invariant). **PASS**.
- *Can the bridge block past its budget?* One cumulative `Deadline` bounds every blocking step:
  - connect: non-blocking socket, retried every 5 ms on `EAGAIN`, deadline re-checked after the loop;
  - every write and read: `SO_SNDTIMEO`/`SO_RCVTIMEO` re-armed with the remaining time, and a zero budget never
    reaches `set_*_timeout`;
  - the lock wait and the exchange share the caller's 250 ms budget (the exchange gets what is `left`).

  The stall, late-answer and missing-socket tests pin the bound. The stall test's lower bound (`≥ BUDGET − 20 ms`)
  proves the deadline really waits, and its upper bound includes CI slack. **PASS**.
- *Can the adapters starve `pam_soos.so`?* Each user has at most one short connection at a time. The remaining case
  (two other clients already at the regular cap) is documented in the ADR as accepted and pre-existing. **PASS**.

### Panic Safety & Fail-Closed

- Production code has no `unwrap`/`expect`/`panic!`/indexing: workspace clippy lints are clean, and the invariant
  `test_lka12_no_panic_or_print_macro_in_production_code` passes. Arithmetic is saturating or checked. Float-to-int
  casts are documented saturating `as` conversions with NaN → 0. **PASS**.
- `--once` has a silent panic hook and `catch_unwind`; a panic renders Idle and exits 0. The widget has a silent hook
  and `catch_unwind` on the main loop. A poller-thread panic drops the sender, which gives `ChannelEvent::Closed`,
  then `Exit::PollerGone` (exit 1 after the delay). **PASS**.
- Every failure path renders Idle, never a success state. Nothing in this diff can unlock or authenticate. **PASS**.

### Test Integrity & Anti-Weakening

- Mechanical listing above: no assertion removed or modified, no escape hatch. **PASS**.
- *Would a plausible wrong implementation pass?*
  - An unbounded read fails `test_lka1_stalled_daemon_times_out_within_the_budget`.
  - Allocating before the length check is caught by the 0/4097/`u32::MAX` test, which asserts the budget.
  - A missing `flock` fails the test where 8 concurrent clients must make a single connection.
  - Treating a refusal as success fails `test_lka1_refusal_of_any_verdict_is_unavailable`, which iterates every
    verdict including `Allow`.
  - The request deadline bound in `test_lka1_request_fields_and_one_request_per_connection` is loose (SUGGESTION 1).

  **PASS**, with one suggestion.
- The mock daemon uses the real `soos_protocol::encode`/`decode_client_message`, so it does not hide any part of the
  wire contract. **PASS**.

### Memory, Bounds & Secrets

- Response frame: the declared length is checked (`0 < n ≤ MAX_MESSAGE_SIZE`) before the `vec!`, so at most 4 100
  bytes are allocated. **PASS**.
- Widget buffers: `buffer_len` allows at most 8192 px per side and 33 554 432 px per surface, at most 67 108 864 px
  across outputs, at most 8 outputs, and 2 buffers per output. An output over budget gets no buffer and one stderr
  line that names only its size. The 512 MiB worst case of shared memory is documented as a follow-up. **PASS**.
- Coordination file:
  - the runtime directory must be absolute, not a symlink, owned by the UID, with `mode & 0o077 == 0`;
  - the file is opened with `O_NOFOLLOW | O_CLOEXEC` and mode `0600`;
  - `fstat` on the opened descriptor then checks: regular file, correct owner, `nlink == 1`, private mode.

  The `fstat` check also rejects a FIFO or device planted by the same user. **PASS**.
- Neither crate has a logger. Stderr lines never contain the state, and nothing biometric is ever received. **PASS**.

### Supply Chain & Automation

- `Cargo.lock` gains only the two path packages. `smithay-client-toolkit` 0.20.0 was already locked with the same
  features. `deny.toml` changes a comment only. `scripts/candid_review.sh` only extends the business crate list.
  No `.github/` or `.githooks/` change. **PASS**.

### English-Only Policy

- All code, comments, documentation and AI documents in the diff are English (`Bézier` is the English proper noun).
  **PASS**.

### Documentation Accuracy

- `Docs/LOCKSCREEN_WIDGET.md` matches the code and tests on all of these points:
  - the three states, their glyphs and words;
  - budgets: 8 outputs, 8192 px per side, 33 554 432 / 67 108 864 px;
  - the `--interval-ms` clamp (250..=2000) and the 1/2/4 s back-off;
  - the JSON key set and the 96-byte line bound;
  - the coordination rules: lock-busy behaviour per client, and stderr written only by the widget at start;
  - the record example, whose hash `6db2722693fa5425` matches the FNV-1a test vector;
  - the reduced-motion order, the motion tokens and the 4.8 s cap;
  - the troubleshooting row for the 250 ms session check.
- `Docs/IPC_PROTOCOL.md` §14 "Client implementation" matches `client.rs` and `shared.rs`: tagged request, `service`,
  `uid_hint`, deadline = monotonic + remaining budget (≥ 50 ms), decode order, one connection, flock + cache.
- ADR 2026-10-15 (end of `AI/DECISIONS.md`): items (1)–(9) match the code. This covers exit codes and delays, no seat,
  buffer policy, motion, reduced motion and accepted residuals; item 9g notes that the hyprlock glyph has no cap.
- `AI/VERIFICATION_MATRIX.md` LKA1–LKA17: every referenced test prefix exists and passes. The counts in walkthrough 214
  §8 match the `#[test]` counts: client 13, shared 17, view/format 14, poll 8, CLI 13; scene 17, render 16, CLI 5;
  11 invariant tests. One small wording mismatch is noted in SUGGESTION 2.
- `AI/walkthroughs/214_lockscreen_adapters.md` is consistent with the diff; "Existing tests migrated: none" is true.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `crates/lockstatus/tests/lockstatus_client_tests.rs:53`: the request-deadline check accepts any
  `deadline_monotonic_ns` up to `before + EXCHANGE_BUDGET_MS + 5 000 ms`. A client that sent a 5 s deadline would
  still pass, although the IPC §14 contract says "CLOCK_MONOTONIC + the budget". Optionally add a new, separate test
  that pins the upper bound closer to `before + EXCHANGE_BUDGET_MS` plus a small scheduling margin. Do not edit this
  assertion.
- **[SUGGESTION]** `AI/VERIFICATION_MATRIX.md:2414` (row LKA1): the row says "a `CLOCK_MONOTONIC` + 250 ms deadline".
  Through the coordinated path, the request actually carries `CLOCK_MONOTONIC` plus the budget left after the lock
  (at least 50 ms), as `Docs/IPC_PROTOCOL.md` §14 correctly states. Align the wording, for example "+ the remaining
  exchange budget, at most 250 ms".

No CRITICAL, MAJOR or MINOR finding.

## 5. Final Verdict

**VERDICT: APPROVED**
