# Walkthrough 235 — Web Motion System of the soos-remote App (UIM8)

- **Date**: 2026-10-18
- **Issue**: GitHub #433, PR 2 of epic #358 (GitHub-only, no backlog id) — **Branch**: `feat/remote-motion`
- **Decision**: ADR "[2026-10-13] Motion System for soos-gui and soos-remote" (item (6) annotated for the web
  reduced-motion reading); normative `Docs/UI_MOTION.md` §6 and §8
- **Matrix criteria**: UIM8 flipped to Verified; RMO1–RMO12 (new, Verified); RMO13 (new, owner device check,
  Pending, same check as UIM10). UIM7 (owner approval of the prototype) stays Pending and gates the merge.
- **Merge gate**: DO NOT MERGE before UIM7.

## 1. Context & Objectives

PR 1 of #358 (#377, walkthrough 203) specified the motion system and its tokens, and fixed in
`AI/tester_contract_motion.md` Part B.1 what the web pull request must prove. This change implements the web
components W1–W9 in `crates/remote/assets/style.css`, with class hooks in `app.js`, and adds the twelve static
checks of Part B.1. Nothing else moves: no PAM, daemon, IPC, server, CSP, route, asset or markup change.

## 2. Architect Design (`AI/architect_spec_remote_motion.md`)

- **Tokens**: the seventeen tokens of §2 in the "Scale" `:root` (the three the feature tour had already added stay
  in place); `--ease-spring` as the `cubic-bezier` fallback there and as the pinned `linear()` inside
  `@supports (transition-timing-function: linear(0, 1))`.
- **W1**: one keyframe per state class, a single `from` step (opacity 0.4, 4 px below), so a class rewrite on the
  one-second poll replays nothing and the muted states end at their own 0.7 opacity; the dot cross-fades.
- **Star**: `rotate: var(--star-turn)` on `.stat-tile[data-state="unlocked"] .stat-star`, on the spring, about its
  centre; the pending loop animates `transform`, so the two compose.
- **W2**: press-in in `--t-fast` (the `:active` rule's own transition), release on the spring (the base rule's);
  the press stays on `transform`, so the existing `transform: none` overrides keep working. `button.is-pending`
  draws a 16 px star with `clip-path: path()` and `currentColor` at the right padding edge, so the label never
  moves.
- **W4/W5/W7**: one `@starting-style` block (the tour entry included) for the revealed cards, banners and
  notification buttons. The cards declare no rest `transform`: one would make the camera card the containing
  block of the fixed full-screen stage. Banners slide 8 px and fade; the attention summary nudges two cycles on
  `translate`.
- **W6**: `.stagger-once li` rises in; items 2–5 wait 1–4 × `--t-stagger`, the sixth onwards 5 × (150 ms).
- **W8**: `view-transition-name: camera-stage`; the group grows over `--t-long` and shrinks over `--t-long-exit`.
- **W9**: the code input border cross-fades; the focus ring stays instant.
- **Reduced motion**: the universal rule gains `animation: none !important` next to the pinned
  `transition: none !important`; everything resolves at once.
