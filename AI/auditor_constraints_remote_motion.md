# Auditor Constraints — Web motion system of the `soos-remote` app (GitHub #433 = PR 2 of #358)

- **Date**: 2026-10-18
- **Branch**: `feat/remote-motion`
- **Inputs**: `AI/architect_spec_remote_motion.md`, `AI/plan_evaluator_report.md` (APPROVED), tester contract
  `tests/invariants/src/remote_motion_contract.rs` (Red: 3 passed, 10 failed on assertions, no compile error).
- **Scope audited**: `crates/remote/assets/style.css`, `crates/remote/assets/app.js` (presentation only). No Rust
  production code, no PAM, daemon, IPC or server change.

## Constraints for the developer

| # | Constraint | Source |
|---|---|---|
| C-1 | No `unsafe`, no new crate, no new dependency; the test module uses `std` only. | AGENTS.md, supply chain |
| C-2 | The CSP string, the routes and the eleven-file asset set stay byte-identical; `index.html` is not modified. | ADR 2026-10-13 motion item (4), RMO-S12, RXB-S8 |
| C-3 | `app.js` changes the view through `classList` and `hidden` only: no `.style.`, `cssText`, `setAttribute("style")`, `.animate(`, Web Animations API, `requestAnimationFrame`, timers or animation-end listeners. | RLC camera test, PTR-S9, RMO-S8, RMO-S9 |
| C-4 | Security truth first: `render()` writes the state word, its class, both `data-state` attributes and every `disabled` flag before calling `confirmLock` / `confirmUnlock`; no optimistic "Unlocked"; `window.confirm` and the passkey stay before `unlockRequestedAt` is set. | Docs/UI_MOTION.md §1.6, RMO-S9 |
| C-5 | The pending loop is bounded: `is-pending` follows `lockRequestedAt` / `unlockRequestedAt` through one helper called after every write of a flag; the 5000 ms confirmation timers are unchanged (WCAG 2.2.2). | RMO-S10 |
| C-6 | `startViewTransition` is feature-detected, skipped under reduced motion, wrapped in `try` with a direct fallback, and its three promises are caught; it is used only where element fullscreen is unavailable, so `requestFullscreen` keeps the tap's user activation. | RMO-S8 |
| C-7 | No frame, image or camera pixel is read, copied, blurred or stored for an effect: the View Transition snapshot is the browser's own and is unreadable by script. | Docs/UI_MOTION.md §6 |
| C-8 | No rest `transform` on the revealed cards: a non-`none` transform would make the card the containing block of the fixed full-screen camera stage and break the overlay. | RMO-S7 |
| C-9 | Brand: no new `box-shadow`, gradient, `filter`, blur or `:has()`; colours only through role tokens (S45); one `@starting-style`, one `@media (hover: hover)` and one reduced-motion block (PTR-S8, S48). | RMO-S6, brand contract |
| C-10 | Reduced motion: the universal rule keeps `transition: none !important` and adds `animation: none !important`; nothing loops or moves. | RMO-S5, PTR-S8 |
| C-11 | Every existing test stays green unmodified (brand S44–S56, RXB S1–S11, RLC camera, PTR S1–S10, UIM S1–S6). | AGENTS.md Zero Test Weakening |

## Findings

| # | Severity | Finding | Resolution |
|---|---|---|---|
| A-1 | INFO | A View Transition makes the page non-interactive for its duration (380 ms enter, 266 ms exit). | Accepted: the overlay is a visual zoom; Escape and the close button work once it ends. |
| A-2 | INFO | A request in flight when `showLogin` runs keeps its flag until the 5 s timer; the star sits in the hidden status card meanwhile. | Accepted: bounded by the timer, invisible. |
| A-3 | INFO | `renderAlerts` rebuilds the history list every poll; `stagger-once` is removed on the next render, so a refresh never replays the stagger. | Pinned by RMO-S11. |

Clearance: CLEARED
