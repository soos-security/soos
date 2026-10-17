# Walkthrough 222 — First-Run Feature Tour in the `soos-remote` Web App

- **Date**: 2026-10-17
- **Issue**: GitHub-only #414 (PR 2 of epic #363), no backlog id. **Branch**: `feat/phone-onboarding-tour`. It is not registered in `scripts/sync_issue.py`; the commit message carries `Closes #414` and `Refs #363`.
- **Matrix criteria**: PTR1–PTR10 (verified), PTR11 (owner phone check, pending)
- **Numbering**: 218–221 are reserved by other open branches.

## 1. Context & Objectives

Epic #363 asks for a first-run tour on the PC and on the phone, with the same wording, in the soos brand and motion system, with reduced motion respected. The PC tour shipped in `soos-gui` (#379, walkthrough 209). This PR adds the phone part to the web app served by `soos-remote`.

The web app had a strict no-storage contract (RMC-S18, RMC-S30, RMC-S38). Remembering that the tour was seen needs one bit of state in the browser, so the owner decided on 2026-10-17 (#363) to allow a narrow exception: one `localStorage` key `soos-remote-tour-v1`, seen or not seen, no personal data, through one guarded helper, with a new contract proving it is the only storage use. The lead applied the matching migration of the three tests in commit 73c24ee (`app_without_tour_storage` strips the single block between `// soos-tour-storage:begin` and `// soos-tour-storage:end`).

## 2. Architect Design

Spec: `AI/architect_spec_phone_onboarding_tour.md`.

- **Pages**: an introduction and eight feature steps: Phone companion, Battery level, Remote lock and unlock, Internet access, Failed-password history, Push alerts, Live camera view, Offline alerts. Each step shows what the feature does and exposes, an "Off:" line, where to turn it off, and **Show it on this page** when the matching card is shown. Navigation uses the GUI labels: **Next**, **Done**, **Back**, **Skip tour**.
- **Same wording**: the six features shared with the PC tour reuse `Feature::{title, explanation, when_off}` of `crates/gui/src/onboarding.rs` verbatim. Battery level and Failed-password history are phone-only steps.
- **Off paths**: **Features** in `soos-gui` where it has a switch; otherwise the exact `remote.toml` key with every key that depends on it. `soos-remote` refuses a file in which a dependent key stays on (exit 78), so the text pairs `push_notifications` with `offline_alerts`, `password_alerts` with both, `camera_view` and `allow_funnel` with `camera_view_funnel`, and `allow_funnel` with removing the `[android]` table.
- **First run and replay**: `showApp` calls `offerTour()`, which runs once per page lifetime and opens the tour when the flag reads "not seen". The sign-in screen never opens it. **Feature tour**, a link-style button in the footer, reopens it at any time.
- **Storage**: one helper `tourStorage(markSeen)` in the marked block. Its whole body is one `try` (the `window.localStorage` property read included, because it throws `SecurityError` when site data is blocked), and the `catch` answers `false`. `closeTour` hides the card first, gives the focus back to **Feature tour** when the tour was opened from it, and then writes `seen`. `showLogin` hides an open tour without writing the flag (candid review, first round: two MINOR findings, both fixed before the second review).
- **No data, no request, no switch**: the tour functions read no server view, response, event, input, location, navigator or clock, call no handler, and show fixed strings of `app.js` only.
- **Markup and style**: the card and the button are built at run time, so `index.html` and its pinned id set stay unchanged. The CSP, `sw.js`, the routes and the eleven-file asset set are unchanged too. The card enters with the `Docs/UI_MOTION.md` W4 entry (`--t-medium`, `--ease-enter`, `--rise`, added with the pinned values) through `@starting-style`. Without `@starting-style`, or under reduced motion, it appears at once: the single reduced-motion block already removes every transition.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`, VALIDATION_VERDICT: APPROVED. Findings applied:

- (MAJOR) The manual off instructions must list the dependent keys.
- (MINOR) Check that the CSS parsers of the existing contracts accept `@starting-style`. They do: RMC-S45 to RMC-S48, RXB-S7 and UIM-S2 stay green.
- (MINOR, out of scope) The `soos-gui` `allow_funnel` switch does not remove an `[android]` table (follow-up, section 9).

## 4. Tester Contract

`tests/invariants/src/remote_tour_contract.rs`, registered in `tests/invariants/src/lib.rs`. It carries its own small JavaScript lexer, self-tested by `test_ptr_helper_self_test`, so no helper of the other contracts changed. The lexer keeps string and regular expression literals intact. On top of it the contract parses functions, call arguments, `TOUR_STEPS` and the Rust GUI strings.

| Test | Proves |
|---|---|
| `test_ptr_s1_storage_block_exists_exactly_once` | one block, whole marker lines, top level, every `localStorage` inside |
| `test_ptr_s2_only_one_key_and_one_fixed_value` | the block is exactly spec §3: one key, one value, one read, one write |
| `test_ptr_s3_every_access_is_guarded_and_blocked_storage_stays_dismissable` | every access in the `try`, `catch` answers false, hide before write, open on false, Skip/Done/Escape close |
| `test_ptr_s4_no_other_storage_api` | no other storage API or computed global access; no request or DOM in the block |
| `test_ptr_s5_tour_text_is_fixed_english_with_the_gui_wording` | step order, plain literals, GUI wording, off keys and dependants, labels |
| `test_ptr_s6_tour_reads_no_data_requests_nothing_and_switches_nothing` | no data read, no request, no handler, allow-listed `setText` arguments |
| `test_ptr_s7_first_run_offer_replay_and_links` | offered once from `showApp`, footer replay, visible-only links, Escape rule |
| `test_ptr_s8_style_uses_brand_and_motion_tokens` | motion tokens and values, W4 entry, reduced motion, role tokens, focus ring |
| `test_ptr_s9_no_inline_style_animation_or_markup_change` | no inline style or script animation, element kinds, attributes, `index.html` unchanged |
| `test_ptr_s10_documentation_adr_matrix_and_walkthrough` | ADR, matrix rows, section 2k, this walkthrough |

**Red phase**: before any production change, ten of the eleven tests failed on their first assertion: missing markers, functions, constants, tokens and documents. Only the helper self-test passed.

**Power check**: each of these eight mutations of the finished `app.js` made the contract fail, and `app.js` was restored after each run:

- `localStorage` used outside the block (PTR-S1, PTR-S7, and RMC-S18/S30/S38)
- the flag written before the card is hidden (PTR-S3)
- the `catch` answering `true` (PTR-S2, PTR-S3)
- a time-stamped value (PTR-S2)
- server data shown in a step (PTR-S6)
- a request from `offerTour` (PTR-S6, PTR-S7)
- an "Off:" line that drifted from the GUI (PTR-S5)
- `sessionStorage` (PTR-S4, and RMC-S18/S30/S38)

No existing test file was edited. The only migration is the lead's 73c24ee.

## 5. Auditor Constraints

`AI/auditor_constraints_phone_onboarding_tour.md`, Clearance: CLEARED. It sets 12 constraints: storage confinement, exact helper, fail-safe order, no other persistence, fixed text, no request or switch, accuracy of the off instructions, same wording, CSP and markup, style, feature detection, and immutable tests. It also records the GUI `[android]` gap described in section 9.

## 6. Implementation

- `crates/remote/assets/app.js`: the header comment now mentions the single flag. The tour section sits before the life cycle: constants, the storage block, then `buildTourCard`, `shownOnPage`, `tourTarget`, `renderTour`, `openTour`, `closeTour`, `tourNext`, `tourBack`, `showTourTarget`, `offerTour` and `tourKeydown`. `showApp` calls `offerTour()`. At load, the tour Escape listener is registered before the camera one, so an Escape that leaves a camera full screen does not also close the tour, and then `buildTourCard()` runs.
- `crates/remote/assets/style.css`: three motion tokens, the tour card rules, the W4 `@starting-style` entry, a link-style footer button with a transparent hover (inside the existing `@media (hover: hover)` block) and press state, and `.tour h2:focus-visible` added to the focus-ring rule.

## 7. Candid Review

Done by an independent reviewer, which records its findings in `AI/candid_review_report.md`, bound
to the diff fingerprint of this branch.

## 8. Verification

- `cargo fmt --all -- --check`, `cargo clippy --locked -p soos-invariants --all-features --all-targets -- -D warnings`
- `cargo test --locked -p soos-invariants --all-features` (all 719 tests, the 11 new ones included)
- `cargo test --locked -p soos-remote --all-features`, all green
- `node --check crates/remote/assets/app.js`
- **Browser run**: Chromium in a 375 x 812 phone viewport, light and dark, with the assets served by a throw-away local stub server. The stub sent the pinned CSP and fixed tailnet answers, and was kept outside the repository. Observed:
  - the tour opens once on load and walks the nine pages;
  - **Show it on this page** appears only for shown cards (not for Sign out on the tailnet);
  - **Done** stores exactly `{"soos-remote-tour-v1": "seen"}`, and a reload keeps the tour closed;
  - **Feature tour** reopens it with the focus on the title;
  - the link closes the tour and focuses the status card, and Escape closes the tour;
  - with a `localStorage` getter that throws `SecurityError`, `tourStorage()` and `tourStorage(true)` return `false`, the tour opens and Skip closes it, without an exception;
  - no console error from the tour. The only console errors came from outside the tour: the stub's 404 for the event stream, and the browser pane refusing to register the service worker (an error the page already catches). The network log shows only the existing endpoints.

## 9. Known Limitations / Follow-ups

- **Native Android app**: the same tour in the native Android app (#357) is a follow-up; the app has no tour yet.
- **GUI `[android]` gap** (found during this work, not changed here): the `soos-gui` Internet access switch clears `allow_funnel` and `camera_view_funnel` but keeps an `[android]` table, which `soos-remote` then refuses (`[android] requires allow_funnel`, exit 78). The phone text names the manual step.
- **Owner check PTR11**: iPhone (Safari and the home-screen app) and Android (Chrome).
- The tour shows no live feature state, by design (fixed text only). A later version could show the state the page already knows, if the owner wants it.
