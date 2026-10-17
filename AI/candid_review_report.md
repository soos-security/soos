# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `feat/phone-onboarding-tour`
- **Base (merge-base)**: `f1e7616` (rebased from `aa25f78`; `origin/main` gained #419)
- **Reviewed-Diff-Fingerprint**: `34bb2bc6402b800cad5f48e992fc6cec2c158bd5d16d22bb8419a2e217445ffd`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_phone_onboarding_tour.md`, `AI/auditor_constraints_phone_onboarding_tour.md`, `AI/walkthroughs/222_phone_onboarding_tour.md`, `Docs/REMOTE_COMPANION.md`, `crates/remote/assets/app.js`, `crates/remote/assets/style.css`, `tests/invariants/src/lib.rs`, `tests/invariants/src/remote_alerts_contract.rs`, `tests/invariants/src/remote_companion_contract.rs`, `tests/invariants/src/remote_passkey_contract.rs`, `tests/invariants/src/remote_push_contract.rs`, `tests/invariants/src/remote_tour_contract.rs`

## 0. Re-Review After the Rebase Onto `origin/main` `f1e7616`

Independent re-review, without author context, of the frozen patch `target/candid_diff.patch`
(`./scripts/candid_subagent.sh --prepare`, merge-base `f1e7616`) after the branch was rebased from
`aa25f78` onto `f1e7616`. Relative to the previous base, `origin/main` gained #419 (`f1e7616`, the
opt-in PC-offline heartbeat of the native Android app: `android/`, `scripts/android_build.sh`,
`android_heartbeat_contract.rs`, `android_native_contract.rs`, `tests/invariants/src/lib.rs`,
`Docs/REMOTE_COMPANION.md` native-app section, ADR/matrix/walkthrough 223). #402 (`aa25f78`, setup
user phase) was already in the previous base and is re-checked below.

Rebase integrity, checked mechanically:
- `git range-diff aa25f78..origin/feat/phone-onboarding-tour origin/main..HEAD`: commit 1
  (`28bcf34`, the owner-approved RMC-S18/S30/S38 contract migration, formerly `b672cfb`) is `=`;
  commit 2 (`18cfdc8`, formerly `3265761`) is `!` only because of context lines in
  `AI/DECISIONS.md` and `AI/VERIFICATION_MATRIX.md` (neighbouring #419 rows/ADR) and the pre-image
  of `AI/candid_review_report.md`, which main rewrote.
- The `+`/`-` lines of `git diff aa25f78 origin/feat/phone-onboarding-tour` and
  `git diff f1e7616 HEAD` (both excluding `AI/candid_review_report.md` and
  `AI/plan_evaluator_report.md`) are byte-identical (2498 lines each, `diff` empty). The branch's own
  changes are therefore unchanged by the rebase.
- `tests/invariants/src/lib.rs`: main's `mod android_heartbeat_contract;` and the branch's
  `mod remote_tour_contract;` are both kept, adjacent; the module count is main's plus exactly one.

Semantic interaction with #402 (setup user phase, default-on profile):
- `scripts/setup.sh` turns every companion feature on by default in the one-command setup
  (`allow_funnel`, `allow_unlock`, `password_alerts`, `push_notifications`, `offline_alerts`,
  `camera_view`, `camera_view_funnel`), while a manual `remote.toml` keeps the `config.rs` defaults
  (off, `battery_status` on). The tour makes no claim about defaults at all: no "opt-in", "off by
  default" or "on by default" wording in any tour string, so neither profile is contradicted. Each
  step only says what the feature does, what turning it off changes and how to turn it off.
- Off paths vs `setup.sh`: setup keeps an explicit `false` of a setup-managed file ("a feature
  already turned off ... stays off unless its option says on"), so the tour's manual
  `<key> = false` advice survives a setup re-run; setup's `FEATURE_PREREQS`
  (`push_notifications`<-`password_alerts`, `offline_alerts`<-`push_notifications`,
  `camera_view_funnel`<-`camera_view allow_funnel`) are exactly the dependants the tour tells the user
  to turn off together, matching `crates/remote/src/config.rs` errors (and `[android] requires
  allow_funnel`). The GUI caveat "Without an [android] table, Features in soos-gui does the same" is
  correct: `feature_control.rs` refuses files that use tables.

Semantic interaction with #419 (Android heartbeat):
- #419 touches neither `crates/remote/assets/`, `crates/remote/src/` nor `crates/gui/src/onboarding.rs`.
- The tour's "Offline alerts" step describes the PC-side `offline_alerts` feature (back-online push
  after the PC was offline) and its Off line ("no offline history is recorded and no back-online
  notification is sent; push alerts keep working") stays true; it claims nothing about the native
  app's own "PC went offline" heartbeat, which is a separate opt-in app setting.
- The only native-app statements ("the same tour in the native Android app (#357) is a follow-up;
  the app has no tour yet") remain true: `android/` contains no tour after #419.
- `android_heartbeat_contract.rs` and `android_native_contract.rs` do not inspect `app.js`; both pass
  together with `remote_tour_contract`.

Test assertions: the step-3 greps on the new frozen patch find no removed assertion line, no new
`#[ignore]`/`cfg(any())`/`should_panic`/tolerance and no inline test module change. Removed lines in
test files are exactly the three `!app.contains(forbidden)` operands of RMC-S18, RMC-S30 and RMC-S38
(now `!app_without_tour_storage(&app).contains(forbidden)`) and their `use` lists, all in the
owner-approved migration commit `28bcf34`; `git diff 28bcf34 HEAD -- tests/` only adds
`remote_tour_contract.rs` and its `lib.rs` registration. No other pre-existing assertion changed.

Executed evidence (this review): `LIBONNXRUNTIME_NO_PKG_CONFIG=1 CARGO_TARGET_DIR=~/.cache/soos-targets/phone-onboarding-tour
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=4 cargo test --locked
-p soos-invariants -p soos-remote -- --test-threads=4`: 45 test binaries, 1202 passed, 0 failed
(soos-invariants 747).

New finding of this pass: one SUGGESTION (4.6 below). Sections 1-5 below are the previous APPROVED
review (bases `6eb0746`/`aa25f78`; the migration commit `73c24ee`/`b672cfb` is now `28bcf34`), whose
findings still apply to the unchanged branch content.

## 1. Executive Summary

The diff adds a fixed-text feature tour to the `soos-remote` web app (`app.js`, `style.css`),
a single marked `localStorage` block holding one key `soos-remote-tour-v1` with the fixed value
`seen`, the ADR, matrix rows PTR1-PTR11, documentation section 2k, walkthrough 222 and a new
invariant module `remote_tour_contract`. Existing no-storage checks (RMC-S18, RMC-S30, RMC-S38)
were migrated in the owner-approved commit 73c24ee to strip only the marked block.

No CRITICAL or MAJOR defect was found. Every `remote.toml` key and dependant cited by the tour
matches `crates/remote/src/config.rs`; the six shared steps are identical to
`crates/gui/src/onboarding.rs`; the storage block is pinned to its canonical form; no request,
no `innerHTML`, no server or user data, no inline style, CSP and `index.html` unchanged.
`cargo test -p soos-invariants -p soos-remote -- --test-threads=3` passes (0 failures).
Findings are MINOR (accessibility focus/announcement) and SUGGESTION level.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/lib.rs` (module registration only),
  `remote_alerts_contract.rs`, `remote_companion_contract.rs`, `remote_passkey_contract.rs`,
  `remote_push_contract.rs` (all four from commit 73c24ee; `git status` shows no further working
  tree edit to them), `remote_tour_contract.rs` (new, untracked).
- Removed/changed assertion lines (`^-.*assert...`): none. The three changed checks keep their
  `assert!` and messages; only the operand changes from `app` to `app_without_tour_storage(&app)`
  (RMC-S18, RMC-S30, RMC-S38). This is the owner-approved contract migration 73c24ee
  (2026-10-17, #363), the only allowed edit to existing tests. Justified.
- Weakening analysis of the migration: stripping the block hides *every* forbidden token inside
  it from RMC-S18/S30/S38 (not only `localStorage`), so the commit message claim "every other
  forbidden token keep their full ban" is only true outside the block. This is compensated:
  PTR-S1 bans `localStorage` anywhere outside the block and requires exactly one top-level block,
  PTR-S2 pins the comment-free block to the canonical spec text (string literals exactly the key
  and value), PTR-S4 bans `fetch(`, `innerHTML`, `document.`, `http`, `setTimeout(` etc. inside
  the block and bans `sessionStorage`/`indexedDB`/cookies across unstripped `app.js`. A new
  self-test `test_rmc_tour_storage_exception_is_one_marked_block` proves the helper removes only
  one well-formed block and panics on duplicate, reversed or unterminated markers. Net: no
  effective weakening.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance): none.
- Inline `mod tests` changes: none.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Scenario: Funnel first visit unauthenticated -> `resume` calls `showLogin` only, `offerTour`
  never runs; after passkey sign-in `showApp` -> `offerTour` opens the tour. PASS.
- Scenario: session expires while the tour is open -> `showLogin` hides the card without marking
  it seen; `tourOffered` stays true so it is not re-offered in the same page lifetime, but is on
  the next load. Consistent with the ADR. PASS.
- Scenario: `showApp` called repeatedly (visibilitychange, pageshow) -> guarded by `tourOffered`. PASS.
- Scenario: TDZ — `showLogin` references `let tour`; all callers are asynchronous (after the
  top-level script finished), `buildTourCard()` runs after the declaration. PASS.
- Scenario: index bounds — `renderTour` clamps; `showTourTarget` at index 0 yields an `undefined`
  step -> re-render, no throw. Last step hides Skip and labels Next "Done"; Back hidden at 0 and
  focus moved to Next. PASS.
- Scenario: Show link for a hidden target (Sign out on the tailnet, camera card absent) ->
  `shownOnPage` walks ancestors, button hidden; re-checked at click time. PASS.
- Accuracy of each `where` line against `crates/remote/src/config.rs`:
  `password_alerts` off requires `push_notifications` off (`push_notifications requires
  password_alerts`), which requires `offline_alerts` off; `allow_funnel` off requires
  `camera_view_funnel` off and no `[android]` table (`[android] requires allow_funnel`);
  `camera_view` off requires `camera_view_funnel` off; `battery_status` and `allow_unlock` have no
  dependants; all are top-level `FileConfig` keys; default path `~/.config/soos/remote.toml`
  when `XDG_CONFIG_HOME` is unset. PASS.
- GUI claims against `crates/gui/src/feature_control.rs`: the Funnel switch clears `allow_funnel`
  and `camera_view_funnel` but not `[android]` (so "Without an [android] table" is the right
  caveat); Push clears `push_notifications` and `offline_alerts`; CameraView clears `camera_view`
  and `camera_view_funnel`; there is no GUI switch for battery or password alerts and the tour
  claims none. PASS.
- Shared wording vs `crates/gui/src/onboarding.rs` (titles, explanations, "Off:" lines, order). PASS.
- Labels referenced by the tour exist: "Sign out", "Disable notifications", "Stop camera view",
  "Notifications" card; alert rows carry an account class. PASS.
- `Docs/REMOTE_COMPANION.md` 2k, the ADR and the matrix rows match the code. PASS.

### PAM Concurrency & Deadlines
- No change under `crates/pam` or any Rust production code. Not applicable. PASS.

### Panic Safety & Fail-Closed
- No Rust production change. JS: every storage access (property read included) is inside one
  `try`; `catch` returns "not seen"; `closeTour` hides before writing and ignores the result, so a
  throwing storage cannot leave the card stuck. No tour path makes a request or toggles a
  feature. PASS.

### Test Integrity & Anti-Weakening
- See section 2. New tests would fail against plausible wrong implementations (a second
  `localStorage` use, a storage call outside the block, a request in tour functions, a changed
  key, wording drift from the GUI, missing dependent keys). Full suite green. PASS.

### Memory, Bounds & Secrets
- The stored value is a constant word; no personal data; no server data rendered into the tour
  (`setText` with fixed literals only, no `innerHTML`). No new origin, no inline style, CSP and
  markup unchanged. PASS.

### Accessibility & Focus
- Region label set, title `h2` focusable with `tabIndex = -1` and a focus ring; replay returns
  focus to the Feature tour button; Escape is not taken while the camera is full screen; footer
  button inherits the 48 px `--touch` min-height; reduced motion removes the entry transition.
- FINDING (MINOR): first-run close loses focus (4.1).
- FINDING (MINOR): step changes are not announced (4.2).

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or hook change. PASS.

### English-Only Policy
- Scanned the patch for non-ASCII Latin letters: none; all text, comments and docs are English. PASS.

## 4. Detailed Findings & Action Items

1. **[MINOR]** `crates/remote/assets/app.js:2422` — `closeTour` restores focus only when the tour
   was opened from the Feature tour button. When the first-run tour is closed by Skip tour, Done or
   Escape, the focused button becomes `hidden` and keyboard/screen-reader focus falls back to
   `document.body`. Suggested follow-up: move focus to a stable element in that case (for example
   the status card or the Feature tour button).
2. **[MINOR]** `crates/remote/assets/app.js:2432` (`tourNext`/`tourBack`/`renderTour`) — Next and
   Back rewrite the card text while focus stays on the button; the card has no `aria-live` and the
   title is not refocused, so a screen reader user does not hear the new step. Suggested
   follow-up: focus the title after each step change, or make the step text region polite-live.
3. **[SUGGESTION]** `crates/remote/assets/app.js:2186` (and the other manual `where` lines) — the
   manual instructions use `systemctl --user restart`, which also starts a stopped companion,
   whereas the GUI uses `try-restart`; and they name `~/.config/soos/remote.toml` although the
   effective path is `$XDG_CONFIG_HOME/soos/remote.toml` when that is set. Acceptable for a short
   tour; consider noting `XDG_CONFIG_HOME` in `Docs/REMOTE_COMPANION.md`.
4. **[SUGGESTION]** `crates/remote/assets/app.js:2330` — the Feature tour footer button is also
   present on the Funnel sign-in screen, so a user can open the tour manually there. Harmless
   (fixed text, links hidden) and the ADR's "never over the sign-in screen" covers the automatic
   offer only; consider hiding the button in `showLogin` for consistency.
5. **[SUGGESTION]** commit 73c24ee message — "every other forbidden token keep their full ban" is
   only true outside the marked block; the block content is covered by PTR-S2/PTR-S4 instead.
   No action required.

6. **[SUGGESTION]** `Docs/REMOTE_COMPANION.md` section 2k — "the native Android app (section 2j)"
   is ambiguous because main has two `## 2j.` headings (offline history and the native app, both
   pre-existing). Consider renumbering the native-app section in a follow-up. No action for this PR.

## 5. Final Verdict

**VERDICT: APPROVED**
