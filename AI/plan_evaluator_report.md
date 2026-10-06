# Plan Evaluation Report
- **Date**: 2026-10-06
- **Issue**: GitHub #339 — soos-remote failed-password alerts, Round 3 "clear acknowledged entries" (owner request 2026-10-06)
- **Branch**: `feat/remote-auth-alerts` (head `e8e3f28`, spec and ADR amendment uncommitted)
- **Base commit**: `47ab53e` (`origin/main`)
- **Scope evaluated**: `AI/architect_spec_remote_auth_alerts.md` section "Round 3 — clear acknowledged entries" (R3.0–R3.8) and the ADR amendment "(Amended 2026-10-06, "clear acknowledged entries" …)" in `AI/DECISIONS.md`. Round 1 of this evaluation.

## 1. Coverage Matrix

| Acceptance line / TDD test | Spec element | Status |
|---|---|---|
| Owner request: acknowledged entries disappear from the page/API immediately after a successful acknowledge | R3.1 (drop in the same critical section), R3.3 A′, R3.4 (`200` body, `GET`, next `event: alerts`), R3.5 (page renders returned view); tests 58, 60 | Covered |
| Acknowledged entries never reappear after a service restart (replayed attempts at or before the persisted marker) | R3.3 R′ (discard), R3.4 restart semantics; tests 59, 60 (restart part), migrated 19, 31, 54 | Covered (with the documented rule-M residual, see F-3) |
| Only attempts newer than the acknowledgement are shown | R3.3 coalescing (a new attempt never joins a removed record), R3.4; tests 58, 60 | Covered |
| "Acknowledged-only data" removed (JSON key, CSS class, page branch) | R3.2 (field and key removed, 7 record keys), R3.5; tests 21 (migrated), 62 | Covered (test power gap, F-1) |
| Refusal: no deletion of system journal entries, documented | R3.0, R3.6 (`Docs/REMOTE_COMPANION.md` §2c sentence), ADR (e); test 62 doc check | Covered |
| O-2: no password stored anywhere | R3.0, R3.8, ADR (b) | Covered (no new field, file or log) |
| Bounded memory | R3.3 eviction, R3.8; test 58 (1 000 attempts, then 40 → `MAX_ALERT_HISTORY`) | Covered |
| Epoch / stale-view rules unchanged | R3.3 A′ (`StaleView`, `BeyondNewest`, `through == 0` order unchanged); tests 58, existing 53 | Covered |
| Push notifications and counts unaffected | R3.4 (sink only for `Kept` and live; scheduler never reads the book); test 61 | Covered |
| Authorization, CSRF, rate limit unchanged | R3.4 (no new route/header/status/key); migrated `alerts-ack` CSRF test | Covered |
| No logging of data, audit via `audit.rs` only | R3.4 "Logging: none added" | Covered |
| ADR amendment records drop-vs-filter decision | R3.1, ADR (a) | Covered |
| Contract migrations recorded in `AI/tester_contract_alerts.md` citing the owner | R3.7 migration table (18, 19, 21, 31, 54, CSRF test, helpers) | Covered — verified each listed assertion exists (see §2) |
| Owner hardware results (traceability) | R3.6 (RMC59, RMC74 updates, RMC75 row) | Covered (wording caveat, F-4) |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `AlertRecord` has `pub acknowledged: bool` today | `crates/remote/src/alerts.rs:327` | present | Yes |
| `record` returns `Result<(), BookError>`, overflow checked first, `highest_seq = seq` before classification | `alerts.rs:409-415` | yes | Yes |
| Rule R predicate `at_us <= loaded_marker_us && at_us < started_us` | `alerts.rs:414-415` | identical | Yes |
| Coalescing requires equal `acknowledged` flag, window `ALERT_COALESCE_WINDOW_US`, newest record only | `alerts.rs:416-430` | yes | Yes |
| Eviction skips acknowledged records (`continue`) | `alerts.rs:449-451` | yes | Yes |
| `acknowledge`: epoch → `through == 0` → `BeyondNewest` order; `ack_high_water_us` raised per record; evicted reset iff `through >= evicted.max_seq`, `max_seq` kept | `alerts.rs:477-500` | yes | Yes |
| `marker()` uses only unacknowledged records, `evicted.first_us`, pending, `ack_high_water_us` | `alerts.rs:505-518` | yes | Yes |
| View filters `!r.acknowledged` for totals and `last_*`; `through = highest_seq`; history newest first | `alerts.rs:526-557` | yes | Yes |
| Runtime `acknowledge`: book under `inner`, then `flush(true)`, `bump()`, `view()` | `alerts.rs:1033-1056` | yes | Yes |
| Runtime `record`: live attempts collected under the mutex, delivered after release | `alerts.rs:1062-1100` | yes (doc of `LiveAttemptSink` at `alerts.rs:803` still says "Called under the alerts runtime mutex" — stale, F-5) | Yes |
| `is_live` requires `at_us >= started_us` (disjoint from R′ discard `at_us < started_us`) | `alerts.rs:799-802` | yes | Yes |
| Push scheduler never reads the book | `crates/remote/src/push.rs:38` imports only `Attempt`, `AttemptKind`, `LiveAttemptSink` | yes | Yes |
| SSE alerts event serializes `runtime.view()` at send time | `crates/remote/src/server.rs:2114, 2212` | yes | Yes |
| Page dims rows via `record.acknowledged === true` | `crates/remote/assets/app.js:492-494` | yes | Yes |
| CSS `.alerts-history li.acknowledged` | `crates/remote/assets/style.css:203` | yes | Yes |
| Test 18 assertions on `acknowledged` | `crates/remote/tests/alerts_tests.rs:675, 682-683, 697, 709, 718-722` | yes; migrated values recomputed by hand (ids 3, 4, then 5/4; marker `T + 1.5 s − 1` unchanged) | Yes |
| Test 19 `(source, acknowledged)` list | `alerts_tests.rs:744-758` | yes; R′ yields `[Other, LockScreen, Sudo]`, `through == 5`, marker-0 case `len == 1` | Yes |
| Test 21 `acknowledged` key/literals | `alerts_tests.rs:926, 958, 1100` | yes | Yes |
| `counts`/`key` helpers, `RECORD_KEYS` (8) | `crates/remote/tests/alerts_server_tests.rs:368-415` | yes | Yes |
| `key(…, true)` call sites | `alerts_server_tests.rs:1470, 1580-1581, 1714` | exactly the CSRF test, test 31 and test 54 listed in the migration table | Yes |
| Test 54 restart expectation | `alerts_server_tests.rs:1655-1730` | marker `< t`; after restart first failure (`t − 20 s`) discarded, check at `t` and `sudo` at `t + 1 s` kept, `through == 3` | Yes (see F-3) |
| Test 52 eviction case unaffected | `alerts_tests.rs:851-880` | record 1 removed at ack, record 2 + 32 → record 2 evicted; all ids > 2, wrong total 1, marker `T − 1` | Yes |
| Audit test "acknowledged in memory" reads only the total | `alerts_server_tests.rs:2044` | yes | Yes |
| `crates/remote/tests/push_server_tests.rs` exists with `FakeTransport` and journal harness | file present | yes | Yes |
| ADR amendment contains `clear acknowledged entries` | `AI/DECISIONS.md` | 1 match | Yes |
| Next walkthrough number | `AI/walkthroughs/` | 187 is the highest → 188 | Yes |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario considered: the "delete" request leaking into a root action (journal vacuum) or a new privileged path. The plan refuses journal deletion explicitly (R3.0, ADR (e)) and adds no route, file, header or privilege. The service stays unprivileged; only `remote-alerts.json` (unchanged format, `0600`) is written.
- Failure scenario considered: a filter-only implementation leaving acknowledged data reachable through a future view or debug seam. The plan drops records (R3.1). Structural fact verified: the acknowledged set is always a **prefix** of the deque (records are appended in seq order and only the newest grows, so `last_seq` is increasing along the deque, and A′ removes `last_seq <= through`), so `VecDeque::retain` with this predicate is equivalent to popping a prefix and eviction behaviour is identical to round 2.
- Result: PASS.

