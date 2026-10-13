# Plan Evaluation Report
- **Date**: 2026-10-13
- **Issue**: GitHub-only (not in `AI/BACKLOG.md`): motion ADR, design direction and motion tokens (GitHub #377 = PR 1 of #358)
- **Branch**: `chore/ui-motion-spec`
- **Base commit**: `6f32ef8`
- **Spec evaluated**: `AI/architect_spec_ui_motion.md`, plus the draft invariants `tests/invariants/src/ui_motion_spec_contract.rs` (registered in `tests/invariants/src/lib.rs`)

## 1. Coverage Matrix
| Acceptance line (#377 / #358 "Spec and ADR" + owner decisions) | Spec element | Status |
|---|---|---|
| ADR amendment in `AI/DECISIONS.md` (amends brand ADR item (3), CSP/routes/11-file asset set/no gradients/no decorative shadows restated) | §1 row 1, D-1, D-9, §7; UIM-S1 | Covered (UIM-S1 needles miss "routes" and the asset set, see m3) |
| `Docs/UI_MOTION.md` (tokens, components, reduced motion, anti-slop rules) | D-1…D-16, §1; UIM-S2, S3, S5 | Covered |
| `Docs/REMOTE_COMPANION.md` §2e update | §1, UIM-S6 (S55 needles kept) | Covered |
| Tester contract for `feat/remote-motion` and `feat/gui-motion` | `AI/tester_contract_motion.md`, §8; UIM-S1 needles | Covered |
| Static self-contained HTML prototype in `Docs/prototypes/` | D-16; UIM-S4 | Covered (test power gaps, m1) |
| Optional invariant tests | UIM-S1…S6 | Covered |
| Owner 1: keep brand, no gradients, no decorative shadows | D-1, D-2, D-16 | Covered |
| Owner 2: spring overshoot at most 4 % | D-5, D-6; UIM-S3 | Covered, math re-verified (§2) |
| Owner 3: prototype approval before implementation PRs merge | D-16 (matrix UIM7); UIM-S5 needle `UIM7` | Covered |
| Owner 4: GUI reduced motion = XDG portal + `--reduce-motion` / `SOOS_REDUCE_MOTION` | D-13; UIM-S5 | Covered (dependency recording, m5) |
| Owner 5: GUI dark variant out of scope | D-15; UIM-S5 | Covered |
| Owner 6–8: four-PR split | §1 consumers, §8 later PRs | Covered |
| #358 lines judged not applicable (custom unlock dialog, battery bar, toasts, overlay interpolation) | D-10, D-15, §9 | Recorded as drift with rationale; owner sees them in the prototype review |
| Every existing test unmodified | §7; `git diff` touches only `lib.rs` (module registration) | Covered |

## 2. Facts Verified Against Code
| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| Only web motion token `--t-fast: 120ms` | `crates/remote/assets/style.css` l.85 | `--t-fast: 120ms;` | Yes |
| Button transition and press scale | `style.css` l.476, l.481 | `background-color/box-shadow/transform var(--t-fast)`; `scale(0.98)` | Yes |
| One reduced-motion block | `style.css` l.754; S48 `media_blocks(...).len() == 1` | one block, `* { transition: none !important }` | Yes |
| Five `:has()` selectors with pinned fallbacks | `style.css` l.400–410, 698; `HAS_FALLBACKS` (5) in `remote_cross_browser_contract.rs` l.138 | 5 selectors, RXB-S7 panics on an unpinned one | Yes |
| `data-state` hooks set synchronously in `render()` | `app.js` l.471–478 | `stateNode.dataset.state`, `className`, `statTile.dataset.state` (known states else `unknown`) | Yes |
| Native unlock confirmation | `app.js` l.938 | `window.confirm("Unlock the PC now?")` | Yes |
| `lockRequestedAt` / `unlockRequestedAt`, 5000 ms deadlines | `app.js` l.59, 62, 853, 952 | `LOCK_CONFIRM_UI_MS = UNLOCK_CONFIRM_UI_MS = 5000`; unlock flag set after `getAssertion` | Yes |
| Camera full screen hooks | `app.js` l.1601–1645, `endCameraView` l.1868 | `camera-full`, `camera-lock`; `endCameraView` exits then hides and clears synchronously | Yes |
| Brand CSS parser: one at-rule level, no nesting | `remote_brand_contract.rs` `parse_css` l.566 | "nested at-rule" / "nested block" errors | Yes |
| `app.js` style ban | `remote_camera_contract.rs` l.1132 | `.style.`, `cssText`, `setAttribute("style"`, `"style"` | Yes |
| GUI motion today | `crates/gui/src/widgets.rs` l.196, l.772 | `animate_value_with_time(…, 0.25)`, `animate_bool_with_time(…, 0.15)` | Yes |
| Spinners | `camera_status.rs` l.154, `header.rs` l.374 | `egui::Spinner` | Yes |
| GUI shadows | `theme.rs` l.94, 195–206 | `SHADOW` window/popup shadows | Yes |
| egui version | `Cargo.lock` | 0.35.0 | Yes |
| zbus | root `Cargo.toml` l.60 | `default-features = false, features = ["tokio"]` (no `blocking-api`, needs a Tokio runtime) | Yes (see m5) |
| soos-gui has no zbus/Tokio | `crates/gui/Cargo.toml` | none | Yes |
| soos-gui is Linux-only (not portable) | `portable_crates_contract.rs` l.92 | listed as Linux-only crate, zbus allowed | Yes |
| Spring D-5 (independent `python3 -I`) | `/tmp/claude-1000/spring_check.py` | ωn 20, ζ 0.75, ωd 13.2288, overshoot 2.8375 % at 237.5 ms, last 0.1 % excursion 399.93 ms | Yes (settle margin only 0.07 ms; test allows +1 ms) |
| 380/32 drift numbers (§9) | same script | ζ 0.8208, overshoot 1.095 %, settle 442.6 ms | Yes |
| D-6 `linear()` | same script | 22 stops, max error 0.0030 at 21.7 %, peak 1.0271 | Yes |
| D-6 fallback `cubic-bezier(0.37, 0.73, 0.1, 1.15)` | same script | peak 1.0302, max error 0.0400 | Yes |
| Closed-form stepping, D-7 criterion | same script | settles at 0.400 s (30 Hz), 0.400 s (60 Hz), 0.403 s (144 Hz), never unsettles | Yes |
| Docs index test scope | `maintainer_hygiene_contract.rs` l.488 | top-level `Docs/*.md` only; `Docs/prototypes/*.html` not required in the index | OK |
| `/home/` path test scope | same file l.120 | scripts, hooks, packaging, tests/docker…; not `Docs/` (UIM-S4 forbids `/home/` anyway) | OK |
| Secret scan | `scripts/secret_scan.sh` l.82 | PEM, GitHub, AWS, Slack, `sk-` patterns; key file names | Prototype cannot match (no keys) |
| English check | `scripts/candid_review.sh` l.304 | scans `*.rs *.md *.sh *.yml *.toml`, **not `*.html`** | Gap (m7) |
| Draft test formatting | `rustfmt --check` | fails (l.115 `STANDARD_EASINGS`, l.289–290, self-test) | m8 |

## 3. Pillar Analysis
### Pillar 1 — Architecture & threat model
- Failure scenario considered: the View Transition snapshot or a motion hook widens the web app's surface (new asset, CSP relaxation, frame retained by an effect). D-9 forbids WAAPI, `.style.`, new assets and CSP changes; `::view-transition-*` styles live in `style.css` under `style-src 'self'`. `endCameraView` hides the stage and clears the canvas synchronously after `exitCameraFullscreen`, so the old-state capture (next rendering step) never holds a frame. No daemon, PAM, IPC or socket change.
- Result: PASS (one robustness gap in the View Transition callback, m4).

### Pillar 2 — PAM deadline & concurrency
- N/A for PAM: no `crates/pam` change. Analogue checked: the GUI UI thread. Scenario: portal read blocks the UI; D-13 confines it to a background thread with one 200 ms deadline and `try_recv` on the UI side. Scenario: the background thread panics (a zbus `tokio` build outside a runtime context panics) and drops the sender; the spec does not say that `Disconnected` or a UI-side elapsed deadline maps to "no preference", so motion could stay "reduced" for the whole session (fail-safe direction, but silent).
- Result: FINDING (MINOR, m6).

### Pillar 3 — Panic safety & fail-closed
- Scenario: `dt = NaN/inf/negative/10 s` or a non-finite target produces a non-finite position, or repaint never stops. D-7 clamps `dt` to `[0, 0.05]`, snaps non-finite state, ignores non-finite targets. Gap: if the widget helper calls `retarget(target)` every frame (normal immediate-mode use) and `retarget` resets `span` even when the target is unchanged, the settle thresholds shrink each frame (never settled → continuous repaint, violating D-14) or `span = 0` snaps mid-overshoot. No security decision depends on motion; a View Transition throw falls back to the direct class change.
- Result: FINDING (MINOR, m9).

### Pillar 4 — Dependencies
- Scenario: the later `feat/gui-motion` PR needs the portal read; the workspace `zbus` has `default-features = false, features = ["tokio"]`, so `zbus::blocking` (#358 wording) does not exist and the GUI must add direct `zbus` **and** `tokio` dependencies and run a current-thread runtime. `Docs/UI_MOTION.md` §8 already says so, but the spec (§1, §4, §7) and the ADR text do not record the new direct dependencies of `soos-gui`, while #358 told the owner "optional `zbus` ... no other new dependency". No new crate enters `Cargo.lock` (both already locked) and `soos-gui` is not a portable crate, so `deny.toml` and the portability guard are unaffected. No OpenCV, nokhwa, animation library or web font.
- Result: FINDING (MINOR, m5).

### Pillar 5 — Data confidentiality
- Scenario: the prototype embeds the owner's face or a brand-pack raster, or an effect caches a camera frame. D-16 forbids images, `data:` URIs, `url(`, fonts and the private brand pack and uses a flat camera placeholder; D-9/§7 forbid frame capture, blur or caching. `SOOS_REDUCE_MOTION` longer than 8 bytes is ignored and never logged.
- Result: PASS.

### Pillar 6 — Test integrity
- Existing tests: the diff only registers `mod ui_motion_spec_contract;` in `lib.rs`; no existing test changes. The design keeps RXB-S7 (no sixth `:has()`), S44/S46 (`--ease-spring` second declaration inside `@supports` is not a top-level `:root` palette/role token), S45 (no colour functions, `linear(` is not one), S48 (one reduced block) and the camera style ban green.
- Power: UIM-S3 recomputes the pinned spring from first principles and would fail on 380/32 vs a doc stating 400/30, on a `>` 4 % overshoot, on a reordered or mistyped `linear()` stop and on a non-monotonic easing. UIM-S2's row check fails on a wrong value or a swapped Rust name. Weak spots: (a) UIM-S4 self-containment can be bypassed (`<script` + newline + `src=`, `import("//cdn…")`, `img.src = "//…"`, `srcset=`, `<source`, `<audio`, `<use href="//…">`), and colour/shadow rules miss named colours, `text-shadow` and `drop-shadow` (a prototype loading GSAP from a protocol-relative URL, or using `color: red` or a glow, passes); (b) the UIM-S2 exit check compares the test's own constants (cannot fail; the doc-row check is the real pin); (c) UIM-S5 needle `reduced-motion` is implied by `prefers-reduced-motion`; (d) UIM-S1 does not pin the "routes" / asset-set restatement #358 requires; (e) the draft fails `cargo fmt --check`.
- Result: FINDING (MINOR, m1, m2, m3, m8).

## 4. Findings
- **[MINOR] m1** UIM-S4 self-containment and brand checks are bypassable (see Pillar 6 (a)). Required change: forbid every `<script` occurrence other than the exact `<script>` opening tag, plus `import(`, `import `, `new image`, `.src`, `srcset`, `<source`, `<audio`, `<track`, `<use`, `<image`, `worker(`, `href="//`, `text-shadow`, `drop-shadow`; scan CSS declaration values inside `<style>` for the brand contract's named colours.
- **[MINOR] m2** UIM-S2's exit assertion is tautological and UIM-S5's `reduced-motion` needle is redundant. Required change: check the 0.7 × entry relation on the values read from the `Docs/UI_MOTION.md` table rows; replace `reduced-motion` with `` `reduced-motion` `` together with `org.freedesktop.appearance` on one line (or `u32`).
- **[MINOR] m3** UIM-S1 does not pin the restatement #358 requires. Required change: add ADR needles for the unchanged routes and the eleven-file asset set (for example "routes" and "eleven").
- **[MINOR] m4** D-9 View Transition: the update callback is deferred, so an Escape or `fullscreenchange` exit followed by a quick re-enter can run the stale "remove `camera-full`" after the synchronous enter (`cameraFull = true`, no overlay, close button visible); `transition.ready` and `finished` reject unhandled when the transition is skipped. Required change: the callback applies `classList.toggle("camera-full", cameraFull)` (and `camera-lock`) from the current flag, the promise rejections are caught, and exits caused by `endCameraView` (view end, lock, `pagehide`) skip the View Transition. The tester contract pins all three.
- **[MINOR] m5** Record the `soos-gui` dependency change. Required change: in spec §1/§4/§7 and in the ADR, state that `feat/gui-motion` adds workspace `zbus` and `tokio` (`rt`, `time`) as direct `soos-gui` dependencies (no new crate in `Cargo.lock`, `blocking-api` not enabled, a current-thread runtime on one background thread), correcting #358's "no other new dependency".
- **[MINOR] m6** D-13 does not cover the background thread dying. Required change: the UI side treats `TryRecvError::Disconnected`, or `PORTAL_READ_DEADLINE_MS` elapsed since start-up, as `PortalAnswer::Unavailable` (full motion), and the thread body runs under `catch_unwind` or returns `Unavailable` on every error. The tester contract adds a `resolve_motion_prefs` case for it.
- **[MINOR] m7** `scripts/candid_review.sh` does not scan `*.html`, so the prototype's English is enforced only by `lang="en"`. Required change: the auditor and the candid reviewer read the prototype text by hand (record this in the auditor constraints); do not change the script in this PR.
- **[MINOR] m8** The draft `ui_motion_spec_contract.rs` fails `cargo fmt --check` (l.115, l.289–290). Required change: run `cargo fmt` before the tester commits it.
- **[MINOR] m9** D-7 leaves `retarget` with an unchanged target unspecified. Required change: `retarget(t)` is a no-op when `t` equals the current target; only a real target change resets `span` and keeps the velocity. The tester contract tests that 120 frames of repeated same-target `retarget` + `step` settle within 450 ms and then request no repaint.
- **[MINOR] m10** D-8 has two gaps. First, the pending loop (`animation` on `transform`) and the state turn (`transition` on `transform`) drive the same property on `.stat-star`, so the star snaps when `is-pending` clears together with the state change. Second, "turns back when protection returns" is wrong for `unreachable`, `unavailable` and `unknown`, which also show 0°. Required change: put the state turn on the individual `rotate` property and the pending loop on `transform` (or the reverse), so the two compose. Also word D-8 as "0° in every state except a confirmed `unlocked`".
- **[MINOR] m11** Matrix rows UIM8–UIM12 cite tests of later PRs (`remote_motion_contract.rs`, `motion_tests.rs`). Required change: those rows stay in a pending status (not `✅ Verified`/`☑ Validated`), or `matrix_citations` fails.

No CRITICAL or MAJOR finding. Every fact the plan cites matches the code, the spring and `linear()` figures hold to the stated precision, and no existing test is modified. The MINOR items are carried as constraints into `AI/tester_contract_motion.md` and `AI/auditor_constraints_ui_motion.md`.

## 5. Verdict
VALIDATION_VERDICT: APPROVED
