# Plan Evaluation Report
- **Date**: 2026-10-17
- **Issue**: GitHub-only (no backlog id) — first-run onboarding tour in the `soos-remote` web app (GitHub #414, PR 2 of epic #363)
- **Branch**: `feat/phone-onboarding-tour`
- **Base commit**: `6eb0746` (plus the lead's contract migration `73c24ee`)
- **Plan evaluated**: `AI/architect_spec_phone_onboarding_tour.md` §1–§8
- **Mode**: self-check by the implementing agent against `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `Docs/UI_MOTION.md`, `Docs/REMOTE_COMPANION.md` and the code; the independent candid review runs separately.

## 1. Coverage Matrix

| Acceptance line (#363 phone part, #414, owner decision 2026-10-17) | Spec element | Status |
|---|---|---|
| Short tour on first sign-in in the web app | §2 first run (`offerTour` from `showApp`, once per page lifetime) | Mapped |
| Same wording as the PC tour | §4 (six shared titles, explanations, "Off:" lines equal to `onboarding.rs`) | Mapped |
| Explains each phone-side feature and its exposure | §4 (eight steps, two phone-only) | Mapped |
| Links to where each can be turned off | §2 links, §4 table | Mapped |
| Switches nothing on its own | §5 | Mapped |
| One key `soos-remote-tour-v1`, fixed value | §3 | Mapped |
| Single guarded helper, every access in try/catch | §3 (exact code) | Mapped |
| Blocked storage: tour unseen but dismissable, no error | §2 blocked storage, §3 | Mapped |
| No other storage API | §3, PTR-S4 | Mapped |
| Fixed English text, no user data | §4 | Mapped |
| CSP without `unsafe-inline`, no `innerHTML`, feature detection, no new origin, no network call | §7, existing RXB-S8 / RMC-S49 / RMC-S18 plus PTR-S9 | Mapped |
| Brand and motion tokens, reduced motion | §6 | Mapped |
| ADR, matrix rows (unused prefix), walkthrough 222, `Docs/REMOTE_COMPANION.md` | §1, PTR-S10 | Mapped |
| Native Android app parity | out of scope, follow-up | Mapped |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| Storage exception strips exactly one marked block | `tests/invariants/src/remote_companion_contract.rs::app_without_tour_storage` (73c24ee) | markers `// soos-tour-storage:begin` / `:end`, at most one block | Yes |
| `index.html` id set is pinned | `remote_brand_contract.rs::test_rmc_s50_*` (`PAGE_IDS`, `getElementById` set) | 29 ids, `app.js` must use exactly them | Yes (tour built at run time) |
| No template literal, `.message` only in `failureText`, two-argument `setText` | `remote_cross_browser_contract.rs::test_rxb_s5_*` | as cited | Yes |
| Asset set and CSP pinned | `test_rxb_s8_no_new_network_surface` | 11 files, pinned CSP | Yes (unchanged) |
| One reduced-motion block | `test_rmc_s48_*` | exactly one | Yes (unchanged) |
| Motion token values | `Docs/UI_MOTION.md` §2, `ui_motion_spec_contract.rs` `TOKENS` | `--t-medium` 260ms, `--ease-enter` cubic-bezier(0.16, 1, 0.3, 1), `--rise` 6px | Yes |
| Shared GUI wording | `crates/gui/src/onboarding.rs::Feature::{title, explanation, when_off}` | nine features, six phone-side | Yes |
| GUI header button and labels | `onboarding_view.rs` | "Features", "Next", "Done", "Back", "Skip tour" | Yes |
| Key dependants (`soos-remote` exit 78) | `crates/remote/src/config.rs` errors | `camera_view_funnel` needs `camera_view` and `allow_funnel`; `push_notifications` needs `password_alerts`; `offline_alerts` needs `push_notifications`; `[android]` needs `allow_funnel` | Yes |
| `battery_status` default `true`, others default `false` | `Docs/REMOTE_COMPANION.md` §5 | as cited | Yes |
| Prefix `PTR` unused | `grep -rn "PTR[0-9]" AI Docs tests` | no match | Yes |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario considered: the tour reads server data (alert counts, devices) and a shoulder-surfer reads it from the first-run card. Result: PASS — §4/§5 forbid every server view in the tour functions (PTR-S5, PTR-S6).
- Failure scenario considered: a new element id or inline style breaks the CSP-clean markup. Result: PASS — no `index.html` change; `.style.` banned (PTR-S9).

### Pillar 2 — PAM deadline & concurrency
- Not touched: no Rust code, no PAM, daemon or IPC change. The tour makes no request, so it adds no load on the companion's bounded connection pool.

### Pillar 3 — Panic safety & fail-closed
- Failure scenario considered: Safari with cookies blocked throws `SecurityError` on the `window.localStorage` getter, and an unguarded access stops `app.js` before the lock button is wired. Result: PASS — the getter itself sits inside the `try` (§3 exact code); `closeTour` hides the card before the call, so even a hypothetical throw could not leave the card stuck open.
- Failure scenario considered: storage quota exceeded on `setItem`. Result: PASS — caught, `false`.

### Pillar 4 — Dependencies
- No dependency; no new asset, font or library.

### Pillar 5 — Data confidentiality
- Failure scenario considered: the stored value grows into a profile (timestamps, step reached). Result: PASS — exact code writes only the constant `seen`; PTR-S2 pins one key and one value.
- Failure scenario considered: a manual-edit instruction leaves a dependant key on and the service refuses to start (exit 78), so the user loses the companion while trying to reduce exposure. Result: FINDING (MAJOR, fixed in the plan) — §4 table lists every dependant per key.

### Pillar 6 — Test integrity
- Failure scenario considered: the developer writes storage outside the block. Result: caught by RMC-S18/S30/S38 (migrated) and PTR-S1.
- Failure scenario considered: the GUI wording changes and the phone drifts silently. Result: caught, PTR-S5 compares the strings with `onboarding.rs`.
- Failure scenario considered: the existing CSS parsers of RMC-S45/S46/S47 or RXB-S7 reject `@starting-style`. Result: FINDING (MINOR, to verify in the red phase): if a parser fails, the motion falls back to no entry animation rather than any change to an existing test.
- No existing test is modified by this plan (the lead's 73c24ee is the only migration).

## 4. Findings

- **[MAJOR]** Manual off instructions must list dependants (exit 78 otherwise) — applied in spec §4.
- **[MINOR]** `@starting-style` parse support in the existing CSS helpers is unverified — verify in Phase 2; fall back to no motion if needed.
- **[MINOR, out of scope]** `soos-gui` turning `allow_funnel` off does not remove an `[android]` table, which `soos-remote` then refuses (exit 78); the phone text names the manual step and the GUI gap is reported as a follow-up.

## 5. Verdict

VALIDATION_VERDICT: APPROVED
