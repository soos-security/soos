# Walkthrough 238 — Follow-Up Wording for the Android Table, the Tours and zbus 6

- **Date**: 2026-10-18
- **Issue**: GitHub #443 (sub-issue of epic #363; no backlog id, branch not registered in `scripts/sync_issue.py`) — **Branch**: `chore/docs-followups`
- **Base commit**: `3f76090`
- **Matrix criteria**: DFW1–DFW5 (new component `docs-followups`)

## 1. Context & Objectives

After #427 (Android tour), #429 (GUI edits above an `[android]` table) and #417 (zbus 6), five pieces of
text no longer matched the code:

1. Matrix row OBT9 said the off-only `remote.toml` edit "refuses tables"; since #429 it accepts one
   `[android]` table at the end of the file (`RemoteEditError::UnsupportedLayout`,
   `crates/gui/src/feature_control.rs`). The note of component `gui-android-safe-keys` already said so.
2. The README did not say that `soos-gui` keeps Internet access (`allow_funnel`) on while `remote.toml`
   has an `[android]` table (`RemoteEditError::FunnelRequiredByAndroid`, `FUNNEL_ANDROID_NOTE` in
   `crates/gui/src/onboarding.rs`), although `soos-remote` refuses that file (exit 78).
3. The README named only "the first-run tour" and did not tell the server-side offline alerts
   (`offline_alerts`, back-online push) apart from the Android app's own PC offline alerts.
4. ADR 2026-10-02 "Presence Auto-Unlock Through logind" still said `zbus` 5; the workspace pins zbus 6
   since #417 (`Cargo.toml`).
5. `Docs/REMOTE_COMPANION.md` has two sections numbered 2j, so a bare "section 2j" was ambiguous.

## 2. Architect Design

Wording only; no code, test, configuration or behaviour change, hence no new ADR. Constraints:

- The two `## 2j.` headings keep their text: the invariants `remote_offline_contract`,
  `android_native_contract`, `android_heartbeat_contract`, `android_alerts_feedback_contract` and
  `android_tour_contract` find them by it. Each cross-reference instead names its section
  (`section 2j "Offline history"` or `section 2j "Native Android app"`), which keeps the substring
  `section 2j` that `test_roa_s5_docs_adr_matrix_walkthrough` looks for in §8 and §9.
- The OBT9 test names stay; only the prose changes.
- The ADR keeps its original text; a dated note follows the zbus phrase.
- README units must keep passing the wording guards: any sentence with "off switch" also names push and
  offline alerts (CWD4), no unit calls unlock or the camera view "opt-in" or "off by default" (RDM10, so
  the Android PC offline alerts are described as "off until it is turned on" rather than "opt-in"), and
  no unit says a key can be turned off in `remote.toml` on its own (RDM12).

## 3. Plan Self-Check

Every claim was checked against the code: the Funnel refusal and its note (`feature_control.rs`,
`onboarding.rs`), the tours (`Docs/GUI_APPLICATION.md`, §2j "Feature tour in the app", §2k: both phone
tours switch nothing), the Android heartbeat (§2j "PC offline alerts", off until turned on in the app's
settings), the zbus pin (`Cargo.toml`, PR #417 title naming RUSTSEC-2026-0370, -0371 and -0372).

## 4. Tests

No test was added or changed: the edit is documentation only and the existing invariants already pin
the text that must survive (headings, "section 2j" in §8 and §9, the README wording guards). They were
run before and after the edit.

## 5. Auditor Constraints

- No sensitive data, no new commands, no new configuration keys.
- No wording that claims more than the code does (README claims stay verifiable, #356).

## 6. Changes

- `AI/VERIFICATION_MATRIX.md`: OBT9 prose; new component `docs-followups`, rows DFW1–DFW5.
- `README.md`: Phone companion row; the security off-switch paragraph (tours, the `[android]` Funnel
  rule, the two kinds of offline alerts).
- `AI/DECISIONS.md`: dated zbus 6 note in ADR 2026-10-02.
- `Docs/REMOTE_COMPANION.md`: the "section 2j" references in §2k, §5, §6, §8 and §9.

## 7. Verification

```bash
LIBONNXRUNTIME_NO_PKG_CONFIG=1 cargo test -p soos-invariants -- --test-threads=3
```
