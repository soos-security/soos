# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `feat/remote-motion`
- **Base (merge-base)**: `3f76090` (equal to the current `origin/main`)
- **Reviewed-Diff-Fingerprint**: `09d020e858c3c4b6b4ace52f5ad9461b6863a11a0c30765cddac8ff79549782c`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_remote_motion.md`,
  `AI/auditor_constraints_remote_motion.md`, `AI/tester_contract_motion.md`,
  `AI/walkthroughs/235_remote_web_motion.md`, `Docs/REMOTE_COMPANION.md`, `Docs/UI_MOTION.md`,
  `crates/remote/assets/app.js`, `crates/remote/assets/style.css`, `tests/invariants/src/lib.rs`,
  `tests/invariants/src/remote_motion_contract.rs`

## 1. Executive Summary

Presentation-only change to the `soos-remote` web app: motion tokens, a gated `linear()` spring, state-word
keyframes, the star turn, a pending star bound to the request flags, `@starting-style` entries, a list stagger,
a View Transition for the CSS-overlay camera full screen, and one reduced-motion block that removes every
animation and transition. No Rust production code, route, header, CSP, asset file, element id or label
changes. No PAM, IPC, daemon or secret-handling code is touched. Tests are additive only (one new invariant
module). Scoped tests and clippy are green. No CRITICAL or MAJOR finding.

Commands re-run by the reviewer (with `LIBONNXRUNTIME_NO_PKG_CONFIG=1`,
`CARGO_TARGET_DIR=~/.cache/soos-targets/remote-motion`, `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`,
`CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_BUILD_JOBS=3`): `cargo test -p soos-invariants -- --test-threads=3`
(800 passed, 0 failed), `cargo test -p soos-remote -- --test-threads=3` (all 43 test binaries ok, 0 failed),
`cargo clippy -p soos-invariants --all-targets -- -D warnings` and
`cargo clippy -p soos-remote --all-targets -- -D warnings` (clean), `node --check crates/remote/assets/app.js`
(syntax ok).

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/lib.rs` (adds `mod remote_motion_contract;` under
  `#[cfg(all(test, unix))]` with a doc comment, 11 added lines, nothing removed) and the new file
  `tests/invariants/src/remote_motion_contract.rs` (1973 added lines).
- Removed or changed assertion lines (`^-` with `assert`, `#[test]`, `proptest!`, `should_panic`): **none**
  in any test file. The only `^-` hit of the grep is the pre-existing ADR line in `AI/DECISIONS.md`, whose
  word diff shows one appended implementation note and no removed text.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline test modules added or removed: **none**.
- `git diff 3f76090 HEAD -- tests 'crates/*/tests' android/app/src/test` contains only the two additions above.
  No pre-existing assertion was removed, weakened or changed.

## 3. Deep Reasoning Audit

### Logic & Architecture

- `syncPending()`: every write of `lockRequestedAt` / `unlockRequestedAt` (confirm, set, deadline, answer,
  failure, `unlockFailed`, unlock set and deadline) is followed on the next line by `syncPending();` (checked
  line by line). The pending star therefore cannot outlive the existing 5 s timers. Sign-out during a pending
  request keeps the star until the existing timer resets the flag: bounded. PASS.
- `render()` still writes the state text, `dataset.state`, the state class and `statTile.dataset.state` before
  any motion; the state class is rewritten each poll with an identical value, so the per-class keyframe does not
  replay (one keyframe name per class, six classes matching the six `LABELS` keys). PASS.
- Camera full screen: `enterCameraFullscreen` sets `cameraFull` first; the overlay-only path (no
  `requestFullscreen` / `webkitRequestFullscreen`) runs `applyCameraFull` inside `withViewTransition` and returns;
  that early return skips only the element-fullscreen request that cannot exist on that path. The
  `exitCameraFullscreen(true)` early return likewise skips only the `exitFullscreen` block, unreachable when
  element fullscreen is unavailable. `applyCameraFull` reads the current flag, so a stop/error/stall
  (`endCameraView` -> `exitCameraFullscreen()` with `animate` undefined -> direct) racing a queued transition
  callback ends in the correct state; a skipped transition still runs its update callback and its rejected
  promises are caught. Escape, the close button and the tap pass `true`; `cameraFullscreenChanged` and
  `endCameraView` do not. PASS.