### Pillar 2 — PAM deadline & concurrency (here: epoch / stale-view / lock races)
- `crates/pam` is untouched. Concurrency in `soos-remote`:
- Scenario A (attempt between view and ack): a new record has `seq > through` and survives; an attempt coalescing into a displayed record pushes its `last_seq > through`, so the whole record stays (unchanged semantics). Visible effect: after clicking, that row is still listed (with a larger count). Correct and fail-safe, but the owner may perceive it as "the entry was not deleted"; see F-6.
- Scenario B (SSE event in flight): an `event: alerts` serialized just before the ack can arrive after the `200` and briefly re-render the removed rows. The ack always `bump()`s, so a fresh event follows within the 1 s throttle and overwrites it. Transient, self-correcting, pre-existing; see F-6.
- Scenario C (restart between view and ack): `409 stale_view`, unchanged.
- Scenario D (lock order): sink still invoked after `inner` is released; R′ adds no lock.
- Result: PASS (observations F-5, F-6).

### Pillar 3 — Panic safety & fail-closed
- New code paths: `Recorded` enum, `retain`, removal of a branch. No index, `unwrap`, or arithmetic without saturation added. Overflow still reported before the discard decision (R′ step 1), so an overflow on a covered attempt still sets `unavailable` / `overflow`.
- Fail-closed direction: every residual (rule M fail-safe, ack-file write failure) shows **more** alerts, never fewer. Scenario: the ack file cannot be written → records are removed in memory (`200`, WARN audit line) but after a restart they reappear because the marker was not persisted. This is fail-safe but not stated in R3.4/R3.6; see F-2.
- Scenario (partial ack after eviction): evicted totals are reset only when `through >= evicted.max_seq`; if ≥ 32 new records arrive between the view and the ack, acknowledged evicted attempts stay counted (counts only, no record data). Fail-safe, pre-existing; should be stated (F-2).
- Result: PASS (F-2 MINOR).

