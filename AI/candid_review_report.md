# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `chore/readme-redesign` (PR #409, GitHub #408, PR 2 of #356)
- **Base (merge-base)**: `e216ac1` (current `origin/main` after `git fetch origin`; `origin/main` is an ancestor of `HEAD`)
- **Reviewed-Diff-Fingerprint**: `6999c5c4ec5a61b3cebf24751911a1d021dfcab51f59779b19d77f462fb40c25`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_readme_redesign.md`,
  `AI/auditor_constraints_readme_redesign.md`, `AI/walkthroughs/216_readme_redesign.md`,
  `Docs/DEVELOPMENT_WORKFLOW.md`, `Docs/README.md`, `Docs/REMOTE_COMPANION.md`, `README.md`,
  `tests/invariants/src/lib.rs`, `tests/invariants/src/readme_contract.rs`

## 1. Executive Summary

Documentation-only change plus one new invariant test module. The README becomes a landing page;
every factual claim was checked against `origin/main` code and docs (setup defaults, code defaults,
manual installer template, `soos-gui` switches, passkey requirements, kill switch, lock-screen widget
support, merged features, model attestation, evidence retention, template encryption, daemon
sandbox). No false claim was found. The new RDM10/RDM11 tests are sound and fail on the plausible
regressions they target; no existing assertion was touched. `AI/DECISIONS.md` gains exactly one
appended ADR line, nothing removed or duplicated. `cargo test --locked -p soos-invariants`:
756 passed, 0 failed. Only MINOR and SUGGESTION findings remain.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/lib.rs` (adds `mod readme_contract;` under
  `#[cfg(all(test, unix))]`), `tests/invariants/src/readme_contract.rs` (new, 853 lines).
- Removed or changed assertions (`^-.*assert|#[test]|proptest!|should_panic`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance/epsilon): **none**.
- Inline `mod tests` changes: **none**.

No existing test of `origin/main` was edited, weakened or removed; the README edits keep every
pinned string of the existing installer, onboarding, setup, upgrade, licence and GDM contracts
(the full `soos-invariants` suite passes).

## 3. Deep Reasoning Audit

### Logic & Architecture

Claims checked against `origin/main`:

- **Setup defaults.** `scripts/setup.sh` `COMPANION_KEYS` = allow_funnel, allow_unlock,
  password_alerts, push_notifications, offline_alerts, camera_view, camera_view_funnel; the render
  fixture `tests/fixtures/setup/remote.toml` sets all seven to `true`. README L37-40, L52, L56,
  L96-98, L288-298 and FAQ L349-355 match. PASS.
- **Code defaults / manual install.** `crates/remote/src/config.rs`: every one of those keys
  `unwrap_or(false)`; `battery_status` defaults to `true` (README lists battery level as a feature
  and never calls it off). `scripts/install_remote.sh` writes all of them commented as `false`.
  "A manual install ... and the code defaults leave all of them off" holds. PASS.
- **soos-gui switches.** `crates/gui/src/feature_control.rs` `RemoteSwitch` = Unlock, Funnel, Push,
  CameraView, OfflineAlerts, all wired in `crates/gui/src/onboarding.rs` L342-346; no
  `password_alerts` switch. `keys_to_clear`: Funnel -> `allow_funnel`, `camera_view_funnel`;
  CameraView -> `camera_view`, `camera_view_funnel`. README "all but the failed-password alerts
  also in soos-gui" and "turning Funnel or the camera view off there also sets
  `camera_view_funnel` to `false`" are exact. The **Features** header button exists (`header.rs`,
  `app.rs` L723). PASS.
- **Passkeys.** `Docs/REMOTE_COMPANION.md` L13 (Funnel behind passkey login), L142-146 (every
  unlock, tailnet and Funnel, fresh assertion with user verification), L635 (fresh passkey check
  per camera view), L933 (lock never depends on passkeys). README L292-294 matches. PASS.
- **Kill switch.** `crates/pam/src/config.rs` L120 (`disabled` flag) and
  `crates/daemon/src/presence/switch.rs` (`GLOBAL_DISABLE_FLAG = "disabled"` stops presence).
  PASS.