- Stagger: `firstShow` is read from `hidden` before the reveal, the class is toggled before the list is
  rebuilt, and the next poll removes it, so a refresh never replays it. PASS.
- `.push button` (specificity 0,1,1) does not override `button:active:not(:disabled)` (0,2,1): the press still
  works in the notifications card. The star `<svg class="stat-star">` is the outer SVG element, so
  `transform-origin: center` turns it about its box centre; `rotate` (angle) and the pending `transform`
  keyframe compose. PASS.
- Engines without `linear()` keep the cubic-bezier fallback thanks to the `@supports` gate around the
  `:root` override. PASS.

### PAM Concurrency & Deadlines

Not applicable: no file under `crates/pam`, `crates/daemon` or `crates/protocol` changes. PASS.

### Panic Safety & Fail-Closed

No Rust production code changes. In `app.js`, `withViewTransition` falls back to the direct class change on any
throw and never leaves a promise unhandled; no new path writes a security state (no optimistic "Unlocked"; the
unlock still goes `window.confirm` -> passkey -> flag -> request, guarded by RMO-S9). PASS.

### Test Integrity & Anti-Weakening

- Section 2: additive only, no assertion touched.
- Mutation probes run by the reviewer on the working tree, then reverted with `git checkout` (tree verified
  clean, fingerprint unchanged): removing the `syncPending();` after the lock `.catch` reset made
  `test_rmo_s10_pending_star_follows_the_request_flags` fail; removing `animation: none !important` from the
  reduce block made `test_rmo_s5_single_reduced_motion_block_removes_motion` fail; turning the Escape exit into
  a non-animated `exitCameraFullscreen()` made `test_rmo_s8_app_hooks_and_guarded_view_transition` fail. The
  new tests can fail against plausible wrong implementations. `test_rmo_helper_self_test` covers the parser.
- `unwrap`/`expect` in the module are test-only, under the module-level `allow` used by the other invariant
  modules. PASS.

### Memory, Bounds & Secrets

No allocation, I/O, logging or storage is added. The View Transition snapshot is the browser's own rendering,
not readable by script; no frame is captured, cached or logged. No `.style.` write, `requestAnimationFrame`,
Web Animations API, new asset, id or route. PASS.

### Supply Chain & Automation

No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change; no library is added. PASS.

### English-Only Policy

All code, comments, docs and the commit message are English; the only non-ASCII characters are typographic
symbols. The commit subject has one scope and stays under 72 characters. PASS.

### Factual Accuracy of Doc Claims (checked against the code)

- PTR-S8 does pin `transition: none !important` in the reduce block's universal rule
  (`tests/invariants/src/remote_tour_contract.rs`, `test_ptr_s8_style_uses_brand_and_motion_tokens`), as the
  ADR note, `Docs/UI_MOTION.md` §8 and `Docs/REMOTE_COMPANION.md` state.
- "Nine keyframes" (six state words, `soos-pending-turn`, `soos-nudge`, `soos-rise-in`), the 16 px button star
  with `clip-path: path()` / `currentColor`, the 22-stop `linear()`, the 6 px card rise and 8 px banner slide,
  delays 1-4 x and 5 x `--t-stagger`, and the overlay-only View Transition all match `style.css` / `app.js`.
- RMO12 counts (CSP, eleven assets, 29 ids, nine labels) are enforced by a green test against the tree.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `crates/remote/assets/style.css` (`@starting-style` block, `.card.camera`) — during the
  260 ms entry the camera card carries a transient `transform`, which would briefly make it the containing
  block of the fixed full-screen stage if full screen were entered within that window. Practically unreachable
  (the stage only opens after a passkey ceremony); no change required.
- **[SUGGESTION]** `AI/DECISIONS.md` (Motion System ADR, item 6) — the appended note records that the web reduce
  block removes all fades (the 0 ms end of the "at most 120 ms" ceiling). It is an annotation, not a change of
  the decision; acceptable as written.
- **[SUGGESTION]** `AI/walkthroughs/235_remote_web_motion.md` — `origin/main` ends at walkthrough 231; numbers
  232-234 are presumably taken by parallel branches. Confirm no collision at merge time.

## 5. Final Verdict

**VERDICT: APPROVED**

Merge stays gated by the owner's prototype approval (UIM7) and the owner device check (RMO13 / UIM10), both
recorded as pending matrix rows.