- **`app.js`**: `syncPending()` toggles `is-pending` from the two request flags and is called after every write of
  a flag; `renderAlerts` / `renderPush` toggle `stagger-once` from the section's former `hidden` flag right after
  the reveal; `withViewTransition` is the §6 helper verbatim; `applyCameraFull()` writes the classes from the
  current flag; `enterCameraFullscreen()` uses the View Transition only when element fullscreen is unavailable (so
  `requestFullscreen` keeps the tap's activation); `exitCameraFullscreen(animate)` animates only for the tap, the
  close button and Escape, never for the view ending or the browser's own exit.

Plan self-check: `AI/plan_evaluator_report.md` (APPROVED; P-1 reduced-motion reading, P-2 the "exactly once"
wording of item 8, P-3 the stagger cut-off of item 11).

## 3. Tester Contract (Red)

New module `tests/invariants/src/remote_motion_contract.rs`, registered in `lib.rs`, with its own one-level CSS
parser and a small JavaScript function extractor:

| Test | Part B.1 item | Red on `3f76090` |
|---|---|---|
| `test_rmo_helper_self_test` | helpers can fail | green by design |
| `test_rmo_s1_motion_tokens_declared_once_with_pinned_values` | 1 | `style.css declares --t-short exactly once` (0 found) |
| `test_rmo_s2_timing_values_use_the_tokens_only` | 2 | `timing declarations found: 3` |
| `test_rmo_s3_only_compositor_and_colour_properties_transition` | 3 | `a rotate transition exists` |
| `test_rmo_s4_keyframes_loops_and_state_word` | 4 | `keyframes declared: []` |
| `test_rmo_s5_single_reduced_motion_block_removes_motion` | 5 | universal `animation` is `None` |
| `test_rmo_s6_brand_kept_and_star_turn` | 6 | `one star-turn rule` (0 found) |
| `test_rmo_s7_starting_style_entries_for_revealed_sections` | 7 | only `.tour` enters |
| `test_rmo_s8_app_hooks_and_guarded_view_transition` | 8 | `one startViewTransition call` (0 found) |
| `test_rmo_s9_security_truth_first` | 9 | green by design (regression guard of the base ordering) |
| `test_rmo_s10_pending_star_follows_the_request_flags` | 10 | `app.js defines syncPending exactly once` (0) |
| `test_rmo_s11_stagger_on_first_render_only` | 11 | `two list hooks` (0 found) |
| `test_rmo_s12_csp_assets_ids_and_labels_unchanged` | 12 | green by design (regression guard) |

Red run: `cargo test --locked -p soos-invariants --all-features remote_motion` → 3 passed, 10 failed, every
failure an assertion (no compile error). One assertion of the new module was tightened during the Green phase
(before any commit): the close button now animates the overlay exit too, so RMO-S8 counts three animated exits
and pins the close-button listener. No existing test was modified; the only edit outside the new module is its
registration in `tests/invariants/src/lib.rs`.

## 4. Auditor Constraints (`AI/auditor_constraints_remote_motion.md`, CLEARED)

C-1 no new dependency; C-2 CSP, routes and assets unchanged; C-3 class and `hidden` writes only; C-4 security
truth first; C-5 the pending loop bounded by the 5 s timers; C-6 a guarded, caught, reduced-motion-aware View
Transition on the overlay path only; C-7 no frame read for an effect; C-8 no rest transform on revealed cards;
C-9 brand counts (one `@starting-style`, one hover block, one reduce block, five `:has()`); C-10 reduced motion
fully instant; C-11 every existing test unmodified.

## 5. Implementation (Green)

- `crates/remote/assets/style.css`: tokens and the gated spring; W1 keyframes; the star turn and pending loop;
  button press, release and pending star; card, banner and button entries; the nudge; the list stagger; the
  View Transition group durations; the code input border; the extended reduce block.
- `crates/remote/assets/app.js`: `syncPending()` and its calls; the two `stagger-once` toggles;
  `withViewTransition`, `applyCameraFull`, `elementFullscreenAvailable`; the enter/exit paths; the close button
  and Escape pass `true`.

A manual check in a local Chromium (static asset server, no PC) confirmed that every rule parses (125 rules, nine
keyframes, one `@starting-style` block), `--ease-spring` resolves to the `linear()` curve, the star reads `45deg`
when unlocked and settles back, `is-pending` follows the flags, and the overlay path applies its classes inside
the transition with quick enter/exit flips and a view end all leaving a consistent state, without an unhandled
rejection.

## 6. Documentation

- `Docs/UI_MOTION.md` §6: an "Implementation" paragraph; §8: the reduced-motion block as implemented.
- `Docs/REMOTE_COMPANION.md` §2e: the motion is implemented by #433 (merge after UIM7); the reduced-motion
  sentence matches the implementation.
- `AI/DECISIONS.md`: annotation of item (6) of the motion ADR (web reduced motion at its strict end).
- `AI/tester_contract_motion.md` Part B.1: delivery note and the three readings.
- `AI/VERIFICATION_MATRIX.md`: UIM8 Verified (merge gated by UIM7); component `remote-motion`, RMO1–RMO13.

## 7. Verification

```bash
LIBONNXRUNTIME_NO_PKG_CONFIG=1 CARGO_TARGET_DIR=~/.cache/soos-targets/remote-motion CARGO_INCREMENTAL=0 \
  CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=3 \
  cargo test --locked -p soos-invariants --all-features -- --test-threads=3   # 800 passed
cargo test --locked -p soos-remote --all-features -- --test-threads=3          # all passed
cargo clippy --locked -p soos-invariants --all-features --all-targets -- -D warnings
cargo fmt --all -- --check
node --check crates/remote/assets/app.js
```

Flakiness: the module reads files and computes deterministically; ten consecutive runs of `remote_motion` passed
13 of 13 each time.

## 8. Pending (owner)

- UIM7: the owner approves `Docs/prototypes/ui_motion_prototype.html`; this PR must not merge before.
- RMO13 / UIM10: iPhone Safari and home-screen app, Android Chrome and Firefox, reduced motion on and off.
