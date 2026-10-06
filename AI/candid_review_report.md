# Candid Review Report

- **Date**: 2026-10-06
- **Target Branch**: `feat/remote-funnel-passkey`
- **Base (merge-base)**: `222665f0`
- **Reviewed-Diff-Fingerprint**: `5c99775b8601fca296d273f5aa4ce0fb0878cf3ede63189e598e214f64c725a0`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `AGENTS.md`, `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/MOCK_STRATEGY.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_remote_companion.md`, `AI/architect_spec_remote_passkey_funnel.md`, `AI/auditor_constraints_remote_companion.md`, `AI/auditor_constraints_remote_passkey_funnel.md`, `AI/research_funnel.md`, `AI/research_webauthn.md`, `AI/tester_contract_remote_companion.md`, `AI/tester_contract_remote_passkey_funnel.md`, `AI/walkthroughs/183_remote_companion.md`, `AI/walkthroughs/184_remote_unlock.md`, `AI/walkthroughs/185_remote_funnel_passkey.md`, `Cargo.lock`, `Cargo.toml`, `Docs/README.md`, `Docs/REMOTE_COMPANION.md`, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`, `crates/remote/Cargo.toml`, `crates/remote/assets/app.js`, `crates/remote/assets/apple-touch-icon.png`, `crates/remote/assets/icon.svg`, `crates/remote/assets/index.html`, `crates/remote/assets/manifest.webmanifest`, `crates/remote/assets/style.css`, `crates/remote/src/assets.rs`, `crates/remote/src/audit.rs`, `crates/remote/src/auth.rs`, `crates/remote/src/challenge.rs`, `crates/remote/src/config.rs`, `crates/remote/src/credentials.rs`, `crates/remote/src/enroll.rs`, `crates/remote/src/http.rs`, `crates/remote/src/identity.rs`, `crates/remote/src/lib.rs`, `crates/remote/src/logind.rs`, `crates/remote/src/main.rs`, `crates/remote/src/routes.rs`, `crates/remote/src/server.rs`, `crates/remote/src/session.rs`, `crates/remote/src/socket.rs`, `crates/remote/src/status.rs`, `crates/remote/src/webauthn.rs`, `crates/remote/src/websession.rs`, `crates/remote/tests/auth_capacity_tests.rs`, `crates/remote/tests/auth_server_tests.rs`, `crates/remote/tests/auth_store_tests.rs`, `crates/remote/tests/common/harness.rs`, `crates/remote/tests/common/passkey.rs`, `crates/remote/tests/config_tests.rs`, `crates/remote/tests/credentials_tests.rs`, `crates/remote/tests/enroll_tests.rs`, `crates/remote/tests/http_tests.proptest-regressions`, `crates/remote/tests/http_tests.rs`, `crates/remote/tests/identity_tests.rs`, `crates/remote/tests/routes_tests.rs`, `crates/remote/tests/server_tests.rs`, `crates/remote/tests/session_tests.rs`, `crates/remote/tests/socket_tests.rs`, `crates/remote/tests/webauthn_tests.rs`, `deny.toml`, `packaging/soos-remote.service`, `scripts/candid_review.sh`, `scripts/install_remote.sh`, `tests/invariants/src/lib.rs`, `tests/invariants/src/presence_unlock_contract.rs`, `tests/invariants/src/remote_companion_contract.rs`, `tests/invariants/src/remote_passkey_contract.rs`, ``

## 1. Executive Summary

The branch up to HEAD `3cedd5d` was approved under fingerprint `9c761e60…`. The only delta
(`git diff HEAD`, excluding this report) is five status cells in `AI/VERIFICATION_MATRIX.md`
(RMC25, RMC40, RMC42, RMC43, RMC44) recording the owner's 2026-10-06 hardware results. The
previous CHANGES_REQUESTED report on this delta asked to downgrade RMC25 and RMC40 to partly
verified and to remove the real host name; both were done. Every status was re-checked against
its row requirement, `Docs/REMOTE_COMPANION.md` and `crates/remote/src`. No status claims
more than its stated evidence. No production code, test, script or dependency changed.

## 2. Test Changes

No test file, inline test module, assertion, `#[ignore]` or escape hatch changed in the delta
(`git diff HEAD --stat` lists only `AI/VERIFICATION_MATRIX.md`, 5 insertions, 5 deletions).
The test changes in the frozen patch belong to the committed branch reviewed and approved under
`9c761e60…`.

## 3. Deep Reasoning Audit

### Logic & Architecture
- RMC42 (✅ Verified): requires (a) registration with the VPN on via `enroll-code` + Face ID,
  visible in `soos-remote passkeys list`, and (b) the same attempt over Funnel refused. (a) is
  stated as 1 record, `synced`, which is a real output label (`crates/remote/src/main.rs:238`).
  (b) is stated as an anonymous Funnel register-options request answering `403 forbidden`;
  `crates/remote/src/server.rs:1011-1012` refuses `RegisterOptions`/`RegisterVerify` on the
  Funnel class with `403 forbidden` before any session or code check, so the anonymous probe
  is the same refusal the page would hit. Both halves evidenced → Verified is justified. PASS.
- RMC40 (⬜ Pending, partly verified): `403 login_required` for status/lock/unlock without a
  session matches `server.rs:1013-1014`; `mode: funnel` / `mode: tailnet` match
  `server.rs:393-399`; a forged `Tailscale-User-Login` over Funnel is irrelevant because the
  Funnel class is refused before identity. The cell lists what is missing (4G, Safari and
  home-screen app separately). PASS.
- RMC43 (⬜ Pending, partly verified): sign-in and *Lock now* reported; Safari/home-screen
  separately, live events and *Sign out* listed as missing. PASS.
- RMC44 (⬜ Pending, partly verified): Funnel unlock reported; tailnet path missing. The note that
  registration framing is covered by RMC42 is accurate (RMC42 registration POSTs went through
  `tailscaled` on the tailnet path). PASS.
- RMC25 (⬜ Pending, automated ✅, hardware partly verified): dismissal of the lock screen and the
  `remote unlock requested` journal line reported (matches the audit line of the row); the
  page state `Unlocked` is explicitly missing. Consistent with `Docs/REMOTE_COMPANION.md`,
  which documents `swaylock-plugin` and the `Unlocked` state. PASS.
- Scenario tried: could any row be read as fully closed by a partial report? Only RMC42 is ✅ and
  its requirement is fully covered. PASS.

### PAM Concurrency & Deadlines
No PAM, daemon or IPC change in the delta. PASS (not applicable).

### Panic Safety & Fail-Closed
No code change. The recorded behavior (`403` refusals for unauthenticated Funnel callers)
confirms fail-closed in the field. PASS.

### Test Integrity & Anti-Weakening
No test touched; matrix evidence columns unchanged, only status cells. PASS.

### Memory, Bounds & Secrets
Scenario: does the delta leak the owner's host name, login, credential fingerprint or session id?
The cells use the `<rp_id>` placeholder, no login, no fingerprint, no tailnet name. The `.ts.net`
strings in the frozen patch are generic examples from the approved commits. PASS.

### Supply Chain & Automation
No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. PASS.

### English-Only Policy
All new text is English. PASS.

## 4. Detailed Findings & Action Items
- **[SUGGESTION]** `AI/VERIFICATION_MATRIX.md:2077` — the RMC40 status reads
  "(partly verified) 2026-10-06:" while RMC43/RMC44 read "(partly verified). Owner report
  2026-10-06:"; harmonising the wording is optional.

## 5. Final Verdict
**VERDICT: APPROVED**
