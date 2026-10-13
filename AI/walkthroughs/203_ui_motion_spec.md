# Walkthrough 203 — Motion ADR, Design Direction and Motion Tokens

- **Date**: 2026-10-13
- **Issue**: GitHub #377 (PR 1 of #358), no backlog id — **Branch**: `chore/ui-motion-spec` (not registered in
  `scripts/sync_issue.py`; the commit carries `Closes #377` and `Refs #358`)
- **Matrix criteria**: UIM1–UIM6 (verified), UIM7–UIM12 (pending: owner approval, later PRs, owner checks)

## 1. Context & Objectives

The owner asked (2026-10-12) for a modern, fluid, organic motion system in `soos-gui` and the `soos-remote` web
app, "Notion/Linear-class", without "AI slop", while keeping the soos brand. #358 plans four PRs; this first one
delivers the decisions and the specification only: the ADR amendment, `Docs/UI_MOTION.md`, the §2e update of
`Docs/REMOTE_COMPANION.md`, the tester contract of the later PRs and a static prototype for the owner's approval.
Owner decisions (2026-10-13): ADR amendment approved (still no gradients, no decorative shadows), spring overshoot
at most 4 %, prototype approval before the implementation PRs merge, GUI reduced motion from the XDG portal plus
`--reduce-motion` / `SOOS_REDUCE_MOTION`, GUI dark variant out of scope, four-PR split.

## 2. Architect Design

Spec `AI/architect_spec_ui_motion.md` (decisions D-1 … D-16):

- Tokens shared by name between `theme.rs` (`T_FAST` …) and `style.css` (`--t-fast` …): 120/180/260/380 ms,
  `T_SPRING` 400 ms, exits 126/182/266 ms (0.7 ×), stagger 30 ms (max 6 items), pending quarter turn 1200 ms,
  easings standard `cubic-bezier(0.2, 0, 0, 1)`, enter `cubic-bezier(0.16, 1, 0.3, 1)`, exit
  `cubic-bezier(0.4, 0, 1, 1)`, press scale 0.98 (existing value kept), rise 6 px, slide 8 px, star turn 45°.
- Spring k = 400, c = 30, m = 1: ζ = 0.75, overshoot 2.84 % (bound 4 %), within 0.1 % from 400 ms. CSS `linear()`
  with 22 stops (error 0.003) behind `@supports (transition-timing-function: linear(0, 1))`, fallback
  `cubic-bezier(0.37, 0.73, 0.1, 1.15)` (peak 1.030). The GUI steps the closed-form solution (frame-rate
  independent), `dt` clamped to 0.05 s, non-finite input harmless, same-target retarget a no-op.
- Signature move: the pink star is at 0° except in a confirmed protection-lowering state (web `unlocked`, GUI
  daemon inactive) where it is turned 45° on the spring; a pending star (quarter turn per 1.2 s) replaces the
  spinners, bounded on the web by the existing 5 s confirmation windows. Keyed on `data-state`, never a new
  `:has()` (RXB-S7 pins five).
