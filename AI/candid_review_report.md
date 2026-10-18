# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `chore/docs-followups`
- **Base (merge-base)**: `3f76090`
- **Reviewed-Diff-Fingerprint**: `5d3a5afccf339eba5d038e60f6b9d759b1b52778f4ae9eb94990dd8c9d4f419f`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/238_docs_followups.md`, `Docs/REMOTE_COMPANION.md`, `README.md`

## 1. Executive Summary

Documentation-only change for GitHub #443 (part of #363): OBT9 prose, README companion wording (the
`[android]` Funnel rule, the three tours, the two kinds of offline alerts), a dated zbus 6 note in ADR
2026-10-02 "Presence Auto-Unlock Through logind", disambiguated "section 2j" cross-references in
`Docs/REMOTE_COMPANION.md`, a new matrix component `docs-followups` (DFW1-DFW5) and walkthrough 238.
No code, test, configuration or dependency file is touched. Every factual claim was checked against
the code on the merge base; all hold. No CRITICAL or MAJOR finding.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: none.
- `git diff <merge-base> HEAD -- tests 'crates/*/tests' android/app/src/test`: empty.
- Removed lines matching `assert|#[test]|...`: 1 hit, patch line 9, the ADR 2026-10-02 line of
  `AI/DECISIONS.md` (prose that contains the word "assert"); it is replaced by the same text plus the
  dated zbus note. Not a test.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance): none. Inline `mod tests` changes: none.

No pre-existing assertion was removed or changed.

## 3. Deep Reasoning Audit

### Logic & Architecture

- OBT9 "refuses tables other than one `[android]` table": `disable_remote_keys` in
  `crates/gui/src/feature_control.rs` returns `UnsupportedLayout` for a second header, any
  non-`[android]` header, or a target key inside the table. PASS.
- README "`soos-gui` turns `allow_funnel` off only once the `[android]` table is removed": the edit
  returns `FunnelRequiredByAndroid` when the parsed table contains `android` and the keys include
  `allow_funnel`; `FUNNEL_ANDROID_NOTE` in `crates/gui/src/onboarding.rs` says the same. PASS.
- README tours: §2k (web app tour) and §2j "Native Android app" / "Feature tour in the app" ("The tour
  switches nothing, makes no request") confirm both phone tours switch nothing; the `soos-gui` tour
  carries the switches. PASS.
- README PC offline alerts "off until it is turned on, and is turned off, in the app's settings":
  matches §2j "PC offline alerts" and "Turn it off with the same switch". PASS.
- "section 2j" cross-references: each edited reference (§2k, §5 `offline_alerts` and `[android]`
  rows, §6 `/api/offline` and `assetlinks.json` rows, §8, §9) was matched to the section it
  describes; every target is correct and both `## 2j.` headings are byte-identical. PASS.
- ADR zbus note: `Cargo.toml` pins `zbus = { version = "6", ... }`; commit `085ba63` (#417) names
  RUSTSEC-2026-0370/-0371/-0372. The original text is kept and a dated note appended, consistent with
  the existing dated-amendment practice and with the issue text. PASS.
- Matrix: DFW1-DFW5 all present; every cited test exists (`test_gak3_...`, `test_gak4_...`,
  `test_cwd_readme_names_push_and_offline_switches`, `test_rdm_companion_opt_out_names_dependent_keys`,
  `test_rdm_companion_wording_matches_setup_defaults`, `test_atr_s5_...`, `test_roa_s5_...`,
  `test_ana_s12_...`, `test_ahb_s4_...`, `test_aaf_s3_...`, `test_atr_s7_...`,
  `presence_logind_error_name_tests::test_standard_error_reply_keeps_its_exact_name`). PASS.

### PAM Concurrency & Deadlines

No PAM code touched. N/A, PASS.

### Panic Safety & Fail-Closed

No production code touched; the docs claim no new allow path. PASS.

### Test Integrity & Anti-Weakening

No test touched (section 2). Re-ran with the scoped environment: `cargo test -p soos-invariants --
--test-threads=3`: 787 passed, 0 failed (covers the README wording guards CWD4 / RDM10 / RDM12 and the
`## 2j.` heading lookups); `cargo test -p soos-gui --test remote_android_edit_tests --
--test-threads=3`: 8 passed; `cargo clippy -p soos-invariants --all-targets -- -D warnings`: clean;
`python3 scripts/sync_issue.py --check`: OK. PASS.

### Memory, Bounds & Secrets

No secrets, frames, embeddings or credentials in the text; no password-capture wording. PASS.

### Supply Chain & Automation

No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or hook change. N/A, PASS.

### English-Only Policy

All added text is English; non-ASCII characters are only typographic symbols already used in these
files. Commit subject is 70 characters, Conventional Commits, one scope. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `README.md:298` — "the two phone tours ... switch nothing and only say where each
  feature is turned off" slightly understates the tours, which also explain each feature. Optional
  rewording, e.g. "explain each feature and say where it is turned off".
- **[SUGGESTION]** `AI/walkthroughs/238_docs_followups.md` — the number skips 232-237 on this branch
  (presumably reserved by parallel branches); no invariant requires contiguity. Confirm at merge time.

## 5. Final Verdict

**VERDICT: APPROVED**
