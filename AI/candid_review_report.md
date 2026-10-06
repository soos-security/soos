# Candid Review Report

- **Date**: 2026-10-06
- **Target Branch**: `feat/remote-auth-alerts`
- **Base (merge-base)**: `222665f`
- **Reviewed-Diff-Fingerprint**: `3eadd21f0fe9a14c0965ec920b253988db4627ebda8a205195672977f840cc36`
- **Audited Files**: .agents/skills/dev-workflow/references/project-facts.md, AGENTS.md, AI/ARCHITECTURE.md, AI/DECISIONS.md, AI/MOCK_STRATEGY.md, AI/VERIFICATION_MATRIX.md, AI/architect_spec_remote_auth_alerts.md, AI/architect_spec_remote_companion.md, AI/architect_spec_remote_passkey_funnel.md, AI/architect_spec_remote_web_push.md, AI/auditor_constraints_alerts.md, AI/auditor_constraints_push.md, AI/auditor_constraints_remote_companion.md, AI/auditor_constraints_remote_passkey_funnel.md, AI/research_alerts.md, AI/research_funnel.md, AI/research_push.md, AI/research_webauthn.md, AI/tester_contract_alerts.md, AI/tester_contract_push.md, AI/tester_contract_remote_companion.md, AI/tester_contract_remote_passkey_funnel.md, AI/walkthroughs/183_remote_companion.md, AI/walkthroughs/184_remote_unlock.md, AI/walkthroughs/185_remote_funnel_passkey.md, AI/walkthroughs/186_remote_auth_alerts.md, AI/walkthroughs/187_remote_web_push.md, Cargo.lock, Cargo.toml, Docs/README.md, Docs/REMOTE_COMPANION.md, Docs/SECURITY_AND_QUALITY_GUIDELINES.md, crates/push-protocol/*, crates/push-sender/*, crates/remote/Cargo.toml, crates/remote/assets/*, crates/remote/src/*, crates/remote/tests/*, deny.toml, packaging/soos-push-sender.service, packaging/soos-remote.service, scripts/candid_review.sh, scripts/candid_subagent.sh, scripts/install_remote.sh, tests/invariants/src/{artifact_freshness,presence_unlock,remote_alerts,remote_companion,remote_passkey,remote_push}_contract.rs, tests/invariants/src/lib.rs (108 files in the frozen patch)

## 1. Executive Summary

The branch diff against `origin/main` was last approved at HEAD `e8e3f28`. This review covers the full frozen
patch, with depth on the only change since then: the uncommitted Round 3 change ("clear acknowledged entries",
owner request 2026-10-06). After that change, an acknowledgement removes acknowledged `AlertRecord`s from the
in-memory `AlertBook`, and `AlertBook::record` discards a replayed attempt that the loaded marker covers. These
attempts were previously stored with an `acknowledged = true` flag. The `acknowledged` field and JSON key are
removed, and the page no longer dims rows. The change also adds the ADR amendment, a §2c documentation paragraph,
four new tests (58–61) and one invariant test (62) with a scanner self-test. It also migrates the assertions in
tests 18, 19, 21, 30, 31 and 54 that encoded "acknowledged rows stay in the history".

`cargo fmt --check` is clean, and `cargo clippy -p soos-remote -p soos-invariants --all-targets -D warnings` is
clean. `cargo test -p soos-remote -p soos-invariants` passes, including 502 invariant tests. No CRITICAL or MAJOR
finding.

Re-review (fresh context, fingerprint `3eadd21f…`): since the previous approval of this Round 3 change
(fingerprint `655a58cb…`), only documentation changed: the traceability edits of `AI/VERIFICATION_MATRIX.md`
(RMC54 evidence, RMC59/RMC74 owner results, new RMC75 table), `AI/walkthroughs/186_remote_auth_alerts.md`
"Round 3", `AI/tester_contract_alerts.md`, `AI/auditor_constraints_alerts.md`, `AI/architect_spec_remote_auth_alerts.md`,
the ADR amendment in `AI/DECISIONS.md` and `Docs/REMOTE_COMPANION.md` §2c. The production diff of
`crates/remote/src/alerts.rs`, `app.js` and `style.css` was re-read against HEAD `e8e3f28` and is the one analysed below.
Documentation accuracy was checked claim by claim: every test named in RMC75 / RMC54 exists
(`grep "fn <name>"`); the §2c statements (removal from the `200` body, `GET` and next `event: alerts`; replays at or
before the marker discarded; record grown past `through` kept whole; marker held down by an older unacknowledged
attempt or pending check; unwritable `remote-alerts.json`; evicted totals until the next covering acknowledgement;
push unaffected; journal never modified) each match `AlertBook::{record, acknowledge, marker}` and the runtime;
the walkthrough's "covered records are always a prefix of the deque" holds because only the back record grows; the
migrated test list (18, 19, 21, 30, 31, 54) matches the ADR amendment ("the `alerts-ack` CSRF test" is test 30);
RMC75 is honestly not marked hardware-verified, and RMC59/RMC74 attribute the owner report to build `e8e3f28`. The
walkthrough R3.7 cites the earlier fingerprint `655a58cb…` together with "this traceability phase edits documentation
only, after that review", which is accurate history. Re-run in this review: `cargo fmt --all -- --check` clean,
`cargo clippy -p soos-remote -p soos-invariants --all-targets -- -D warnings` clean, `cargo test -p soos-remote -p
soos-invariants` all green.

## 2. Test Changes

Mechanical listing (step 3) on the frozen patch. The uncommitted Round 3 test edits are:

| File | Change | Justification |
|---|---|---|
| `crates/remote/tests/alerts_tests.rs` | Tests 18, 19, 21: `!r.acknowledged` / `r.acknowledged` / `(source, acknowledged)` assertions replaced by history length / id / source-order assertions; the record JSON key list drops `acknowledged`; new tests 58, 59 | Owner request 2026-10-06 (ADR amendment, spec R3.3, matrix RMC75, `AI/tester_contract_alerts.md` "Contract migration — owner request 2026-10-06"). The behaviour under test changed by owner decision. Each replacement keeps or raises the strength: exact ids, exact counts, `through`, and the marker values are still asserted. Each changed site carries an inline migration comment. |
| `crates/remote/tests/alerts_server_tests.rs` | `counts`/`key` helpers drop the `acknowledged` dimension and now assert that the key is absent; `RECORD_KEYS` 8 → 7; tests 30, 31, 54 migrated; new test 60 (end-to-end clear via `200`, `GET`, SSE, and restart) | Same owner request. Test 54 now also pins the rule-M fail-safe residual, where an acknowledged attempt above the marker is shown again. That behaviour is documented in §2c and the spec (R3.4, plan-evaluator F-3). |
| `crates/remote/tests/push_server_tests.rs` | New test 61 only (acknowledging never changes push) | Additive. |
| `tests/invariants/src/remote_alerts_contract.rs` | New test 62 plus scanner self-test | Additive. |

- Removed checks: every removed `assert` line reads the deleted `acknowledged` field. None removes a bound, a status code, a marker value or a security property.
- New escape hatches: none. The only match is a sentence in a walkthrough. No `#[ignore]`, `should_panic`, tolerance or `cfg(any())` was added.
- Inline `mod tests`: none touched.

## 3. Deep Reasoning Audit

### Logic & Architecture
- **Prefix removal correctness.** Coalescing only joins the back record, so `last_seq` is monotonic along the deque. `retain(last_seq > through)` therefore removes exactly a prefix. A record that grew past `through` survives whole, as specified and tested. PASS.
- **High water before removal.** The high water absorbs `last_us` of each removed record before `retain`. `marker()` still takes the minimum of the high water and (the earliest remaining record, evicted, or pending time) − 1. The old code filtered acknowledged rows out of that minimum, so the persisted marker is identical. With fewer evictions of unacknowledged rows, the marker is at most higher, and it still stays strictly below every unacknowledged attempt. PASS.
- **Discarded replay.** The condition `at_us <= loaded_marker && at_us < started_us` is unchanged from the old "acknowledged on arrival" rule. The seq is still consumed, so `through`, `BeyondNewest` and `409 stale_view` semantics are unchanged. A discarded attempt does not touch `records`, so it cannot split a coalescing run (tested). PASS.
- **Coalescing after acknowledgement.** The old code refused to join an acknowledged record. The new code cannot join one because it has been removed. The behaviour is equivalent (tested within 60 s). PASS.
- **Eviction.** The `if oldest.acknowledged { continue }` branch is gone. All stored records are unacknowledged, so every eviction is correctly counted in the evicted totals. The totals reset only when `through >= evicted.max_seq` (tested both ways). PASS.
- **Runtime.** `Recorded::Discarded` does not set `changed`, so no spurious `bump`. That is acceptable because nothing visible changed. The `through` increment becomes visible on the next bump, and acknowledging a stale `through` is accepted because the book compares against `highest_seq`. PASS.
- **Consumers.** The `acknowledged` field had no other reader in `crates/remote/src` or `push-sender` (grep confirms). The `app.js` dimming branch and the CSS rule are removed. PASS.

### PAM Concurrency & Deadlines
`crates/pam` and the daemon IPC are untouched. `soos-remote` is a user-level leaf crate. The live sink runs outside the runtime mutex, and the doc comment was corrected to say so. PASS (N/A to PAM).

### Panic Safety & Fail-Closed
- No new `unwrap`/`expect`/indexing in `alerts.rs` production code.
- A discarded attempt satisfies `at_us < started_us`, so `is_live` is false and push behaviour is unchanged (test 61).
- The only residuals err toward showing more alerts, never fewer. This is documented in §2c.

PASS.

### Test Integrity & Anti-Weakening
- I tried a plausible wrong implementation: a view-only filter that keeps records in memory. Test 58's bounded section would still pass, but test 62 forbids any `acknowledged` identifier in `crates/remote/src`. Test 58's eviction and total assertions, after acknowledging 1 000 attempts and then adding 40, would fail if acknowledged records still took up slots.
- I tried a second wrong implementation: removing records without updating the high water. Test 58's marker assertions (`T + 2 s`, `T + 4 s`) and test 60's `ack_marker` would fail.
- The scanner self-test protects test 62 from being vacuous.

PASS.

### Memory, Bounds & Secrets
- The history stays bounded by `MAX_ALERT_HISTORY` (32), and `retain` allocates nothing.
- No new logging. The `remote-alerts.json` format is unchanged.
- §2c explicitly states that the system journal is never modified.

PASS.

### Supply Chain & Automation
No change to `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or hooks since the approved state. PASS.

### English-Only Policy
All new code, comments, docs and the ADR amendment are in English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `tests/invariants/src/remote_alerts_contract.rs` (`blank_string_literals`): raw byte strings (`br"…"`) are not recognised as raw, because an `r` preceded by `b` is skipped, so the body is scanned as an escaped string. Correction of the earlier wording: the failure direction is not only a false positive. A `br"…\"` literal ending in a backslash would make the scanner treat `\"` as an escape and blank the following code up to the next quote, which could hide an `acknowledged` identifier from test 62 (false negative). No raw byte string exists in `crates/remote/src` today, so test 62 is currently sound; optional hardening: accept `b` (and `c`) before `r` in the raw-start check and add a self-test case.

- **[MINOR]** `AI/walkthroughs/186_remote_auth_alerts.md` (R3.7): repeats the earlier report's claim that the scanner suggestion's "failure direction is a false positive, never a hidden violation"; per the corrected suggestion above, a `br"…\"` literal could hide a violation. Documentation-only and not reachable with today's code; reword at the next traceability edit (and, if the scanner is hardened, drop the caveat).

## 5. Final Verdict

**VERDICT: APPROVED**
