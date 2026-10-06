# Candid Review Report

- **Date**: 2026-10-06
- **Target Branch**: `feat/remote-funnel-passkey`
- **Base (merge-base)**: `222665f`
- **Reviewed-Diff-Fingerprint**: `67f5f087591ed3727b976818332a089b69cd5cf4182a881879f1151d3398c11e`
- **Audited Files**: full branch patch (`target/candid_diff.patch`); everything up to HEAD `6d779bf` was approved under the previous fingerprint `5c99775b…`. The only delta since then is the uncommitted change to `AI/VERIFICATION_MATRIX.md` (status cells of RMC25, RMC40, RMC43, RMC44; 4 lines changed).

## 1. Executive Summary

The delta records the owner's hardware report of 2026-10-06: the test ran on 4G with the Tailscale
VPN off, in the home-screen web app (not a Safari tab), and after *Unlock now* plus Face ID the page
showed `Unlocked`. Each updated status cell was checked against its row's acceptance criteria and
against this evidence. RMC25 is promoted to Verified; its hardware criterion (with `allow_unlock`
and a registered passkey, *Unlock now* and Face ID dismiss the lock screen and the page shows
`Unlocked`) is now fully covered. RMC40, RMC43 and RMC44 remain Pending (partly verified) and list
exactly the still-unreported items. No production code, test, script or dependency changed.

## 2. Test Changes

Mechanical listing on the delta: no test file, assertion, `#[ignore]`, `should_panic`, tolerance or
inline test module is touched (the delta is one Markdown file). The branch-level listing is
unchanged from the previously approved review.

## 3. Deep Reasoning Audit

### Logic & Architecture
- **RMC25** — criterion: hardware dismissal of the lock screen with Face ID **and** the page showing
  `Unlocked`. The previous cell left only the `Unlocked` state unreported; the owner now reports it.
  Confirmed `Unlocked` is a real page state (`crates/remote/assets/app.js:44` status label and
  `:274` feedback after an accepted unlock), so the claim is plausible and matches the code. The
  automated part was already verified. The "(tracked with RMC44)" note refers to the shared hardware
  check; RMC25's own hardware criterion does not require the tailnet path, so Verified while RMC44
  stays Pending is consistent. → PASS.
- **RMC40** — criterion requires Safari **and** the home-screen app, on 4G with the VPN off. The cell
  now credits 4G and the home-screen app and lists "a Safari tab" as unreported; the other items
  (port 443 only, Funnel-mode refusals, tailnet one-tap) were already evidenced in the approved
  version. Pending is correct. → PASS.
- **RMC43** — criterion: Safari and home-screen app sign-in, status, live events, *Lock now*,
  *Sign out*. The cell credits home-screen-app sign-in on 4G and *Lock now*, and lists Safari tab,
  live events and *Sign out* as unreported. Pending is correct. → PASS (see MINOR note on "status").
- **RMC44** — criterion: unlock both via tailnet and via Funnel; passkey POST framing. The cell
  credits the Funnel path in the home-screen app with `Unlocked` shown, lists the tailnet path as
  unreported, and defers registration framing to RMC42 (Verified). Claims no more than the evidence.
  → PASS.
- Attempted break: does any cell credit Safari-tab behaviour or the VPN-on unlock path from this
  report? No. Does any cell promote to Verified with an open criterion item? No.

### PAM Concurrency & Deadlines
No code change. → PASS (not applicable).

### Panic Safety & Fail-Closed
No code change. → PASS (not applicable).

### Test Integrity & Anti-Weakening
No test change; evidence columns unchanged. → PASS.

### Memory, Bounds & Secrets
The status cells contain no login, host name, credential id or other identifier (`<rp_id>` remains
a placeholder). → PASS.

### Supply Chain & Automation
No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or hook change in the delta. → PASS.

### English-Only Policy
All new text is English. → PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `AI/VERIFICATION_MATRIX.md` RMC43 — the criterion lists "status" among the items that
  must work after sign-in, but the cell neither credits nor lists it as unreported (pre-existing
  wording, not introduced by this delta). The `Unlocked` state seen in the RMC44 report indicates the
  page status works in a Funnel session; consider naming it explicitly when the row is closed. Does
  not affect the Pending status.

No CRITICAL or MAJOR findings.

## 5. Final Verdict

**VERDICT: APPROVED**
