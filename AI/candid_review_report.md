# Candid Review Report

- **Date**: 2026-10-06
- **Target Branch**: `feat/remote-auth-alerts`
- **Base (merge-base)**: `222665f`
- **Reviewed-Diff-Fingerprint**: `6d8107f243caae214db43e4af1894027a65f1327a97ef7aefcee4d67ee43f169`
- **Audited Files**: full branch diff vs `origin/main` (`target/candid_diff.patch`). HEAD
  `0c3fefe` was approved under fingerprint `4e900161…`. The only new delta (uncommitted, vs HEAD)
  is documentation: `AI/DECISIONS.md` (lines 124, 125) and `AI/VERIFICATION_MATRIX.md`
  (rows RMC59, RMC74). The previous review of this delta (`bdb7c55b…`) requested changes.

## 1. Executive Summary

The delta records the owner's confirmation of the alert safe subset in the Failed-Password Alerts
ADR and the Web Push ADR, and moves RMC59 / RMC74 to "Pending (partly verified)" with host checks
and owner reports. All three items of the `bdb7c55b…` review are resolved: the stale "the hand-off
asks the owner to confirm" clause is gone (F-1), both quotes carry an English translation (F-2),
and the "Not reported yet" lists now include the 5 s / ~10 s bounds and the Funnel-session case
(suggestion). A word-level diff vs HEAD shows no other change. No code, test, script or dependency
is touched. **APPROVED.**

## 2. Test Changes

Mechanical listing over the frozen patch: identical to the approved `4e900161…` review (one
removed assertion line in `tests/invariants/src/presence_unlock_contract.rs`, recorded contract
migration PAU17; topic value migrations in `push_tests.rs` / `push_server_tests.rs` justified by
Contract Migration 3). No new `#[ignore]`, `should_panic`, tolerance or epsilon. The delta touches
no test file.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Tried: F-1 leftover. `grep` of lines 124–125 for "pending", "asks the owner", "confirm": the Web
  Push status now ends "...the tests and the implementation exist (they cannot carry typed
  text)." with no residual request; line 124's "needs the owner's explicit confirmation before
  tests are written" is the original requirement, followed by the dated **Owner confirmation**
  record. Consistent. PASS.
- Tried: does RMC59 overclaim? Claimed: host checks (`state: active`, `lock_screen: monitored`,
  24 h replay, anonymous Funnel `403`) and the owner report "a wrong password at the lock screen
  reached the iPhone". Not reported list: 5 s bound, Funnel-session case, banner source and count,
  `sudo`, GDM, `drift_verrou`, *Acknowledge* across restart — this covers every clause of the
  criterion. Status `⬜ Pending`. PASS.
- Tried: does RMC74 overclaim? Claimed: one `apple` subscription, sandboxed sender delivering,
  test notification from both paths, lock-screen notification. Not reported: `sudo`, ~10 s bound,
  app-closed / phone-locked. Status `⬜ Pending`. PASS (see suggestion S-1).
- `⏳` → `⬜`: matrix convention recognized by the citation parser (MXC2); the two remaining `⏳`
  markers (SFU5, SFX4) are pre-existing and outside the delta. `cargo test -p soos-invariants`:
  500 passed. PASS.

### PAM Concurrency & Deadlines
- No code change in the delta; carried over from the `4e900161…` full-diff review. PASS.

### Panic Safety & Fail-Closed
- No code change in the delta; carried over. PASS.

### Test Integrity & Anti-Weakening
- No test change in the delta; carried-over listing unchanged. PASS.

### Memory, Bounds & Secrets
- Tried: does new text expose a password, account, endpoint, key or token? Only the placeholder
  `/home/<owner>/`, device class `apple` and status values; the confirmation explicitly excludes
  typed text. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change in the delta. PASS.

### English-Only Policy
- The French owner quote is kept verbatim as consent evidence (the ADR requires the owner's own
  words) and is now followed by the English translation "I confirm, install and configure
  everything" in both ADRs. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION] S-1** `AI/VERIFICATION_MATRIX.md` RMC74 — the criterion clause "the page lists the
  phone under devices" is neither explicitly claimed nor in the "Not reported yet" list
  ("subscribed one `apple` device" implies it only indirectly). Optional: state it either way.

No CRITICAL, MAJOR or MINOR finding.

## 5. Final Verdict

**VERDICT: APPROVED**
