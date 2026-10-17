# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `feat/android-alerts`
- **Base (merge-base)**: `9e82795`
- **Reviewed-Diff-Fingerprint**: `1f37f7174bf516bd7159f309a77a314a5fba5b755186add2295b7bcf3f469a98`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_android_alerts.md`,
  `AI/auditor_constraints_android_alerts.md`, `AI/tester_contract_android_alerts.md`,
  `AI/walkthroughs/219_android_alerts.md`, `Docs/REMOTE_COMPANION.md`,
  `android/app/src/main/java/org/soos/remote/net/ApiJson.kt`,
  `android/app/src/main/java/org/soos/remote/net/SoosApi.kt`,
  `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt`,
  `android/app/src/main/res/values/strings.xml`,
  `android/app/src/test/java/org/soos/remote/ApiJsonTest.kt`,
  `android/app/src/test/java/org/soos/remote/SoosApiTest.kt`,
  `tests/invariants/src/android_native_contract.rs`

## 1. Executive Summary

GitHub #404 (PR 2 of #357) adds the failed-password alerts list and its acknowledgement to the native
Android app. The change is client-only: `git diff --stat` against the merge base shows no change under
`crates/`, `scripts/`, `.github/`, `android/gradle/`, `android/app/build.gradle.kts` or the manifest, so
RC-1, the dependency verification metadata, the `aapt2` allowlist and every Rust security invariant are
untouched. The wire contract was checked against the server source, not the spec: the field names,
nullability and integer types of `AlertsView`/`AlertRecord` (`crates/remote/src/alerts.rs`), the six
snake-case enums (`alerts.rs`, `journal.rs`), the `alerts_ack` gate order and refusal tokens
(`crates/remote/src/server.rs`), `check_action_csrf` and `parse_alerts_ack_headers`
(`crates/remote/src/routes.rs`) all match the Kotlin client. No CRITICAL or MAJOR finding; three MINOR
and one SUGGESTION items below.

## 2. Test Changes (mechanical listing from step 3)

- Test files touched: `android/app/src/test/java/org/soos/remote/ApiJsonTest.kt`,
  `android/app/src/test/java/org/soos/remote/SoosApiTest.kt`,
  `tests/invariants/src/android_native_contract.rs` (plus the tester contract document).
- `grep -nE '^-[^-].*(assert|#\[test\]|@Test|proptest!|#\[should_panic)'` on the frozen patch: **no
  output**. `git diff $(git merge-base origin/main HEAD) HEAD -- '*tests*' 'tests/'` (and `*test*`)
  contains additions only (509 insertions, 0 deletions).
- New escape hatches (`#[ignore]`, `@Ignore`, `should_panic`, tolerance/epsilon): **none**.
- `test_ana_s8_kotlin_hygiene` gains an add-only block (ACTION_ALERTS_ACK, the two paths, the two header
  names, `MAX_ALERT_HISTORY`, `ALERTS_EPOCH_HEX_LEN`, the body bound) inserted before the existing
  `Origin` needle; every pre-existing line of the function is unchanged. New test
  `test_ana_s14_alerts_contract`; new helpers `rust_usize_const`, `kotlin_int_const`.
- The Kotlin files gain imports and appended `@Test` methods only.
- Conclusion: **no pre-existing assertion was removed, weakened or changed.**

## 3. Deep Reasoning Audit

### Logic & Architecture
- Parser vs server: `through: u64`, counters `u32` (cap `4294967295` = `MAX_ALERT_COUNT`), `count: u32`
  with the record minimum 1, `id`/times `u64`, `Option<_>` serialized as `null` → nullable members
  accept absent or `null`. Tried: `through = 18446744073709551615` (JVM `BigInteger`, Android
  `Double`) → `long()` accepts only `Int`/`Long` → `null` on both runtimes. `1.5`, `"3"`, `true`, `-1`
  refused. PASS.
- Epoch: the server parses exactly 16 lowercase hex characters; `isAlertsEpoch` is the same set; a
  bad snapshot is refused locally before any I/O (`ackWithAnInvalidSnapshotSendsNothing`). PASS.
- Ack wire: body-less `POST`, `Origin`, `X-Soos-Action: alerts-ack`, one epoch and one decimal through
  header (the server requires 1–N ASCII digits, `Long.toString()` of a non-negative value satisfies
  it); no `Content-Type`; the cookie through the shared `send`. Same no-body POST shape as the existing
  `lock()`. PASS.
- `200` of the ack is parsed as the fresh view (`alerts_response(200, &view)`), refusals as
  `Refused(status, token)`. A `200 {"result":...}` is `Unreachable` (tested). PASS.
