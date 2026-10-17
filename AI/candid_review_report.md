# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `chore/readme-redesign` (PR #409, GitHub #408)
- **Base (merge-base)**: `27d1e7c` (= `origin/main` after `git fetch origin`, includes #406, #407, #418, #419; `origin/main` is an ancestor of `HEAD`)
- **Reviewed-Diff-Fingerprint**: `cdf548d17ec6a3864ff6fa04b56cd2354532c667c03d92adad82d373ab176bc7`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_readme_redesign.md`, `AI/auditor_constraints_readme_redesign.md`, `AI/walkthroughs/216_readme_redesign.md`, `Docs/DEVELOPMENT_WORKFLOW.md`, `Docs/README.md`, `Docs/REMOTE_COMPANION.md`, `README.md`, `tests/invariants/src/lib.rs`, `tests/invariants/src/readme_contract.rs`

## 1. Executive Summary

Documentation-only change plus one new static invariant module. No production code, no
`Cargo.*`, no workflow or script change. Every factual claim of the rewritten `README.md` and of
the edited lines of `Docs/REMOTE_COMPANION.md`, `Docs/README.md`, `Docs/DEVELOPMENT_WORKFLOW.md`
and `AI/DECISIONS.md` was checked against `origin/main` (`27d1e7c`). No claim was found false.
The companion wording matches `scripts/setup.sh` (all seven keys written `true`, fixture
`tests/fixtures/setup/remote.toml`), the code defaults and dependency checks of
`crates/remote/src/config.rs` (exit 78 = `EXIT_CONFIG`), `scripts/install_remote.sh` (all keys
commented `false`) and the `soos-gui` switches (`RemoteSwitch` in
`crates/gui/src/feature_control.rs`, routed in `crates/gui/src/onboarding.rs`). Every listed
feature is merged on `origin/main`. `cargo test --locked -p soos-invariants` passes: 771 passed,
0 failed, the four new `test_rdm_companion_*` tests included. Only SUGGESTION-level items remain.

## 2. Test Changes (mechanical listing from step 3)

- Test files touched: `tests/invariants/src/lib.rs` (adds `mod readme_contract;` under
  `#[cfg(all(test, unix))]`, nothing removed), `tests/invariants/src/readme_contract.rs` (new file).
- Removed or changed assertions (`^-` with assert/#[test]/proptest/should_panic): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance/epsilon): **none**.
- Inline `mod tests` changes: **none**.
- No existing test on `origin/main` was edited. The old README sentence "(each can be turned off
  later in soos-gui)" from #402 was replaced. No existing contract pinned it: the
  setup/installer/onboarding/upgrade/licence README contracts all pass unchanged.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Phone companion vs `scripts/setup.sh`: `COMPANION_KEYS` = `allow_funnel allow_unlock
  password_alerts push_notifications offline_alerts camera_view camera_view_funnel`, all `true`
  in the render fixture. README L37-40, L56, L96-100, L290-303 and FAQ L352-360 say setup turns
  them on. PASS.
- Code defaults: every key is `unwrap_or(false)` in `config.rs`, and `install_remote.sh` comments
  them as `= false`. "A manual install and the code defaults leave all of them off" is correct. PASS.
- Dependency list (README L299-302) vs `config.rs`: `CameraFunnelRequiresCameraView`,
  `CameraFunnelRequiresFunnel`, `PushRequiresAlerts`, `OfflineRequiresPush`,
  `AndroidRequiresFunnel`. "`offline_alerts` with `password_alerts`" is transitively correct
  (offline needs push, push needs alerts). Exit 78 = `EXIT_CONFIG` (`crates/remote/src/lib.rs:139`). PASS.
- soos-gui switches: `RemoteSwitch` = Unlock, Funnel, Push, CameraView, OfflineAlerts, with no
  `password_alerts` switch. Funnel clears `camera_view_funnel`, and so does CameraView. "All but
  the failed-password alerts" and "turning Funnel or the camera view off also sets
  `camera_view_funnel` to false" are both correct. The **Features** header button exists
  (`header.rs`, `app.rs:723`). PASS (see S-1 for an under-statement).
- Passkey claims vs `Docs/REMOTE_COMPANION.md` §2b, §2f and `AI/ARCHITECTURE.md`: a Funnel
  session needs a passkey sign-in, every unlock needs a fresh UV assertion "on every path", and
  every camera view starts with a fresh UV assertion on the tailnet and over Funnel. On the
  tailnet, status and lock need no passkey. PASS.
- Native Android app (§2j "Native Android app", the second §2j heading, which predates this
  branch): status, lock, Funnel passkey sign-in, failed-password alerts with acknowledgement
  (#406), and offline alerts when turned on (#419). Unlock and the camera view stay in the web
  app. PASS.
- Phone tour §2k (#418) is listed as "a first-run feature tour in the web app". PASS.
- Kill switch: `crates/pam/src/config.rs:120` (`flag_dir.join("disabled")`) and
  `crates/daemon/src/presence/switch.rs` (`GLOBAL_DISABLE_FLAG`) both read it. PASS.
- Lock-screen widget: swaylock-plugin, hyprlock and GNOME Shell 45 or later (#407, `LOCKSCREEN_WIDGET.md` L24-26), KDE later. PASS.
- Merged-only rule: `gh pr list --state merged --limit 12` and `git log origin/main` contain
  #402, #403, #406, #407, #413, #418 and #419. No unmerged feature is listed. PASS.
- Other spot checks: the badge workflows `ci.yml`/`fuzz.yml` exist; the Rust 1.98.1 badge matches
  `rust-toolchain.toml`; the evidence 7-day retention and daily cap; `sensor_preference`; the
  uncalibrated IR PAD threshold (ADR 2026-09-30); the 2.7x single-scale PAD and #212; model
  attestation stops the start (`models/README.md` L61); setup creates the `[preview]` section
  only when `daemon.toml` is absent. PASS.
- `Docs/REMOTE_COMPANION.md` heading renames (`(opt-in)` → `(off by default, on after
  scripts/setup.sh)`): no repository link or test targets the old anchors. PASS.
- `AI/DECISIONS.md`: exactly one appended line (`+` 1, `-` 0) and no duplicated or reverted line
  compared with `origin/main`. PASS.

### PAM Concurrency & Deadlines
No change under `crates/pam` or `crates/daemon`. The README statements (`timeout_ms` default
1000 ms, no Tokio, `PAM_IGNORE` fallback) match `AGENTS.md` and the ADR. PASS.

### Panic Safety & Fail-Closed
No production code. `expect`/`unwrap_or` appear only in test code. The README never states a
path to `PAM_SUCCESS` other than a fresh `Allow`. PASS.

### Test Integrity & Anti-Weakening
- RDM10 returns early if the fixture sets no exposure key. RDM12 asserts the fixture yields
  exactly the four keys, which closes that bypass. Tried: emptying the fixture makes RDM12 fail.
  PASS.
- RDM10 can fail against a plausible wrong README: re-inserting "opt-in unlock" in the Mermaid
  node or "Remote unlock is off by default" is caught (helper cases in RDM11). PASS.
- `section(&readme, "Security")` returns the first `## ` heading containing "security", which is
  "Security model" (it precedes "Contributing and security"). PASS.
- RDM12's `claims_independent_opt_out` is a phrase denylist, so a new wording such as "any key can
  be set to false" would slip through. This is acceptable as a regression guard, and the
  positive needle check (exit 78 and the dependent keys) carries the contract (S-3).
- Test run: 771 passed, 0 failed.

### Memory, Bounds & Secrets
No runtime code. The README examples use synthetic values (`alice`, `you@example.com`,
`mypc.tail1234.ts.net`). No secret, real host name or face image is added. PASS.

### Supply Chain & Automation
No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. PASS.

### English-Only Policy
All added text is English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION] S-1** `README.md:296` — "Funnel, remote unlock and the camera view each have an
  off switch in `soos-gui`" is true but omits that push and offline alerts also have switches
  (`RemoteSwitch::Push`, `RemoteSwitch::OfflineAlerts`). The feature table at L56 states it
  correctly ("all but the failed-password alerts"). Optionally add "push and offline alerts" to
  L296.
- **[SUGGESTION] S-2** `Docs/REMOTE_COMPANION.md:200` (unedited line) — the §2b example comment
  `allow_funnel = true  # opt-in; false (the default) ...` and `AI/ARCHITECTURE.md:406,413`
  ("Push sender (opt-in)", "Unlock (opt-in)") still use "opt-in" for keys that setup turns on.
  They describe the code default correctly. Align them in a follow-up if desired.
- **[SUGGESTION] S-3** `tests/invariants/src/readme_contract.rs:874` — `claims_independent_opt_out`
  matches four fixed phrasings only. Consider a broader pattern (e.g. "can be set to `false`"
  without "depend") if the README is rewritten again.
- **[SUGGESTION] S-4** (outside the diff) `scripts/setup.sh` `--help` says every companion feature
  "can be turned off later in soos-gui (Features)", which is not true for `password_alerts`. The
  README is now more accurate than the script help. File it as a follow-up.

No CRITICAL, MAJOR or MINOR findings.

## 5. Final Verdict

**VERDICT: APPROVED**
