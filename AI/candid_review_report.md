# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `chore/readme-redesign` (PR #409, GitHub #408)
- **Base (merge-base)**: `6f89f05` (`origin/main` after `git fetch origin`, includes #418 and #419; `origin/main` is an ancestor of `HEAD`)
- **Reviewed-Diff-Fingerprint**: `eb8205d97ff806faef9d7c6e172fd00f513812dad125f5ac07feb78fe1b671a7`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_readme_redesign.md`,
  `AI/auditor_constraints_readme_redesign.md`, `AI/walkthroughs/216_readme_redesign.md`,
  `Docs/DEVELOPMENT_WORKFLOW.md`, `Docs/README.md`, `Docs/REMOTE_COMPANION.md`, `README.md`,
  `tests/invariants/src/lib.rs`, `tests/invariants/src/readme_contract.rs`

## 1. Executive Summary

Documentation-only change plus one new static invariant module (`readme_contract`, RDM1–RDM8,
RDM10–RDM13). No production code, PAM, daemon, protocol, dependency, CI or script change.
`cargo test --locked -p soos-invariants -- --test-threads=4` passes (770 passed, 0 failed),
including every pre-existing contract that pins README text (installer, onboarding, setup, upgrade,
licence, GDM). Every factual claim of `README.md` and of the edited lines of
`Docs/REMOTE_COMPANION.md`, `Docs/README.md`, `Docs/DEVELOPMENT_WORKFLOW.md` and `AI/DECISIONS.md`
was checked against the code and docs on `origin/main`; none was found false. The finding of the
previous review round (README claiming a `remote.toml` key can be turned off on its own) is fixed:
the security section now names every dependent key and the exit 78 refusal, matching
`crates/remote/src/config.rs`. Only two SUGGESTION-level items remain.

## 2. Test Changes (mechanical listing from step 3)

- Test files touched: `tests/invariants/src/lib.rs` (adds `mod readme_contract;` under
  `#[cfg(all(test, unix))]` with a doc comment, nothing removed) and the new
  `tests/invariants/src/readme_contract.rs`.
- Removed or changed assertions (`^-` with `assert`/`#[test]`/`proptest!`/`should_panic`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.
- No existing test on `origin/main` is edited or weakened; the fixture
  `tests/fixtures/setup/remote.toml` read by RDM10/RDM12 is unchanged (last touched by #402).

## 3. Deep Reasoning Audit

### Logic & Architecture
Claims verified against `origin/main` (scenario: try to find a sentence the code contradicts):
- **Setup defaults**: `scripts/setup.sh` (`COMPANION_KEYS`, the render summary and the fixture)
  writes `allow_funnel`, `allow_unlock`, `password_alerts`, `push_notifications`, `offline_alerts`,
  `camera_view`, `camera_view_funnel` = `true`. README "Why soos", Features row, Quick Start
  paragraph, Security model and FAQ all say the one-command setup turns them on. PASS.
- **Manual install and code defaults off**: the `scripts/install_remote.sh` template has every key
  commented `= false`; `config.rs` uses `unwrap_or(false)` for all seven (battery defaults to `true`
  but is not among the keys the README calls off). PASS.
- **Dependency refusals (exit 78)**: `config.rs` errors `PushRequiresAlerts`, `OfflineRequiresPush`,
  `CameraFunnelRequiresCameraView`, `CameraFunnelRequiresFunnel`, `AndroidRequiresFunnel`;
  `main.rs` maps configuration errors to `EXIT_CONFIG` = 78. The README list (`camera_view_funnel`
  with `allow_funnel` or `camera_view`; `push_notifications` and `offline_alerts` with
  `password_alerts`; `offline_alerts` with `push_notifications`; `[android]` needs `allow_funnel`)
  is complete for the boolean keys (the other prerequisites are `rp_id`, not a switch). PASS.
- **soos-gui switches**: `crates/gui/src/feature_control.rs` `RemoteSwitch` = Unlock, Funnel, Push,
  CameraView, OfflineAlerts; Funnel and CameraView also clear `camera_view_funnel`, Push also clears
  `offline_alerts`; there is no `password_alerts` switch. `onboarding.rs` maps the tour features onto
  these switches and the header **Features** button reopens the tour. README "all but the
  failed-password alerts can be turned off in soos-gui" and "turning Funnel or the camera view off
  there also sets `camera_view_funnel` to `false`" are exact. PASS.
- **Passkey requirements**: `Docs/REMOTE_COMPANION.md` §2a (every unlock, tailnet or Funnel, needs a
  fresh assertion with user verification), §2b (Funnel needs a passkey sign-in; tailnet status and
  lock need none), §2f and §6 (`/api/camera/start` takes an assertion). PASS.
- **Camera `[preview]`**: `setup.sh` creates `daemon.toml` with `[preview]` only when the file is
  absent and otherwise prints the administrator instruction; matches the FAQ. PASS.
- **Kill switch**: `crates/pam/src/config.rs` checks `<flag_dir>/disabled`; the daemon presence
  switch reads `GLOBAL_DISABLE_FLAG = "disabled"`. PASS.
- **Lock-screen widget**: `Docs/LOCKSCREEN_WIDGET.md` lists swaylock-plugin, hyprlock and GNOME
  Shell 45+ (#407), KDE Plasma later. PASS.
- **Merged-only features**: `gh pr list --state merged --limit 12` shows #403, #407, #413, #418
  (phone web app tour, `Docs/REMOTE_COMPANION.md` §2k) and #419 (Android PC-offline alerts) merged;
  network lock, evidence store, Android app (§2j) and battery (§2g) are documented on `main`. PASS.
- Other checks: Rust badge equals the `rust-toolchain.toml` channel `1.98.1`; `setup.sh` flags
  `--presence`, `--no-remote`, `--tailscale-login`, `--rp-id` exist; `wait_daemon_ready.sh` states
  the ~37.5 s worst case; PAD at 2.7× single scale (`Docs/VISION_CRATE.md`); IR exposure gate
  (`crates/vision/src/ir_liveness.rs`); the daemon does not drive the IR emitter
  (`Docs/CAMERA_V4L_CRATE.md`); `sensor_preference` default `PreferIr`; evidence 7-day retention and
  daily cap. PASS.
- **Edited section titles** in `Docs/REMOTE_COMPANION.md` (§2a, §2c, §2d, §2f, §2j): no repository
  link or test references the old `(opt-in)` anchors. PASS.
- **AI/DECISIONS.md**: both the three-dot and the two-dot diff against `origin/main` show exactly one
  appended line and zero removed lines (no duplicated or reverted ADR). PASS.

### PAM Concurrency & Deadlines
No file under `crates/pam` changed. N/A — PASS.

### Panic Safety & Fail-Closed
No production code changed. The README still states that only a fresh `Allow` becomes
`PAM_SUCCESS` and that every failure returns `PAM_IGNORE`, consistent with ARCHITECTURE §2. The new
code uses `expect`/`unwrap_or` only in test helpers. PASS.

### Test Integrity & Anti-Weakening
- RDM10 (`test_rdm_companion_wording_matches_setup_defaults`) fails on a plausible wrong README
  (e.g. "Remote unlock is off by default", a camera view over Tailscale without "Funnel", a security
  section missing `scripts/setup.sh`, `passkey`, `soos-gui`, `remote.toml` or any exposure key). Its
  early return when the fixture sets no key is guarded by RDM12, which asserts the fixture yields all
  four exposure keys, so RDM10 cannot pass vacuously. `section(…, "Security")` resolves to
  "Security model" (it precedes "Contributing and security"). PASS.
- RDM11 exercises `keys_set_true` (spacing, commented line, quoted `"true"` rejected), `text_units`
  (paragraph wrap, list items, numbered steps, table rows, fenced lines), `calls_exposure_opt_in`
  (positive and negative cases, including presence auto-unlock and evidence snapshots) and
  `camera_view_omits_funnel`. PASS.
- RDM12 (`test_rdm_companion_opt_out_names_dependent_keys`) asserts the fixture, rejects the known
  "independent opt-out" phrasings, and requires `exit 78`, `password_alerts`, `push_notifications`,
  `offline_alerts`, `camera_view_funnel` and `[android]` in the security section. RDM13 tests the
  helper. Sound, see SUGGESTION 2.
- No pre-existing assertion was modified. PASS.

### Memory, Bounds & Secrets
No runtime code. The README and docs contain no secret, real login or host name (examples use
`you@example.com`, `mypc.tail1234.ts.net`, user `alice`); screenshots stay synthetic. PASS.

### Supply Chain & Automation
No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. Badges point at the
repository's own workflows and shields.io. PASS.

### English-Only Policy
All added text (README, Docs, AI files, test code and messages) is English. PASS.

## 4. Detailed Findings & Action Items

1. **[SUGGESTION]** `AI/ARCHITECTURE.md:413` (outside the diff; also line 406) — the component table
   still labels "Unlock (opt-in)" and "Push sender (opt-in)", while the README and
   `Docs/REMOTE_COMPANION.md` now say "off by default, on after `scripts/setup.sh`". Correct for the
   code default, but a follow-up could align the wording with ADR 2026-10-15 item (3).
2. **[SUGGESTION]** `tests/invariants/src/readme_contract.rs:874` — `claims_independent_opt_out` is a
   denylist of four exact phrasings, so a reworded false claim (e.g. "you can set any key to
   `false`") would pass RDM12's first check. The needle check on the security section still catches
   the loss of the dependency list, so this is optional hardening only.

No CRITICAL, MAJOR or MINOR finding.

Note for the pusher (not a defect of the diff): the local branch is rebased (`ahead 12, behind 8`
of `origin/chore/readme-redesign`), so the push to PR #409 needs `--force-with-lease`.

## 5. Final Verdict

**VERDICT: APPROVED**