- UI flow: `stale_view` → feedback kept, `refresh()` re-reads the view, the button needs another tap
  (never a blind retry). `busy` gates the Refresh, Lock, Acknowledge and pull-to-refresh actions.
  Layout: the scrolling `Column` sits in a `PullToRefreshBox` inside a `fillMaxSize` `Box` with bounded
  height, so no nested infinite-height scroll. FINDINGS MINOR-1, MINOR-2.

### PAM Concurrency & Deadlines
- Not applicable: no change under `crates/pam` or any Rust crate. Network calls stay on
  `Dispatchers.IO` with the unchanged 5 s / 16 KiB transport. PASS.

### Panic Safety & Fail-Closed
- `parseAlerts` is total: `objectOf` catches `JSONException`; the private `Malformed` and
  `JSONException` are caught inside the function; no `!!`, every cast is `as?`. `alertTime` catches
  `DateTimeException`. A malformed view shows "Alerts could not be loaded", never "no attempts"
  (unknown state/enum refuses the whole view). PASS.

### Test Integrity & Anti-Weakening
- See section 2: additions only. The new tests would fail against plausible wrong implementations
  (an `opt*`-coercing parser fails the `"3"`/`1.5`/`true` cases; a client adding `Content-Type` or a
  second header fails the exact header-list assertion; sending an invalid snapshot fails the
  no-request assertion; 33 entries and 16385 chars are refused).
- Re-run: `cargo test -p soos-invariants --lib android_native_contract -- --test-threads=3`: 14 passed;
  `cargo test -p soos-invariants -- --test-threads=3`: 708 passed, 0 failed;
  `cargo clippy -p soos-invariants --all-targets -- -D warnings`: clean; `cargo fmt --check -p
  soos-invariants`: clean (all with the scoped low-memory environment).
- The JVM tests (ANA18, ANA19) cannot run on this host (no JDK/SDK); they are honestly marked pending
  the CI job `Android (debug build)` in the matrix. I read them for compile-level correctness
  (Long/Int literal typing, `ApiResult` data-class equality, `Fake` throwing `IOException` on an empty
  queue → `Unreachable`): no issue found. PASS (with the CI run as the remaining evidence).

### Memory, Bounds & Secrets
- Body ≤ 16384 characters (pinned to `MAX_RESPONSE_BYTES` by ANA-S8), history ≤ 32 (pinned to
  `lib.rs`), all strings from closed sets. The largest legal server view (32 records) is ~6 KB, well
  under the bound. No logging; no server text, status number or exception text is rendered on the
  alerts card (server tokens only select `strings.xml` resources). The session cookie stays in memory
  and is still dropped on `login_required`. No password ever transits (the alerts carry classes,
  times and counts only). PASS.

### Supply Chain & Automation
- No change to `libs.versions.toml`, `verification-metadata.xml`, `build.gradle.kts`, the manifest,
  CI workflows or scripts. `PullToRefreshBox` comes from the already-resolved material3 artifact;
  `java.time` is platform API at minSdk 34. PASS.

### English-Only Policy
- All code, comments, resources, docs, ADR, matrix rows and the walkthrough are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR-1]** `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt:224` (`loadAlerts`) and `:272`
  (`acknowledge`) — `ackFeedback` is only cleared by `clearAlerts()` or a new tap, so "Acknowledged."
  stays displayed after later refreshes, including when the refreshed view shows new unacknowledged
  attempts next to it. Suggested correction: clear `ackFeedback` when a refresh (not triggered by the
  acknowledgement itself) loads a new view.
- **[MINOR-2]** `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt:287` — on an ack refused with
  `login_required`, `send` drops the session, the follow-up `refresh()` sees a signed-out state and
  `clearAlerts()` erases the `alerts_ack_login_required` feedback, so that fixed sentence is never
  shown (the user still sees "Signed out" and the sign-in button, so this is cosmetic).
- **[MINOR-3]** `android/app/src/main/res/values/strings.xml:18` and `SoosApp.kt:485` — the spec
  (§3) asks for plurals; the implementation uses `_one`/`_other` string pairs selected by
  `count == 1`. Correct for English (the app is English-only) and documented in the walkthrough, but
  it deviates from the spec and Android lint reports `PluralsCandidate` warnings (not errors:
  `warningsAsErrors = false`, so CI is not blocked).
- **[SUGGESTION]** `tests/invariants/src/android_native_contract.rs:946` — the bare `Text(result`
  check only inspects the preceding character; `Text(text = result...)` or `Text(r.result)` would pass.
  Acceptable as a needle check; a stricter form could be added later.

## 5. Final Verdict

No CRITICAL or MAJOR finding; no pre-existing test assertion was changed. The Kotlin evidence (ANA18,
ANA19) remains pending the CI job `Android (debug build)` on the pull request, as the matrix states.

**VERDICT: APPROVED**
