# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `chore/readme-redesign` (PR #409, GitHub #408)
- **Base (merge-base)**: `6f89f05` (`origin/main`, includes #418; `origin/main` is an ancestor of `HEAD`)
- **Reviewed-Diff-Fingerprint**: `853dd2f479524dec32af24dd1082c64d5b3e309d846c7e466c50b77d2acf7294`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_readme_redesign.md`,
  `AI/auditor_constraints_readme_redesign.md`, `AI/walkthroughs/216_readme_redesign.md`,
  `Docs/DEVELOPMENT_WORKFLOW.md`, `Docs/README.md`, `Docs/REMOTE_COMPANION.md`, `README.md`,
  `tests/invariants/src/lib.rs`, `tests/invariants/src/readme_contract.rs`

## 1. Executive Summary

Documentation-only change plus a new static invariant module. No production code, no PAM, daemon,
protocol or dependency change. `cargo test --locked -p soos-invariants` passes (768 passed, 0 failed),
including all ten `readme_contract` tests. Almost every factual claim checked out against `origin/main`.
One user-facing instruction is wrong: the README (three places) and `Docs/DEVELOPMENT_WORKFLOW.md`
say that each companion key can be set to `false` in `remote.toml` on its own. With the setup defaults
that makes `soos-remote` refuse to start (exit 78) for `allow_funnel`, `camera_view` and
`password_alerts`, which contradicts the off-path rule that #418 merged on `main`. Two minor items
follow.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/lib.rs` (adds `mod readme_contract;` only) and
  `tests/invariants/src/readme_contract.rs` (new file).
- Removed or changed assertions (`^-` lines with `assert`, `#[test]`, ...): **none**.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance or epsilon): **none**.
- Inline `mod tests` changes: **none**.

No existing assertion on `origin/main` was edited or weakened. The headings renamed in
`Docs/REMOTE_COMPANION.md` (§2a, §2c, §2d, §2f, §2j) are not pinned by any test or anchor link
(`git grep` found no `REMOTE_COMPANION.md#` anchors, and only walkthroughs mention the old titles).

## 3. Deep Reasoning Audit

### Logic & Architecture
Scenarios attempted, each checked against the code or docs on `origin/main`:
- **Setup defaults.** `scripts/setup.sh` `COMPANION_KEYS` and `tests/fixtures/setup/remote.toml` set
  all seven keys to `true`. The README says that setup turns Funnel, unlock, the camera view (on the
  tailnet and over Funnel) and the three alerts on. PASS.
- **Code and manual-install defaults.** `crates/remote/src/config.rs` defaults every key to `false`
  (`battery_status` excepted). The `scripts/install_remote.sh` template leaves every key commented out
  as `false`. The README says "a manual install and the code defaults leave all of them off". PASS.
- **soos-gui switches.** `RemoteSwitch` in `crates/gui/src/feature_control.rs` has Unlock, Funnel,
  Push, CameraView and OfflineAlerts, and none for `password_alerts`. `keys_to_clear`: Funnel clears
  `allow_funnel` and `camera_view_funnel`, and CameraView clears `camera_view` and
  `camera_view_funnel`. In `onboarding.rs` these switches sit in the tour, which the header
  **Features** button opens. The README says the failed-password alerts are the only feature without
  a switch, and that Funnel or camera-view off also clears `camera_view_funnel`. PASS.
- **Passkeys.** `Docs/REMOTE_COMPANION.md` L48–49 and L143 say that every unlock needs a fresh passkey
  assertion with user verification, on the tailnet as over Funnel. L57–59 say the camera view needs a
  fresh passkey assertion, and §2b says a Funnel request needs a web session from a passkey sign-in.
  On the tailnet, status and lock need no passkey. PASS.
- **Remote-toml off path.** The README says each key can be set to `false` in `remote.toml` on its own.
  `config.rs` refuses three cases: `camera_view_funnel` without `camera_view` or without
  `allow_funnel` (`CameraFunnelRequires*`), and `push_notifications` without `password_alerts`
  (`PushRequiresAlerts`). **FINDING 1.**
- **Kill switch.** `/etc/soos/disabled` is `GLOBAL_DISABLE_FLAG`, which the presence worker reads
  (`crates/daemon/src/presence/switch.rs`, `Docs/DAEMON.md` L381). It stops face PAM and presence at
  once. PASS.
- **Lock-screen widget.** `Docs/LOCKSCREEN_WIDGET.md` L24–26 list swaylock-plugin, hyprlock and
  GNOME Shell 45 or later, with KDE Plasma in a later PR. PASS.
- **Merged features only.** #403, #407 (GNOME), #402 (setup user phase), #418 (web app tour) and #419
  (Android offline heartbeat) are all merged (`gh pr list --state merged`). The IR emitter PR #393 is
  merged, but `Docs/CAMERA_V4L_CRATE.md` L561 says the daemon still does not drive the emitter, so the
  FAQ sentence is accurate. PASS.
