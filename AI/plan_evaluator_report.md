# Plan Evaluation Report
- **Date**: 2026-10-18
- **Issue**: GitHub-only sub-issue #433 (part of epic #358) — web motion system of the `soos-remote` app (UIM8)
- **Branch**: `feat/remote-motion`
- **Base commit**: `3f76090`
- **Plan evaluated**: `AI/architect_spec_remote_motion.md` §0–§4
- **Mode**: self-check by the implementing agent. The independent candid review runs separately before any push.

## 1. Coverage Matrix

| Acceptance line | Spec element | Status |
|---|---|---|
| Tokens and spring of `Docs/UI_MOTION.md` §2–§3 (Part B.1 item 1) | D-1 | Covered |
| W1 state word, dot, star turn | D-2, D-3 | Covered |
| W2 press, release spring, pending star bound to the request flags (items 4, 10) | D-4, D-12 | Covered |
| W3 native confirmation | D-12 (unchanged) | Covered |
| W4 `@starting-style` entries, instant hide (item 7) | D-5 | Covered |
| W5 banner slide, cross-fade, two-cycle nudge (item 4) | D-6 | Covered |
| W6 `stagger-once` (item 11) | D-7, D-13 | Covered |
| W7 notification button fade | D-8 | Covered |
| W8 View Transition on the overlay path only (item 8) | D-9, D-14 | Covered |
| W9 code input | D-10 | Covered |
| Reduced motion (item 5) | D-11 | Covered |
| Token use, properties, brand, unchanged surface, security truth (items 2, 3, 6, 9, 12) | D-1–D-15, §3 | Covered |

## 2. Facts Verified

| Fact | Check | Actual | Match |
|---|---|---|---|
| Exactly one `@starting-style` block allowed | `remote_tour_contract::css_at_block` (PTR-S8) | asserts one occurrence | Plan uses one block (D-5) |
| Reduced block universal rule pinned | PTR-S8 | `transition: none !important` | D-11 keeps it, adds `animation` |
| `.tour` rule count | PTR-S8 | base rule + entry rule = 2 | D-5 adds no `.tour` rule |
| `--t-medium`, `--ease-enter`, `--rise` declared once | PTR-S8, UIM-S2 | already in the Scale `:root` | D-1 does not repeat them |
| Five `:has()` rules only | RXB-S7 | five pinned fallbacks | no new `:has()` (D-3 uses `data-state`) |
| One `@media (hover: hover)` block | PTR-S8 | `css_at_block` | no new hover block |
| Brand parser one at-rule level | `remote_brand_contract::parse_css` | nested at-rule is an error | `@supports`, `@keyframes`, `@starting-style` all top level |
| `transform: none` overrides | `button:disabled`, tour replay, reduce block | on `transform` | D-4 keeps the press on `transform` |
| Fixed stage inside a transformed card | CSS containing block rule | any non-`none` transform | D-5 declares no rest transform |
| Element fullscreen on iPhone | `enterCameraFullscreen` | overlay only when no `requestFullscreen` | D-14 View Transition there only |
| No `.style.`, `.animate(`, `requestAnimationFrame` | RLC camera test, PTR-S9 | forbidden | D-15 |
| `theme.rs` motion constants | grep `crates/gui/src/theme.rs` | absent (GUI PR later) | pinned table, live check when present |
| Prefix `RMO` unused | grep `AI/VERIFICATION_MATRIX.md`, tests | 0 hits | Yes |
| Walkthrough 235 free | `ls AI/walkthroughs` | absent (number assigned by the lead) | Yes |

## 3. Pillar Analysis

- **Architecture alignment**: presentation only; ADR 2026-10-13 motion items (1)–(7) followed; CSP, routes and the
  eleven-file asset set unchanged.
- **Security**: state text, classes, `data-state` and `disabled` are written before any motion class; the pending
  loop is bounded by the existing 5 s confirmation timers; no optimistic state.
- **Test integrity**: existing tests unmodified; the new module carries its own parser copy (Part B.1).
- **Supply chain**: std only in the new test module; no new web asset or library.

## 4. Findings

| # | Severity | Finding | Resolution |
|---|---|---|---|
| P-1 | MINOR | §8 allows opacity and colour fades of at most `--t-fast` under reduced motion, PTR-S8 pins `transition: none !important` | Reduced motion is fully instant (strict end of §8); documented in `Docs/UI_MOTION.md` §8 |
| P-2 | INFO | Part B.1 item 8 says `startViewTransition` "appears exactly once"; the normative helper names it twice (the `typeof` guard and the call) | RMO-S8 pins one call `document.startViewTransition(` and every mention inside the helper |
| P-3 | INFO | Item 11 "staggers only `li:nth-child(-n+6)`" vs §4 "items beyond the sixth enter with the sixth" | Delays stop at 5 × `--t-stagger` (the sixth); later items share it (D-7) |

VALIDATION_VERDICT: APPROVED
