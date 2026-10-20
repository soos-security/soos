# Candid Review Report

- **Date**: 2026-10-20
- **Target Branch**: `feat/remote-motion` (PR #436, GitHub #433 = PR 2 of #358), after the rebase onto `origin/main` `da320a4`
- **Base (merge-base)**: `da320a4`
- **Reviewed-Diff-Fingerprint**: `6ae0f13f445950239baa8c2fd5c028bf53ff7ddc68bf3666be17d693e0dec56c`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_remote_motion.md`, `AI/auditor_constraints_remote_motion.md`, `AI/tester_contract_motion.md`, `AI/walkthroughs/235_remote_web_motion.md`, `Docs/REMOTE_COMPANION.md`, `Docs/UI_MOTION.md`, `crates/remote/assets/app.js`, `crates/remote/assets/style.css`, `tests/invariants/src/lib.rs`, `tests/invariants/src/remote_motion_contract.rs`

## 1. Executive Summary

Independent review of the full raw diff against the new merge base, with the semantic interaction with main
(#418 phone tour, #450/#461 README web captures, #457) checked explicitly. The motion code itself is sound:
class-only hooks, one guarded `startViewTransition`, the pending flags drive `is-pending` through one helper
after every write, security truth is written before any motion, CSP/routes/assets/ids are unchanged, and the
single reduced-motion block removes every transition and animation. The tour card keeps its main entry
(`.tour` transition, now inside the single `@starting-style` block), the single `// soos-tour-storage:begin/end`
block is untouched (app.js 2315–2336), and RMC-S18/S30/S38, PTR-S1…S10, RXB-S* and RMO-S1…S12 all pass.

However, the requested test command **fails**:
`soos-invariants::matrix_citations::test_matrix_claimed_rows_cite_at_least_one_resolvable_evidence` rejects the
UIM7 row that `ee3f495` flipped to `✅ Verified` without resolvable evidence, so CI (Quality) would fail. In
addition, the motion change alters two committed README captures (the unlocked scenes now show the star turned
45°), so `Docs/assets/screenshots/web-unlocked-{light,dark}.png` and their MANIFEST hashes are stale.
Verdict: **CHANGES_REQUESTED**.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `tests/invariants/src/lib.rs` (+11: one new `#[cfg(all(test, unix))] mod remote_motion_contract;`, the 96th module = 95 on main + 1), `tests/invariants/src/remote_motion_contract.rs` (new, 13 tests).
- Removed/changed assertion lines (`^-` with assert/#[test]/proptest/should_panic): **none** in code. The only grep hit is the ADR line in `AI/DECISIONS.md` (prose, it contains the word "assertion"); the diff only appends a dated italic note to item (6), no ADR text is removed (verified with `--word-diff`).
- New escape hatches (`#[ignore]`, `cfg(any())`, `should_panic`, tolerance/epsilon): **none**.
- Inline `mod tests` added/removed: **none**.
- `AI/tester_contract_motion.md`: only an appended "Delivered" paragraph; no contract item reworded.

No pre-existing assertion changed.

## 3. Deep Reasoning Audit

### Logic & Architecture
- **Pending flags**: every assignment of `lockRequestedAt` / `unlockRequestedAt` (app.js 640, 650, 1024, 1032, 1051, 1072, 1086, 1128, 1136) is immediately followed by `syncPending()`; the only unpaired lines are the `let` declarations (282, 284). Scenario "unlock POST succeeds but the desktop never confirms" → the 5000 ms timer resets the flag and `syncPending()` removes the star. PASS.
- **View Transition race**: tap-enter, then `endCameraView` (stop/error/stall) before the transition callback runs → `exitCameraFullscreen()` sets `cameraFull = false` and applies the classes synchronously; the deferred callback `applyCameraFull` reads the current flag (false) and re-applies the same state. No stale overlay. PASS.
- **Event argument leak**: `closeButton` now wraps `exitCameraFullscreen(true)`; `cameraFullscreenChanged` (1884) and `endCameraView` (2100) call it without an argument; `animate === true` is strict, so a stray `Event` cannot request animation. PASS.
- **Element fullscreen path**: `requestFullscreen` stays inside the tap activation (classes applied synchronously, no transition on that path). PASS.
- **State word replay**: `stateNode.className` is rewritten every second with an identical string; an unchanged `animation-name` does not restart, and each state has its own keyframe name so a real change does. PASS.
- **Nudge**: `.alerts-attention` is toggled with `classList.toggle` on a persistent node (app.js 808), not recreated per poll, so the 2-iteration nudge does not replay each second. PASS.
- **Stagger**: `firstShow` is read before `hidden = false`; the list is rebuilt each render, so later renders (class removed) do not animate. PASS.
- **Interaction with #418 tour**: `.tour` keeps the main transition (style.css 1164) and its entry now lives in the single top-level `@starting-style` block (RMO7, PTR-S8 green). `.tour-where` is a `.banner`, so it also slides in when revealed — harmless and consistent with W5. `soos-tour-storage` block unique and unchanged. PASS.
- **Containing block**: revealed cards carry no rest `transform`; the transient entry transform of the camera card lasts 260 ms and the stage cannot be full screen before the card is shown. PASS.
- **Docs**: the documented deviation (reduced motion removes even the ≤120 ms fades because PTR-S8 pins `transition: none !important`) is recorded in the ADR item (6) note, `Docs/UI_MOTION.md` §8 and the tester contract. PASS.
- **Matrix**: the UIM7 row (2371) claims `✅ Verified` but cites no resolvable evidence → **FINDING (MAJOR)**. A duplicated `---` separator was left by the conflict resolution (2996, 2998) → MINOR.

### PAM Concurrency & Deadlines
No change under `crates/pam` or in any Rust production code. N/A — PASS.

### Panic Safety & Fail-Closed
No Rust production code changed. In `app.js`, `withViewTransition` catches a throwing `startViewTransition` and applies directly, and swallows the three transition promises so a skipped transition leaves no unhandled rejection. No path writes a protection-lowering state optimistically (RMO9 green: `requestUnlock` keeps `window.confirm`, the passkey, then the pending flag, then the request). PASS.

### Test Integrity & Anti-Weakening
- Step-3 listing above: no assertion removed or relaxed.
- Requested command (`cargo test --locked -p soos-invariants -p soos-remote -p soos-test-fixtures -- --test-threads=3`, re-run with `--no-fail-fast` to reach every target): `soos-invariants` 859 passed / **1 failed** (`matrix_citations::test_matrix_claimed_rows_cite_at_least_one_resolvable_evidence`, `AI/VERIFICATION_MATRIX.md:2371 UIM7`); every `soos-remote` and `soos-test-fixtures` target passed. Relevant groups: RMO 13/13, PTR-S 10/10, RXB-S 11/11, RMC-S18/S30/S38 ok, README contracts 27/27.
- New tests: `test_rmo_helper_self_test` proves the parser/extractors can fail; RMO9/RMO12 are explicit regression guards. PASS apart from the failing matrix invariant.

### Memory, Bounds & Secrets
No new timer (motion adds none; both confirmation timers stay 5000 ms), no storage, no network, no `.style.` writes, no frame access: the View Transition snapshot is the browser's own, unreadable by script. CSP string unchanged (no server code in the diff; RMO12 and RXB green). Reduced motion: one `prefers-reduced-motion` block with `transition: none !important; animation: none !important;` on `*, *::before, *::after`, and `withViewTransition` skips the transition under reduce. PASS.

### Supply Chain & Automation
No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. No library, polyfill or web font added. PASS.

**README captures (#450/#461)** — re-run in scratch copies (`git archive`) with `CHROME=/usr/bin/google-chrome-stable`
(headless Chrome, `--virtual-time-budget=4000`, so `@starting-style` entries, W1 and the nudge have settled and the
captured frame is the rest state):
- Base `da320a4`: all four PNGs reproduce `Docs/assets/MANIFEST.toml` bit for bit (the host is a valid reference).
- Head: `web-locked-alerts-{light,dark}.png` identical to MANIFEST (`02a30d2b…`, `4cb600f4…`).
- Head: `web-unlocked-light.png` → `db661b7ddac3e5e26a572905f61520838401db1a5bb20a130af42668c9fdf3a1` (MANIFEST `03838f7b…`), `web-unlocked-dark.png` → `186c80a2dda7fbc9c6e3206b6b33218ecd50ec1e0208912cdc741d49c36e2326` (MANIFEST `c150088e…`), deterministic over two runs. The only differing region (119×120 px, 930 px) is the tile star, turned 45° by `.stat-tile[data-state="unlocked"] .stat-star { rotate: var(--star-turn); }` — the intended signature move. The committed captures therefore no longer show what the generator produces → **FINDING (MINOR)**. No image was committed by this review.
- Observation: Chrome hung (killed after 15 min, then a 90 s timeout) on 2 of 6 head `locked-alerts` screenshot runs and 0 of 5 base runs; every completed head run matched MANIFEST. The sample is too small to attribute it to the motion CSS; the capture script has no per-run timeout → SUGGESTION.

### English-Only Policy
All code, comments, docs and matrix rows in the diff are English. PASS.

## 4. Detailed Findings & Action Items

- **[MAJOR]** `AI/VERIFICATION_MATRIX.md:2371` — row UIM7 was set to `✅ Verified` by `ee3f495` with only "Owner opens the prototype … records the approval on #358" as evidence; `soos-invariants::matrix_citations::test_matrix_claimed_rows_cite_at_least_one_resolvable_evidence` fails ("1 claimed rows cite no resolvable evidence … UIM7"), so CI Quality will be red. Correction (documentation only, no test change): keep the owner-approval text and add resolvable evidence, e.g. the prototype path `Docs/prototypes/ui_motion_prototype.html` and/or `soos-invariants::ui_motion_spec_contract::test_uim_s4_prototype_is_self_contained_and_on_brand` (the pattern of owner rows such as RMC20), then re-run the suite.
- **[MINOR]** `Docs/assets/screenshots/web-unlocked-light.png`, `Docs/assets/screenshots/web-unlocked-dark.png`, `Docs/assets/MANIFEST.toml:35-37` and `:41-43` — stale after this change: `scripts/capture_readme_web.sh` now yields `db661b7d…` (light) and `186c80a2…` (dark); the README (`README.md:85`, `:89`) shows the unlocked tile with the star at 0° while the shipped app turns it 45°. Regenerate with `scripts/capture_readme_web.sh` and `scripts/capture_readme_assets.sh --web` on this branch (or in a follow-up `chore/` PR) so the README matches the generator.
- **[MINOR]** `AI/VERIFICATION_MATRIX.md:2996-2998` — two consecutive `---` separators left by the conflict resolution; drop one.
- **[SUGGESTION]** `scripts/capture_readme_web.sh` — browser runs have no timeout; an intermittent headless hang (seen 2/6 on head `locked-alerts`, 0/5 on base) blocks the script indefinitely. Consider a bounded `timeout` per browser run in a separate change.

## 5. Final Verdict

**VERDICT: CHANGES_REQUESTED**