- **`[preview]` creation.** The FAQ claim matches ADR 2026-10-15 item (7): setup creates the section
  only when `daemon.toml` is absent. PASS.
- **Wording of the §2b edit.** The edited sentence now chains "turns Funnel on (…section 4): with
  `allow_funnel` absent…". The colon clause belonged to the tailnet-only clause and now reads as if it
  explained setup. Section 4 also never says "no VPN on the phone". **FINDING 3.**
- **ADR register.** The `AI/DECISIONS.md` diff against `origin/main` adds exactly one line, appended
  after the #414 ADR, with no removed, duplicated or reverted lines. Walkthrough number 216 is unused
  on `main`. PASS.

### PAM Concurrency & Deadlines
No change to `crates/pam`. N/A, PASS.

### Panic Safety & Fail-Closed
No production code. The test helpers use `unwrap_or("")` and `expect` only inside `#[cfg(test)]`
invariants. PASS.

### Test Integrity & Anti-Weakening
- **RDM10 against bad wording.** I mentally re-introduced "remote unlock (opt-in)", "off by default
  camera view" and "camera view through your Tailscale network". `calls_exposure_opt_in` and
  `camera_view_omits_funnel` catch all three. The removal of `auto-unlock` and the exposure words
  prevent false positives on the presence, network-lock and snapshot rows. PASS.
- **Fixture dependency.** RDM10 reads the fixture that `setup_user_phase_contract.rs` L1630 pins
  byte-equal to the setup render, so the wording check follows the defaults. PASS.
- **RDM11 helpers.** The tests cover both polarities of each helper. PASS.
- **Vacuous pass.** RDM10 returns early when `keys_set_true` finds nothing. `keys_set_true` accepts
  only an exact `key = true` line. If the fixture gained an inline comment (`allow_funnel = true #
  ...`) or other whitespace, the test would silently pass instead of failing. **FINDING 2.**
- **Missing check.** No test catches the remote.toml off-path inaccuracy of Finding 1.

### Memory, Bounds & Secrets
No secrets, frames or embeddings. The README uses example identities only (`you@example.com`,
`mypc.tail1234.ts.net`, `alice`). PASS.

### Supply Chain & Automation
No change to `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/`. PASS.

### English-Only Policy
All added text is English. PASS.

## 4. Detailed Findings & Action Items

1. **[MAJOR]** `README.md:297` (also `README.md:56`, `README.md:98`, `README.md:355`,
   `Docs/DEVELOPMENT_WORKFLOW.md:261-262`). The text says that every companion key can be set to
   `false` in `remote.toml` ("each can be turned off ... in `remote.toml`", "Every key can be set to
   `false`", "you can turn it off in `soos-gui` or `remote.toml`").
   - **Failure.** On a setup-written file (all seven keys `true`), the user sets only
     `allow_funnel = false` to close internet exposure, or only `camera_view = false`, or only
     `password_alerts = false`. `soos-remote` then refuses its configuration
     (`CameraFunnelRequiresFunnel`, `CameraFunnelRequiresCameraView` or `PushRequiresAlerts`, exit 78),
     and the companion stops.
   - **Conflict with main.** This contradicts the off-path rule of ADR 2026-10-17 "Phone Onboarding
     Tour" (#414/#418) item (1) on `main`: "the exact `remote.toml` key together with every key that
     depends on it, so a manual edit never makes `soos-remote` exit 78".
   - **Required correction.** State that a key is set to `false` together with the keys that depend on
     it: `camera_view_funnel` with `allow_funnel` or `camera_view`, `push_notifications` and
     `offline_alerts` with `password_alerts`, and `offline_alerts` with `push_notifications`.
     Alternatively, point to the soos-gui switches, which clear the dependent keys, and to
     `Docs/REMOTE_COMPANION.md` §2k/§5. Extend RDM10 so that a "set to `false` in `remote.toml`" claim
     must name the dependency or link it.
2. **[MINOR]** `tests/invariants/src/readme_contract.rs:775` (`keys_set_true` L692–704). RDM10 returns
   early and passes when no exposure key matches the strict `key = true` line form. A fixture
   reformatted with an inline comment or quoting would make the whole wording check silently vacuous.
   Make the empty case an assertion: either fail when the fixture contains none of the four keys at
   all, or parse the fixture with `toml` as `parse_remote_keys` does.
3. **[MINOR]** `Docs/REMOTE_COMPANION.md:176`. The inserted clause leaves the colon before "with
   `allow_funnel` absent or `false` every Funnel request is `403`" attached to the setup sentence, so
   the paragraph reads awkwardly. The cross-reference "no VPN on the phone, section 4" points to a
   section that does not contain that wording. End the setup clause with a full stop, restore the
   colon after the tailnet-only clause, and cite ADR 2026-10-15 item (3) instead of section 4.

## 5. Final Verdict

**VERDICT: CHANGES_REQUESTED**