### Pillar 4 — Dependencies
- No new crate, feature or version. `cargo deny` unaffected.
- Result: PASS.

### Pillar 5 — Data confidentiality
- Scenario: acknowledged data retained in memory after acknowledgement. Under A′/R′ nothing acknowledged is kept except two scalars (`ack_high_water_us`, `loaded_marker_us`) and, in the rare burst case above, aggregated evicted counts. No password, account name, raw line or cursor is added anywhere; no log line added; the ack file keeps its single `u64`.
- Result: PASS.

### Pillar 6 — Test integrity
- Each migrated assertion exists in code at the cited lines and encodes the old "acknowledged rows stay in the history" contract; each is justified by the owner's request; every count, `through`, `last_*` and marker value is kept (hand-checked for 18, 19, 31, 52, 54). Migrations are confined to `acknowledged`-related assertions and recorded in `AI/tester_contract_alerts.md`.
- Power check against a plausible wrong implementation "keep a **private** `acknowledged` flag and filter in the view": because the acknowledged set is always a deque prefix (Pillar 1), the view, the totals, eviction and the marker are identical to the dropping implementation; test 59's coalescing clause catches a filter implementation of R′ (an acknowledged replayed record between two kept attempts breaks coalescing), but **no test catches retention after A′**: test 62 only greps `pub acknowledged:`. See F-1.
- Power check "discard does not consume a seq": caught by test 59 (`through` advances) and migrated 19 (`through == 5`) and 54 (`through == 3`).
- Power check "`<` instead of `<=` for the marker": caught by test 59 (`at_us == marker` before start is discarded, `marker + 1` kept).
- Power check "ack cancels pending push": caught by test 61.
- Result: FINDING F-1 (MINOR).

