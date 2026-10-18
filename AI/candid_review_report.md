# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `chore/docs-followups`
- **Base (merge-base)**: `ed0bb84`
- **Reviewed-Diff-Fingerprint**: `ad81c9f77c561caeddf9d2c264ee1571f0d57f966ba3e9e4dc226283564db6ad`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/238_docs_followups.md`, `Docs/REMOTE_COMPANION.md`, `README.md`

## 1. Executive Summary

Re-review after the rebase of `chore/docs-followups` onto `origin/main` `ed0bb84` (which gained #434,
#435, #440, #442, #445 and #447). The frozen patch is documentation only (GitHub #443, part of #363):
OBT9 prose, README companion wording (the `[android]` Funnel rule, the three tours, the two kinds of
offline alerts), a dated zbus 6 note in ADR 2026-10-02 "Presence Auto-Unlock Through logind",
disambiguated "section 2j" cross-references in `Docs/REMOTE_COMPANION.md`, matrix component
`docs-followups` (DFW1-DFW5) and walkthrough 238. No code, test, configuration or dependency file is
touched. Both README feature-table rows that conflicted during the rebase were checked against the code
on the new base and are true. No CRITICAL or MAJOR finding.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test, fixture or `tests/` files touched: none.
- Removed or changed `assert` / `#[test]` / `proptest!` / `#[should_panic]` lines: none. The single
  `^-` hit is the ADR 2026-10-02 paragraph in `AI/DECISIONS.md` (prose), replaced by the same text plus
  the dated note "(2026-10-18 note: migrated to zbus 6 by #417 for RUSTSEC-2026-0370/-0371/-0372)".
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.

No test weakening.

## 3. Deep Reasoning Audit

### Logic & Architecture

- **Phone companion row (`README.md:56`)**: kept from the branch. Its new clause "`soos-gui` turns
  Internet access (`allow_funnel`) off only once the `[android]` table of the native Android app is
  removed from `remote.toml`" matches `disable_remote_keys` in `crates/gui/src/feature_control.rs`
  on the base: when the parsed file contains `android` and the keys include `allow_funnel`, it returns
  `RemoteEditError::FunnelRequiredByAndroid` before any edit. The clause does not contradict #430:
  every other switch still edits the top-level keys above one `[android]` table (that table copied
  byte for byte, any other table or a target key inside it `UnsupportedLayout`); the row only restricts
  `allow_funnel`. The onboarding note `FUNNEL_ANDROID_NOTE` (`crates/gui/src/onboarding.rs`) says the
  same. PASS.
- **Native Android app row (`README.md:57`)**: identical to `origin/main` (#442 "read-only battery and
  offline-history cards"); the branch does not change it. Checked against `android/.../cards/CardModel.kt`
  and `ui/PcCards.kt` (cards present) and `SettingsStore.kt` (`heartbeat_enabled` defaults to `false`,
  matching "PC offline alerts once turned on in its settings"). PASS.
- **Security off-switch paragraph (`README.md:297-305`)**: "the two phone tours ... switch nothing and
  only say where each feature is turned off" matches `Docs/REMOTE_COMPANION.md` §2j "Feature tour in
  the app" and the web tour section (both "switches nothing"; each step gives the `soos-gui`
  **Features** switch or the `remote.toml` keys with their dependants). "While `remote.toml` has an
  `[android]` table, `soos-gui` does not turn Internet access off" matches `FunnelRequiredByAndroid`.
  The Android PC offline alerts are "off until it is turned on ... in the app's settings": matches the
  default `false` of `heartbeat_enabled`. The back-online notifications are attributed to
  `offline_alerts` of `soos-remote` (§2j "Offline history"). PASS.
- **Claims against the rest of the new base**: #434 (lock-screen installer), #435 (IR differential
  liveness cues), #440, #445 and #447 (release dry run) touch none of the edited README units, ADR
  paragraph, matrix rows or companion cross-references; no branch sentence describes their areas.
  The lock-screen widget row and the other feature-table rows are main's text, unchanged. PASS.
- **ADR note**: workspace pin `zbus = { version = "6", default-features = false, features = ["tokio"] }`
  (`Cargo.toml:65`) matches the note; the original "zbus 5" decision text is kept. PASS.
- **"section 2j" disambiguation**: every changed reference names "Offline history" or "Native Android
  app" correctly for its subject (offline intervals / back-online push → Offline history; assetlinks,
  signing key, PC offline alerts, app scope, app tour → Native Android app). Both `## 2j.` headings are
  unchanged. PASS.
- **Duplicates**: one `## Component: \`docs-followups\`` heading and one DFW1 row in the matrix, one
  `---` separator before it (after the RDR component of #447); one ADR "Presence Auto-Unlock Through
  logind (GitHub #323" entry; one Phone companion and one Native Android app feature row (the second
  "Phone companion" match at `README.md:330` is the comparison table, present on main). Walkthrough 238
  is new and fills the gap left on main between 237 and 239. PASS.
- **Invariants**: `cargo test --locked -p soos-invariants -- --test-threads=3` on the rebased tree:
  822 passed, 0 failed. PASS.

### PAM Concurrency & Deadlines

No Rust or PAM file touched. PASS (not applicable).

### Panic Safety & Fail-Closed

No production code touched; no wording claims a fail-open path. PASS.

### Test Integrity & Anti-Weakening

Section 2 listing: no test touched. Rows DFW1-DFW5 cite existing tests that exist on the base
(`test_gak3_funnel_is_refused_while_android_is_configured`, `test_gak4_...`, the README and companion
wording guards, the `## 2j.` heading checks); all ran green. PASS.

### Memory, Bounds & Secrets

No data, key, credential or command added. PASS.

### Supply Chain & Automation

No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. PASS.

### English-Only Policy

All added text is English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `AI/VERIFICATION_MATRIX.md` (component `docs-followups` intro, "the code on `main`
  (3f76090)") and `AI/walkthroughs/238_docs_followups.md:5` ("Base commit: `3f76090`") still name the
  pre-rebase base. Every claim was re-checked on `ed0bb84` and holds, so this is a provenance note only;
  updating it is optional.

## 5. Final Verdict

**VERDICT: APPROVED**