- **Lock-screen widget.** `Docs/LOCKSCREEN_WIDGET.md` table: swaylock-plugin, hyprlock, GNOME
  Shell 45+ (#407 merged as `e216ac1`), KDE Plasma "later PR". PASS.
- **Merged features only.** `gh pr list --state merged`: #419 (Android opt-in offline heartbeat),
  #413, #407, #403, #402 are all on `origin/main`. Android row "opt-in PC offline alerts" matches
  #419. PASS.
- Other claims spot-checked: `PrivateNetwork=yes` (`packaging/soos-daemon.service` L81), SHA-256 and
  tensor-shape attestation (`models/README.md` L59-63), 7-day retention and daily cap
  (`Docs/EVIDENCE_STORE_CRATE.md` L14-15), AES-256-GCM templates, `PreferIr` default and no emitter
  control (`Docs/CAMERA_V4L_CRATE.md`), IR gate (`crates/vision/src/ir_liveness.rs`), single 2.7x
  crop, screenshot source commit `a3cae16` (#392, last change to `Docs/assets/screenshots`). PASS.
- **Docs edits.** `Docs/REMOTE_COMPANION.md` headings "off by default, on after
  `scripts/setup.sh`" for 2a/2c/2d/2f/2j and the Funnel paragraph agree with config.rs and setup.sh;
  `Docs/README.md` and `Docs/DEVELOPMENT_WORKFLOW.md` §10 list the same section order as the README.
  PASS, with finding 1 (stale ADR sentence) and suggestion 4.

### PAM Concurrency & Deadlines

No production code changed. The README restates the deadline (`timeout_ms`, default 1000 ms) and
"no Tokio" correctly. PASS (not applicable).

### Panic Safety & Fail-Closed

No production code changed. The README never implies an error path leads to `PAM_SUCCESS`
("Only a fresh `Allow` for your UID becomes `PAM_SUCCESS`"). The test module uses `expect` /
`unwrap_or` only in test code. PASS.

### Test Integrity & Anti-Weakening

- RDM10 scenarios: replacing README L56 with "remote unlock (opt-in)" or the old diagram label
  "opt-in unlock" -> `calls_exposure_opt_in` flags it (helper asserts cover both). Dropping
  "Funnel" from the FAQ camera paragraph -> `camera_view_omits_funnel` flags it. Removing
  `remote.toml`, `soos-gui`, `passkey` or any of the four keys from the security section -> needle
  assertion fails. The test fails against plausible wrong text. PASS.
- `auto-unlock` is stripped before matching, so "presence auto-unlock ... opt-in" does not
  false-positive; the snapshot and polkit opt-in lines are correctly excluded (helper asserts). PASS.
- The early `return` when the fixture sets no exposure key is by design (the wording rule follows
  the defaults); the fixture is byte-pinned to the setup render by `setup_user_phase_contract` and
  `crates/remote/tests/setup_render_tests.rs`, so it cannot drift silently. See suggestion 3.
- Coverage gap: RDM10 does not check the alert keys; see suggestion 2.

### Memory, Bounds & Secrets

The test reads repository files only. The README states no password, frame or embedding ever
crosses IPC or logs, consistent with the invariants. No secret, real host name or real login
appears (examples use `example.com` and `tail1234.ts.net`). PASS.

### Supply Chain & Automation

No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or hook change. Badges point to the existing
workflows `ci.yml` and `fuzz.yml` (RDM3). PASS.

### English-Only Policy

All added text is English. PASS.

## 4. Detailed Findings & Action Items

1. **[MINOR]** `AI/DECISIONS.md:149` (item (2) of the new ADR) — states that "the `setup.sh`
   paragraph stays byte-identical so the open PR #402 rebases without conflict", but the branch
   changed that paragraph (README L97-98: "each can be turned off later in `remote.toml`, and all
   but the failed-password alerts in soos-gui"; the change itself is correct, since the old text
   implied every feature had a soos-gui switch), and #402 is already merged. Reword item (2): the
   paragraph keeps its pinned substrings and was corrected for the soos-gui switch list.
2. **[SUGGESTION]** `tests/invariants/src/readme_contract.rs:681` — `COMPANION_EXPOSURE_KEYS`
   covers only Funnel, unlock and the camera view; setup also turns `password_alerts`,
   `push_notifications` and `offline_alerts` on, yet a README sentence calling the failed-password
   or push alerts "opt-in" would pass, although `Docs/DEVELOPMENT_WORKFLOW.md` §10 asks the README
   to describe all companion defaults as setup writes them. Optional follow-up as a new test.
3. **[SUGGESTION]** `tests/invariants/src/readme_contract.rs:692` — `keys_set_true` matches only
   `key = true` with nothing after the value; if setup ever wrote an inline comment, RDM10 would
   return early and pass vacuously. Consider asserting that the fixture yields at least one key
   while `scripts/setup.sh` lists them in `COMPANION_KEYS`.
4. **[SUGGESTION]** `Docs/REMOTE_COMPANION.md:123` (also 259, 350, 595, 986) — "on after
   `scripts/setup.sh`" holds only when setup runs the companion phase (Tailscale login and `rp_id`
   given, no `--no-remote`); the camera view also needs the daemon `[preview]` section, which setup
   creates only when `daemon.toml` is absent (the README FAQ already says so). A short qualifier
   would make this explicit.
5. **[SUGGESTION]** `README.md:98` — the reworded line is about 150 characters, wider than the
   surrounding wrap; cosmetic only.

## 5. Final Verdict

No CRITICAL or MAJOR finding. Every README claim checked matches `origin/main`; the new tests are
sound and no existing test changed; the ADR register only gains one appended line.

**VERDICT: APPROVED**
