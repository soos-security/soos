# Candid Review Report

- **Date**: 2026-10-06
- **Target Branch**: `feat/remote-funnel-passkey`
- **Base (merge-base)**: `222665f`
- **Reviewed-Diff-Fingerprint**: `9d326db3dfd5b8ac78ec6971015b546f25cfda8465007d33150df922c0cec29e`
- **Audited Files**: full branch patch (`target/candid_diff.patch`, 72 files); everything up to HEAD
  `7ba324a` plus the uncommitted `AI/MOCK_STRATEGY.md`, `AI/VERIFICATION_MATRIX.md` and walkthrough
  185 §9 changes was approved under the previous fingerprint `e5682f8c…`. The only delta since then
  is the closing paragraph of `AI/ARCHITECTURE.md` §13 (lines 389-393), rewritten and rewrapped.

## 1. Executive Summary

The previous review raised one MINOR: `AI/ARCHITECTURE.md` still said the remote unlock on the
phone was pending (RMC25), contradicting the matrix, plus a line-wrap suggestion. The rewritten
paragraph now reads "Verified by the owner on 2026-10-06:" followed by RMC20 (Serve forwarding:
`Host: localhost`, original name in `X-Forwarded-Host`, `X-Forwarded-Proto: https`), RMC21 (web app
and *Lock now*, Shortcuts steps still pending) and RMC25, RMC40, RMC42–RMC44 (Funnel reachability,
registration, Face ID login and unlock). Every statement was checked against the current matrix
rows; all match. The paragraph is rewrapped to 92–96 columns, consistent with the surrounding text.
Both earlier items are resolved. No production code, test, script or dependency changed.

## 2. Test Changes

Delta: one Markdown paragraph; no test file, assertion, `#[ignore]`, `should_panic`, tolerance or
inline test module is touched. Step-3 greps over the full branch patch return the same hits as the
previously approved review: 20 test files touched (new `crates/remote/tests/*`, invariant
contracts), one removed assertion line (already justified in the earlier approved reviews), one
escape-hatch hit (report prose only). Nothing new.

## 3. Deep Reasoning Audit

### Logic & Architecture
Each claim of the new paragraph against `AI/VERIFICATION_MATRIX.md`:
- **RMC20** — matrix: ✅ Verified (owner, 2026-10-06); evidence lists `Host: localhost`,
  `X-Forwarded-Host: arch.<tailnet>.ts.net`, `X-Forwarded-Proto: https`. The paragraph's
  parenthetical matches exactly. → PASS.
- **RMC21** — matrix: ⬜ Pending; owner report 2026-10-06 covers the home-screen web app and
  *Lock now*, Shortcuts steps not reported. The paragraph credits only those two items and states
  "Shortcuts steps still pending". It does not imply RMC21 is Verified. → PASS.
- **RMC25** — matrix: ✅ Verified (automated; hardware owner report 2026-10-06, Face ID unlock over
  Funnel, page showed `Unlocked`). Listed among the verified rows under "Face ID unlock". → PASS;
  the previous MINOR is resolved.
- **RMC40, RMC42, RMC43, RMC44** — matrix: all ✅ Verified 2026-10-06 (RMC40 now includes the Safari
  tab; RMC43 Safari tab, live events, *Sign out*; RMC44 both tailnet and Funnel paths). The paragraph's
  "Funnel reachability, passkey registration, Face ID login and Face ID unlock" maps one-to-one. → PASS.
- RMC41 is not mentioned, correctly: it is automated, not an owner check.
- Attempted break: does the paragraph credit anything still open? The only open owner item across
  these rows is the RMC21 Shortcuts step, and it is explicitly called pending. No overclaim.
- Stale-claim sweep (`pending|not reported|partly verified` across `AI/ARCHITECTURE.md`,
  `AI/MOCK_STRATEGY.md`, `AI/DECISIONS.md`, `Docs/REMOTE_COMPANION.md`, `Docs/README.md`, the
  specs, research notes and walkthroughs 183–185): no remaining pending claim for RMC20, RMC25,
  RMC40 or RMC42–RMC44 in a living document. Remaining hits are correct or historical:
  `AI/DECISIONS.md:121` "RMC21 pending owner check on the phone" (still true, Shortcuts);
  walkthrough 185 §9 "Shortcuts steps of RMC21 are still pending" (true) and the spec-fold item
  (unrelated); walkthroughs 183 line 9/315 and 184 line 9 are dated point-in-time records of their
  own phases, not status sources (see SUGGESTION). → PASS.

### PAM Concurrency & Deadlines
No code change in the delta. → PASS (not applicable).

### Panic Safety & Fail-Closed
No code change in the delta. → PASS (not applicable).

### Test Integrity & Anti-Weakening
No test change; matrix evidence columns untouched by the delta. → PASS.

### Memory, Bounds & Secrets
The paragraph contains no login, host name, credential id or tailnet name. → PASS.

### Supply Chain & Automation
No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change in the delta. → PASS.

### English-Only Policy
The rewritten paragraph is English. → PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `AI/walkthroughs/184_remote_unlock.md:9` and
  `AI/walkthroughs/183_remote_companion.md:9,315` — these say the RMC25 / RMC21 phone checks are
  pending. They are historical phase records and the matrix is the status source of truth, so no
  change is required; optionally add a one-line "superseded by matrix status of 2026-10-06" note.

No CRITICAL, MAJOR or MINOR findings.

## 5. Final Verdict

**VERDICT: APPROVED**