## 4. Findings

- **[MINOR] F-1 — Test 62 cannot detect a private-field filter implementation of A′.** Every proposed behavioural test passes if `AlertRecord` keeps a non-`pub` `acknowledged` flag and the view filters it (the acknowledged set is always a deque prefix, so views are indistinguishable). Required plan change: test 62 asserts that `crates/remote/src/alerts.rs` contains no `acknowledged:` field declaration and no `.acknowledged` access at all (any visibility; `acknowledged_until_us` does not match `acknowledged:`), in addition to `pub acknowledged:`.
- **[MINOR] F-2 — Fail-safe residuals not fully stated.** R3.4 and the `Docs/REMOTE_COMPANION.md` §2c text mention only the rule-M residual. Add: (a) when `remote-alerts.json` cannot be written (`password alert acknowledgement not persisted`), the entries are removed from the page at once but reappear after a restart; (b) when more than 32 records arrive between the displayed view and the acknowledgement, evicted attempts stay counted in the totals (counts only) until the next acknowledgement.
- **[MINOR] F-3 — Test 54 migration should assert the documented residual explicitly.** After the restart, the acknowledged `sudo` attempt at `t + 1 s` lies above the marker (`< t`) and is shown again (rule-M residual). The migrated assertions check only the `lock_screen` record; add `counts == {(lock_screen, wrong_password): 1, (sudo, wrong_password): 1}` so the residual stated in R3.4/ADR (d) is pinned by a test and cannot silently change in either direction.
- **[MINOR] F-4 — "Identical persisted values" overclaims slightly; traceability wording.** R3.1 and ADR (b) say dropping never changes a persisted marker. This holds whenever journal time order matches seq order (eviction of unacknowledged records happens at the same moment in both designs because acknowledged rows are a prefix), but when late lines make journal time and seq disagree, removing an evicted-reset record now raises `ack_high_water_us` where round 2 did not; the marker can only be equal or higher and stays strictly below every unacknowledged attempt (still correct). Reword to "never lower than round 2 and still strictly below every unacknowledged attempt; identical in all migrated tests". Also, in the traceability phase, record that the owner's 2026-10-06 report ("Acknowledge resets the banner and stays reset after a service restart") was made on the round-2 build (`e8e3f28`): RMC75 needs its own owner check after reinstall (rows disappear and do not return after a restart); do not mark RMC75 hardware-verified from that report.
- **[MINOR] F-5 — Stale doc comment.** `LiveAttemptSink` (`alerts.rs:803-804`) says the sink is "Called under the alerts runtime mutex"; the code and R3.8 say it runs after the mutex is released. The developer should correct the comment while touching this file (doc only).
- **[MINOR] F-6 — UX note for §2c.** Document in one sentence that an entry that received a new attempt after the page was displayed is kept (with its new count) and that a just-removed list may flash back for under a second before the next update. No behaviour change.

No CRITICAL or MAJOR finding: the drop-at-once design is sound (prefix property makes `retain` exact and eviction-equivalent), R′ is disjoint from `is_live` and guarded additionally by `Kept`, the epoch/`through`/`BeyondNewest` order is preserved, push is independent of the book, memory stays ≤ `MAX_ALERT_HISTORY` records, and every migration preserves its numeric expectations.

## 5. Verdict

VALIDATION_VERDICT: APPROVED

Conditions carried to Phase 2 (tester) and Phase 6 (traceability): F-1 and F-3 are folded into tests 62 and the test-54 migration; F-2, F-4 and F-6 are folded into the ADR amendment wording, R3.4 and `Docs/REMOTE_COMPANION.md` §2c; F-5 is a doc-comment fix for the developer.
