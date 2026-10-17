# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `chore/readme-redesign` (PR #409, GitHub #408)
- **Base (merge-base)**: `f1e7616` (`origin/main` has since advanced to `e216ac1`, #407 merged at 2026-10-17T20:07:30Z during this review)
- **Reviewed-Diff-Fingerprint**: `6b0c6263006cbef0db60c5412f48e3569f5ea67c7941dd05c230df4f97138ea9`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_readme_redesign.md`,
  `AI/auditor_constraints_readme_redesign.md`, `AI/walkthroughs/216_readme_redesign.md`,
  `Docs/DEVELOPMENT_WORKFLOW.md`, `Docs/README.md`, `README.md`, `tests/invariants/src/lib.rs`,
  `tests/invariants/src/readme_contract.rs`

## 1. Executive Summary

Documentation-only change plus one new std-only invariant module (`readme_contract`, RDM1–RDM8,
RDM10, RDM11). No production code, no PAM, IPC, daemon or supply-chain change. Every README claim
was checked against `origin/main`. The phone companion wording matches `scripts/setup.sh`
(`COMPANION_KEYS`, lines 78–90), the render fixture `tests/fixtures/setup/remote.toml`, the code
defaults in `crates/remote/src/config.rs` (all `unwrap_or(false)`), the commented-out `false`
template of `scripts/install_remote.sh`, the `RemoteSwitch::keys_cleared` table of
`crates/gui/src/feature_control.rs:114-121` and `Docs/REMOTE_COMPANION.md` §2a, §2b and §2f. `cargo
test --locked -p soos-invariants` passes (745 passed, 0 failed).

However `origin/main` moved during the review: PR #407 (GNOME Shell unlock-dialog extension) was
merged. Because of that the README now states that a merged feature "comes later", and the branch no
longer merges cleanly into `origin/main` (`AI/DECISIONS.md`, `AI/candid_review_report.md`). A rebase,
a README fix and a new review are needed.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/lib.rs` (adds `mod readme_contract;` under
  `#[cfg(all(test, unix))]`, nothing removed), `tests/invariants/src/readme_contract.rs` (new file).
- Removed or changed assertions (`^-` with `assert`/`#[test]`/`proptest!`/`should_panic`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` added or removed: **none**.

No existing assertion on `origin/main` was edited or weakened. The setup, installer, onboarding,
upgrade, licence and GDM README contracts run unchanged and pass on the new README.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Companion defaults: `setup.sh` writes `allow_funnel`, `allow_unlock`, `password_alerts`,
  `push_notifications`, `offline_alerts`, `camera_view`, `camera_view_funnel = true` (fixture and
  `setup.sh:2283-2284`). README lines 36–39, 55, 287–297 and 348–354 say so. Code defaults (`config.rs`
  lines 611–612, 668 and the doc comments 106, 123, 125, 176, 201) and `install_remote.sh:103-142` are
  all off, as the README says → PASS.
- soos-gui switches: `RemoteSwitch` = Unlock, Funnel, Push, CameraView, OfflineAlerts; there is no
  `password_alerts` switch; Funnel and CameraView also clear `camera_view_funnel`. README line 55 ("all
  but the failed-password alerts also in `soos-gui`") and lines 293–295 match. The switches only turn
  features off (`DisableRemote`), which matches "off switch". The **Features** header button exists
  (`header.rs:393-416`, `onboarding.rs:607`) → PASS. README lines 96–97 disagree with this → finding 3.
- Passkeys: every unlock needs a fresh UV assertion on the tailnet and over Funnel
  (`REMOTE_COMPANION.md:142-146`), every camera view starts with one (`:634-635`), Funnel needs a
  passkey sign-in (`:224-226`), and on the tailnet status and lock need no passkey → PASS.
- `[preview]` is created only when `daemon.toml` is absent (`setup.sh:194-195, 960-962, 1350`) → PASS.
- Merged-only features: #419 (Android PC-offline alerts) is merged and listed → PASS. #407 (GNOME
  Shell extension) is now **MERGED** but described as coming later → finding 1.
- Other claims checked: `PrivateNetwork=yes` (`packaging/soos-daemon.service:81`), Rust 1.98.1,
  the `fuzz.yml` workflow, commit `a3cae16`, #392 merged, the three screenshots, the 3 s lock grace
  (`lock_grace_ms = 3000`), the PAD scale 2.7 and #212, the stricter, uncalibrated IR threshold and
  exposure gate (`ir_liveness.rs:13-21`), the 7-day retention and daily cap of the evidence store,
  `sensor_preference` and the emitter not being driven, `soos-notify`, `with-faillock`, the 30 s /
  37.5 s wait, and the titles and open state of the roadmap epics #352–#363 → PASS.
- Kill switch: `/etc/soos/disabled` stops face PAM and presence, but `crates/remote` never reads it,
  so remote unlock still works → finding 4.

### PAM Concurrency & Deadlines
No `crates/pam` change. The README describes the deadline as `timeout_ms`, default 1000 ms, which
matches the ADR → PASS.

### Panic Safety & Fail-Closed
No production code. The README never claims a path from an error to `PAM_SUCCESS`; it says only a
fresh `Allow` becomes `PAM_SUCCESS` → PASS.

### Test Integrity & Anti-Weakening
- RDM10 reads the byte-equal setup fixture, so the wording check follows the defaults. The fixture
  is also pinned by `setup_user_phase_contract` and `crates/remote/tests/setup_render_tests.rs`. A
  missing fixture panics in `read`, so the test does not pass silently. The early return when no
  exposure key is `true` is intended: the wording rule only applies while setup turns these keys on.
- Mutation reasoning: adding "Remote unlock is off by default" or "opt-in live camera view" to any
  paragraph, table row or Mermaid line fails RDM10 (helpers RDM11 show this). Dropping `remote.toml`,
  `passkey`, `soos-gui`, `scripts/setup.sh` or any exposure key from the security section fails it. A
  camera view over Tailscale that does not name Funnel fails it. The "auto-unlock" stripping and the
  "camera snapshots" exclusion stop false positives on presence and evidence → sound.
- Limitation (not a defect): RDM10 cannot catch an inaccurate statement about **which** switches
  soos-gui has (finding 3), and it checks only README.md, not `Docs/REMOTE_COMPANION.md` (finding 5).

### Memory, Bounds & Secrets
No runtime code. No real face, login or host name: the fixture and README use `example.com` and
`tail1234` placeholders → PASS.

### Supply Chain & Automation
No `Cargo.*`, `deny.toml`, `.github/` or script change. The new module uses std only → PASS.
Mergeability: `git merge-tree origin/main HEAD` reports conflicts in `AI/DECISIONS.md` and
`AI/candid_review_report.md` → finding 2.

### English-Only Policy
All added text is English → PASS.

## 4. Detailed Findings & Action Items

1. **[MAJOR]** `README.md:54` (also `README.md:369-371`, FAQ) — The Features row says "GNOME Shell
   and KDE Plasma adapters come later", and the FAQ says the widget supports only swaylock-plugin and
   hyprlock. PR #407 (`soos-lockscreen@soos` GNOME Shell extension, `Docs/LOCKSCREEN_WIDGET.md` §11) is
   merged on `origin/main` (`e216ac1`). This now breaks the rule of ADR item (3) that every merged
   feature is described truthfully. Fix after the rebase: list GNOME Shell (45 or later, system
   extension) as supported, and keep only KDE Plasma as "later".
2. **[MAJOR]** branch vs `origin/main` — The branch conflicts with `origin/main` `e216ac1` in
   `AI/DECISIONS.md` (both append a 2026-10-17 ADR at the end) and `AI/candid_review_report.md`. The
   PR cannot be merged as it is. Fix: rebase onto `origin/main`, keep both ADRs, then run a new
   candid review (the fingerprint will change).
3. **[MINOR]** `README.md:96-97` — "with every feature on (each can be turned off later in
   soos-gui)" is wrong for `password_alerts`. No `RemoteSwitch` exists for it
   (`feature_control.rs:86-121`), and README line 55 and ADR item (7) both say so. The only soos-gui
   route is stopping the whole companion. Fix: "with every feature on (all but the failed-password
   alerts can be turned off later in soos-gui, and every key in `remote.toml`)". No existing contract
   pins this sentence (`setup_contract.rs:234-242` only checks the position of `./scripts/setup.sh`),
   and #402 is merged, so the "byte-identical for #402 rebase" reason of ADR item (2) no longer
   applies.
4. **[MINOR]** `README.md:35` — "A kill switch turns soos off at once" overstates it.
   `/etc/soos/disabled` stops face PAM and presence auto-unlock, but `soos-remote` does not read it,
   so passkey remote unlock and the camera view of a set-up companion keep working. Fix: "A kill
   switch turns face unlock off at once", as the accurate Security model bullet at line 280 says.
5. **[SUGGESTION]** `Docs/REMOTE_COMPANION.md:123, 258, 349, 594, 985, 174` (outside the diff) — The
   headings still say "(opt-in)" and §2b says the default stays tailnet-only. That now contradicts the
   setup defaults the README states. A follow-up should align the doc, or extend RDM10 to it.
6. **[SUGGESTION]** `README.md:56` — The link "§2j" is ambiguous: `Docs/REMOTE_COMPANION.md` has two
   `## 2j` headings (lines 985 and 1063, pre-existing). Renumber the Android section in a follow-up.

## 5. Final Verdict

Two MAJOR findings (1, 2): the README describes the now-merged GNOME Shell extension as future work,
and the branch conflicts with `origin/main`. Rebase, fix findings 1–4, then run a new review.

**VERDICT: CHANGES_REQUESTED**