- Facts found in the code that changed #358's plan: the unlock confirmation is the native `window.confirm`
  (no dialog to animate), the battery is a text line (no bar), the GUI has no toast component (none added), the
  face overlay is not interpolated (truth over smoothness), `--ease-spring` is declared twice (fallback and
  `@supports` value), `soos-gui` will need `zbus` and `tokio` as direct dependencies (both already in the
  lockfile).

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`: `VALIDATION_VERDICT: APPROVED`, no CRITICAL or MAJOR finding, eleven MINOR
findings, all applied (spec §9): a stronger prototype check (single inline script, no `src=`/`href=`/`srcset`/
`import(`, no named colours, no text or drop shadows), the exit check read from the document, ADR needles for
routes and assets, a View Transition that applies the current flag and catches rejected promises, the
zbus/tokio dependency note, a dead portal thread treated as "no preference", a no-op same-target retarget, the
star wording and distinct `rotate`/`transform` properties, pending matrix rows, `cargo fmt`, and a manual English
review of the HTML prototype (the candid script does not scan `.html`). The evaluator re-derived the spring
figures independently (settle at 399.93 ms, linear error 0.0030, fallback peak 1.0302).

## 4. Tester Contract

`AI/tester_contract_motion.md`. Part A, this PR: `tests/invariants/src/ui_motion_spec_contract.rs` —
`test_uim_helper_self_test`, `test_uim_s1_motion_adr_and_documents_exist`,
`test_uim_s2_motion_tokens_are_documented_and_consistent`, `test_uim_s3_spring_and_easing_mathematics`,
`test_uim_s4_prototype_is_self_contained_and_on_brand`,
`test_uim_s5_reduced_motion_sources_and_budgets_documented`,
`test_uim_s6_remote_and_gui_docs_point_to_the_motion_spec`. Red on the base tree: 1 passed (helper self-test),
6 failed, each on an assertion or a missing specified document. Part B fixes what the later
`remote_motion_contract.rs` and `crates/gui/tests/motion_tests.rs` must assert. Migrated existing tests: none.

## 5. Auditor Constraints

`AI/auditor_constraints_ui_motion.md`. First pass: BLOCKED on C-1 — the planned `zbus` dependency of `soos-gui`
would break `presence_unlock_contract::test_pau_zbus_is_used_only_by_the_daemon` (PAU17, zbus only in the daemon and
`soos-remote`) and the "Audited D-Bus client, two crates only" guideline, with no mandated Contract Migration. Fixed in
this PR (docs only): the motion ADR item (6) widens the zbus scope to a read-only session-bus portal exception for
`soos-gui` (one connection on the reader thread, `Settings.ReadOne` / `Read` on `org.freedesktop.portal.Desktop` for
the two keys, no object server, no well-known name, no signal match), and the tester contract Part B.2a records the
planned PAU17 migration (`ALLOWED` gains `crates/gui/Cargo.toml`, every other assertion unchanged). Clearance: CLEARED.
Two prototype advisories were also applied (one registered demo spring; the Slow ×4 hint). Key constraints: no production file, asset or
dependency change in this PR; the prototype stays self-contained, English-only, without secrets, without any face
image or private brand-pack file; the later PRs keep the CSP, use class hooks only, write state before motion,
keep the native confirmation and passkey, bound the pending loop, run the portal read off the UI thread with a
200 ms deadline, and add no continuous repaint.

## 6. Implementation

Documentation and tests only:

- `AI/DECISIONS.md`: ADR "[2026-10-13] Motion System for soos-gui and soos-remote" appended.
- `Docs/UI_MOTION.md` (new, indexed in `Docs/README.md`): principles, tokens, spring, choreography, signature
  move, web components W1–W9 with the View Transition helper, GUI components G1–G10, reduced motion, egui
  budgets, out of scope, prototype approval.
- `Docs/REMOTE_COMPANION.md` §2e (motion paragraph; every S55 needle kept), `Docs/GUI_APPLICATION.md` §5.5.
- `Docs/prototypes/ui_motion_prototype.html` (new): token sheet with the exact spring, `linear()` and fallback
  curves, a phone mock of the web app (status tile, lock/unlock with the pending star, banners, staggered list,
  notifications button swap, camera full-screen View Transition) and a desktop mock of the GUI (sliding tab,
  daemon switch on the closed-form spring with a pending state, staggered cards, camera status cross-fade,
  progress, banners, count cross-fade, star tiles); Reduce motion and Slow ×4 toggles. Open it with
  `xdg-open Docs/prototypes/ui_motion_prototype.html`.
- `AI/tester_contract_motion.md`, `AI/architect_spec_ui_motion.md`, `AI/plan_evaluator_report.md`,
  `AI/auditor_constraints_ui_motion.md`, matrix rows UIM1–UIM12.

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) only, per the run's instructions; the layer-2 independent review and the
fingerprint gate run in the release step (`save.sh`), not in this run.

## 8. Verification Results

```bash
cargo fmt --all -- --check
cargo clippy --locked -p soos-invariants --all-targets --all-features -- -D warnings
cargo test --locked -p soos-invariants --all-targets --all-features
./scripts/candid_review.sh
```

Results (private target directory): `cargo fmt --all -- --check` clean; clippy on `soos-invariants` with
`-D warnings` clean (one `assertions_on_constants` finding fixed with a `const { assert!(..) }` block);
`soos-invariants` 611 passed, 0 failed (the seven `ui_motion_spec_contract` tests included; `matrix_citations`,
`maintainer_hygiene_contract`, `remote_brand_contract`, `remote_cross_browser_contract` and
`presence_unlock_contract` green unmodified); the `uim` tests 10 runs in a row green; `./scripts/candid_review.sh`
PASSED. A first full run also caught two issue references (`#377`, `#358`) in the prototype that read as 3-digit
hex colours; the prototype text was reworded (the check was kept). The prototype script passes `node --check`.

## 9. Known Limitations / Follow-ups

- UIM7: the owner approves the prototype before `feat/remote-motion` and `feat/gui-motion` merge.
- The GUI reads the reduced-motion portal setting once at start-up; a live `SettingChanged` subscription is a
  follow-up.
- The pending star's `transform` returns to none when the request ends; the spring turn of the state angle
  carries the eye, but a mid-turn reset can be visible in Slow ×4.
- `candid_review.sh` does not scan `.html` for the English policy; the prototype is reviewed by hand.
- GUI dark variant: separate issue (owner decision 5). Visual-regression harness and `crate:remote` label are
  decided in the later PRs.
