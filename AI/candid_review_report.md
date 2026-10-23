# Candid Review Report

- **Date**: 2026-10-23
- **Target Branch**: `chore/android-owner-checks`
- **Base (merge-base)**: `e7826de`
- **Reviewed-Diff-Fingerprint**: `9ad38a754bb9aba8d395d881aaf35f67a97e5917c5a3f941b4e8289fc3a3d528`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/254_android_owner_checks.md`,
  `Docs/REMOTE_COMPANION.md`

## 1. Executive Summary

Documentation-only change recording the owner's Android checks of 2026-10-23 and the CI evidence
of the JVM-only Android rows. A first review of commit bb65d35 requested changes: AAF4 claimed its
untested expired-session clause, ATR9 and ANA14 rested on a general "everything works" while the
walkthrough presented specific observations, and the walkthrough claimed a `remote.toml`
troubleshooting row that did not exist. Commits 1515ad4 and 02c6dc0 fix all three. AAF4, ATR9 and
ANA14 are back to Pending, and their notes state exactly what was reported. ANA15 is also back to
Pending because the system bars were not confirmed. The missing troubleshooting row now exists.
No row now claims more than the owner reported or CI proved.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

None. No test, fixture, script or code file is touched.

## 3. Deep Reasoning Audit

### Logic & Architecture

- **CI rows** (ANA18, ANA19, ANA22, ANA23, ANA24, AAF1, AHB4, AHB5, AHB6, ATR1) are now ✅ and
  cite run 39622619945. Checked with `gh run view`: workflow `Android`, job `Android (debug build)`,
  push on `main`, head `fb2a0bf29d5d`, conclusion success. The log shows
  `> Task :app:testDebugUnitTest` executed and `BUILD SUCCESSFUL`. Nothing under `android/` changed
  between fb2a0bf and the merge-base. Every cited method (31 in total) exists in its cited file
  under `android/app/src/test/java/org/soos/remote/`. Every cited whole class also exists. PASS.
- **Owner rows**:
  - RAN12, RDB10, ANA26, AAF4, ANA15, ATR9 and ANA14 stay ⬜ Pending.
  - Their notes match the owner's report as relayed: RAN12 parts 1–2; the RDB10 Android part; the
    ANA26 battery card; the first AAF4 clause; the ANA15 icon, themed icon and header; and the
    "everything works" basis of ATR9 and ANA14, with no item-by-item report.
  - ANA16, ANA21 and AHB8 are unchanged.
  - PASS.
- **Troubleshooting rows**:
  - The `chrome://net-internals/#dns` and `#sockets` paths are valid.
  - The keytool row escapes its pipe (`\|`), so the table still renders.
  - The `sdkmanager` versions `platforms;android-36` and `build-tools;35.0.0` match
    `.github/workflows/android.yml`. `platform-tools` is now described as an addition for `adb`.
  - The `remote.toml` row quotes the preflight message of `scripts/setup.sh` (line 1581). Its
    `mv` path matches `${CONFIG_HOME}/soos/remote.toml`.
  - The `226/NAMESPACE` workaround does not lower the sysctl.
  - PASS.
- **Walkthrough 254**: the number is free (253 is the latest). Its §2 table and header match the
  matrix. The setup details (GNOME, Tailscale 1.104.1, laptop) come from the owner's pasted
  `setup.sh` output. PASS.

### PAM Concurrency & Deadlines

Not affected: no PAM code or configuration. PASS.

### Panic Safety & Fail-Closed

Not affected: no code. PASS.

### Test Integrity & Anti-Weakening

- No test changed.
- Status changes go only toward Pending where evidence is partial. ✅ is used only where a green
  CI run executed the cited tests.
- `cargo test --locked -p soos-invariants -- --test-threads=3`: 879 passed, 0 failed (matrix
  citations and structure).
- PASS.

### Memory, Bounds & Secrets

- No credential, key, token, address or biometric data is recorded.
- The troubleshooting rows recommend no weakening of the sandbox or of the AppArmor sysctl.
- PASS.

### Supply Chain & Automation

No workflow, dependency or script change. PASS.

### English-Only Policy

All added text is in English. The French `keytool` label (`SHA 256:`) is quoted as tool output
only. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `AI/VERIFICATION_MATRIX.md` (AAF4): the owner reported "on refresh" without
  separating *Refresh* from pull to refresh. Confirm both during the remaining owner check. The row
  is already Pending, so nothing is blocked.

No CRITICAL or MAJOR finding remains.

## 5. Final Verdict

**VERDICT: APPROVED**
