# Tester Contract — Motion system for `soos-gui` and `soos-remote` (GitHub #377 = PR 1 of #358)

- **Date**: 2026-10-13
- **Branch**: `chore/ui-motion-spec` (GitHub-only issue, not registered in `scripts/sync_issue.py`; `Closes #377`,
  `Refs #358`)
- **Spec**: `AI/architect_spec_ui_motion.md` (plan evaluator `VALIDATION_VERDICT: APPROVED`), normative document
  `Docs/UI_MOTION.md`, ADR "[2026-10-13] Motion System for soos-gui and soos-remote".

This contract has two parts: Part A, the static invariants delivered **now** (this PR, no production code); Part B,
the contract the later `feat/remote-motion` and `feat/gui-motion` PRs must encode (their testers write and prove
these Red first; this document fixes what they must assert, not the exact code).

## Part A — This PR: `tests/invariants/src/ui_motion_spec_contract.rs`

Registered in `tests/invariants/src/lib.rs` (`#[cfg(test)]`). The pinned constants of the module (`TOKENS`,
spring parameters, `SPRING_LINEAR`, `SPRING_FALLBACK`) are the contract values.

| Test (`soos-invariants::ui_motion_spec_contract::…`) | Criterion | Red evidence (base tree `6f32ef8`) |
|---|---|---|
| `test_uim_helper_self_test` | helpers can fail: bezier solver, `linear()` parser (rejects missing positions and a wrong first stop), number, Rust-constant, CSS-declaration, hex-colour and table-row extractors, closed-form step equals the analytic response | green by design (no document needed) |
| `test_uim_s1_motion_adr_and_documents_exist` | UIM1: ADR line with its needles (brand ADR amended, browser policy, no gradients, decorative shadows, `classList`, `startViewTransition`, reduced-motion sources, `4 %`, CSP, doc and prototype paths); `Docs/UI_MOTION.md`, this contract and the prototype exist; `Docs/README.md` indexes `UI_MOTION.md`; this contract covers both later PRs | `ADR \`[2026-10-13] Motion System for soos-gui and soos-remote\` in AI/DECISIONS.md` |
| `test_uim_s2_motion_tokens_are_documented_and_consistent` | UIM2: 17 tokens each in one table row (CSS name, Rust name, CSS value); exits = round(0.7 × entry); `--t-spring` = spring settle time; forward-compatible live cross-check of `style.css` and `theme.rs` (a token already declared must have the pinned value; `--t-fast` stays `120ms`; `--ease-spring` only the fallback or the pinned `linear()`) | `read Docs/UI_MOTION.md: No such file or directory` |
| `test_uim_s3_spring_and_easing_mathematics` | UIM3: 0 < ζ < 1; 1 % ≤ overshoot ≤ 4 %; continuous 0.1 % settle ≤ `T_SPRING` (400 ms) ≤ 450 ms; closed-form stepping at 30/60/144 Hz settles (ε 0.001, velocity 0.1/s) within 450 ms, never unsettles again, overshoot ≤ 4 %, finite; `linear()` positions strictly increase, ≥ 16 stops, peak ≤ 1.04, error ≤ 0.005; fallback peak ≤ 1.04, error ≤ 0.05; standard easings monotonic, f(0) = 0, f(1) = 1, no overshoot; the document carries the parameters, both curves and the `@supports` gate | the mathematics passed; failed at `read Docs/UI_MOTION.md` |
| `test_uim_s4_prototype_is_self_contained_and_on_brand` | UIM4: ≤ 96 KiB; `<!doctype html>`, `lang="en"`; no external URL, `<link>`, `src=`, `url(`, `@import`, `data:`/`blob:`, image/video/frame, network, storage, camera API, gradient, functional colour, filter, `/home/`; no `transition: all`; `box-shadow` only `inset 0 0 0` or `none`; every hex colour is a `theme.rs` colour; every token declared with its value, both `--ease-spring` values and the `@supports` gate; `prefers-reduced-motion` and `id="reduce-motion"` | `read Docs/prototypes/ui_motion_prototype.html: No such file or directory` |
| `test_uim_s5_reduced_motion_sources_and_budgets_documented` | UIM5: `prefers-reduced-motion`, `org.freedesktop.appearance`, `reduced-motion`, `org.gnome.desktop.interface`, `enable-animations`, `--reduce-motion`, `SOOS_REDUCE_MOTION`, `200 ms`, `request_repaint_after`, `animation_time`, `startViewTransition`, `@starting-style`, `window.confirm`, dark variant out of scope, prototype path, `UIM7` | `read Docs/UI_MOTION.md: No such file or directory` |
| `test_uim_s6_remote_and_gui_docs_point_to_the_motion_spec` | UIM6: `Docs/REMOTE_COMPANION.md` §2e mentions `Docs/UI_MOTION.md`, the ADR title, `prefers-reduced-motion`, `startViewTransition`, `--t-fast`; `Docs/GUI_APPLICATION.md` points to `UI_MOTION.md` | `` §2e must mention `Docs/UI_MOTION.md` `` |

Red run: `cargo test --locked -p soos-invariants --all-features --all-targets uim` → `1 passed; 6 failed`. Every
failure is an assertion or a missing specified document (no compile error).

### Migrated existing tests

None. Every existing test stays unmodified, notably `remote_brand_contract` (S44–S56, including the S55 needles of
§2e), `remote_cross_browser_contract` (RXB-S7 and its five pinned `:has()` fallbacks), `remote_camera_contract`,
`matrix_citations` and `maintainer_hygiene_contract::test_docs_index_lists_every_docs_page`.

### Flakiness check

The module is pure file reads and deterministic arithmetic (no clock, no I/O beyond reading repository files); a
10-run loop is recorded in walkthrough 203.

## Part B — Contract of the later pull requests

The later testers may add assertions; they may not drop or weaken one of these. Existing tests stay green
**unmodified**; any change to an existing test needs a Contract Migration entry mandated by an acceptance line of
#358.

### B.1 `feat/remote-motion`: `tests/invariants/src/remote_motion_contract.rs` (rows UIM8 and new RMO* rows)

Static checks (the project has no JavaScript runtime in CI); reuse the one-level CSS parser style of
`remote_brand_contract.rs` (own copy, the other helpers stay untouched).

1. **Tokens**: `--t-fast`, `--t-short`, `--t-medium`, `--t-long`, `--t-spring`, the three exit tokens,
   `--t-stagger`, `--t-pending`, `--ease-standard`, `--ease-enter`, `--ease-exit`, `--press-scale`, `--rise`,
   `--slide`, `--star-turn` each declared **exactly once**, in a top-level `:root`, with the values of
   `Docs/UI_MOTION.md` §2. `--ease-spring` is declared exactly twice: the fallback `cubic-bezier(0.37, 0.73, 0.1,
   1.15)` in a top-level `:root`, and the pinned `linear()` in a `:root` rule inside
   `@supports (transition-timing-function: linear(0, 1))`. Durations equal the `theme.rs` constants of the same
   name (live cross-check once `feat/gui-motion` is merged; the pinned table before).
2. **Token use**: outside `:root`, every `transition`, `transition-duration`, `transition-delay`,
   `transition-timing-function`, `animation`, `animation-duration`, `animation-delay` and
   `animation-timing-function` value uses only `var(--t-*)` / `var(--ease-*)` (and `calc()` of `--t-stagger` by an
   integer ≤ 5): no literal `ms`/`s` duration and no literal `cubic-bezier(`/`linear(`/`steps(`. The single
   exception is a literal `0s` inside the reduced-motion block.
3. **Properties**: every transitioned property is one of `transform`, `translate`, `scale`, `rotate`, `opacity`,
   `color`, `background-color`, `border-color`, `outline-color`, `fill`, `box-shadow` (existing inset outlines
   only); never `all`, `width`, `height`, `top`, `left`, `right`, `bottom`, `margin*`, `padding*`, `inset`.
4. **Keyframes**: every `@keyframes` step declares only `transform`, `translate`, `scale`, `rotate` or `opacity`.
   Every animation name used anywhere is declared once. An `infinite` iteration appears only in rules whose selector
   contains `.is-pending`. The attention nudge has an iteration count of exactly 2.
5. **Reduced motion**: `media_blocks(…, "prefers-reduced-motion: reduce")` still finds exactly one block (S48); it
   contains `animation: none !important` on `*, *::before, *::after` (or an equivalent universal selector), removes
   transform transitions, and sets every remaining transition duration to `var(--t-fast)` or less.
6. **Brand kept**: no new `box-shadow` declaration (the set of `box-shadow` declarations equals the base set); no
   `linear-gradient`/`radial-gradient`/`conic-gradient` outside `html`; no `filter`, `backdrop-filter`, `blur(`;
   no new `:has()` selector (RXB-S7 keeps five); the star turn is keyed on
   `.stat-tile[data-state="unlocked"] .stat-star`; `transform-origin` of the star is its centre.
7. **Entries**: every `@starting-style` block is top level; it sets only `opacity`, `transform`/`translate`; the
   sections it targets are revealed from `[hidden]`; no `transition-behavior: allow-discrete` (exits instant).
8. **`app.js` hooks**: no `.animate(`, `.style.`, `cssText`, `setAttribute("style"`, `getAnimations`,
   `requestAnimationFrame`, `element.animate`, `Animation(`, `KeyframeEffect`, or animation library;
   `startViewTransition` appears exactly once, inside `withViewTransition`, after a
   `typeof document.startViewTransition === "function"` check and a
   `matchMedia("(prefers-reduced-motion: reduce)")` check, inside `try` with `apply()` in the `catch`; it is called
   only from the camera full-screen enter/exit path for browsers without element fullscreen. `cameraFull` is
   written before the call and the update callback reads it (no captured argument); `ready`,
   `updateCallbackDone` and `finished` each get a `catch`; the exit path of `endCameraView` (view ended, error,
   stall) applies the classes without a transition.
9. **Security truth first**: in `render()`, the writes of `setText(stateNode, …)`, `stateNode.className`,
   `stateNode.dataset.state`, `statTile.dataset.state` and the `disabled` writes precede any motion class toggle;
   no `.state-*` class, `"Unlocked"` text or `data-state="unlocked"` write occurs in `requestUnlock` before the
   server response (the unlock path still calls `window.confirm` before any request, then the passkey); no
   `setTimeout` delays a state, text or `disabled` write.
10. **Pending bound**: `classList.add("is-pending")` / `classList.toggle("is-pending", …)` happen only where
    `lockRequestedAt` / `unlockRequestedAt` is set; every place that resets them to `0` (confirmation, failure,
    `LOCK_CONFIRM_UI_MS` / `UNLOCK_CONFIRM_UI_MS` timers) clears `is-pending`; both timers stay 5000 ms.
11. **Stagger**: `stagger-once` is added only on the first render after the list's section becomes visible and is
    removed on the next render; the CSS staggers only `li:nth-child(-n+6)`.
12. **Unchanged surface**: the CSP string of `http.rs`, the eleven-file asset set, every element id and label
    (existing tests S49, S50, S54, RXB-S8 unmodified).

Owner rows (pending until the owner check PR): iPhone Safari home-screen app, Android Chrome and Firefox, with
reduced motion on and off (UIM10).

### B.2 `feat/gui-motion`: `crates/gui/tests/motion_tests.rs` and headless `egui::Context` tests (rows UIM9 and new GMO* rows)

1. **Tokens**: `theme.rs` exposes every constant of `Docs/UI_MOTION.md` §2 and §3 with the documented values
   (`UIM-S2` becomes a live check); `theme_tests.rs` and `brand_layout_tests.rs` unmodified.
2. **Spring**: `SpringState` stepped at 30, 60 and 144 Hz from 0 to 1 settles within 450 ms of simulated time,
   overshoot ≤ 4 % (and ≥ 1 %), `settled()` true from then on and the value is exactly the target; a retarget at
   peak velocity reverses without a discontinuity (value continuous across the retarget); retargeting every frame
   to the unchanged target is a no-op (the spring still settles, then requests no repaint); `dt` = 0, negative,
   `NaN`, `+inf`, `-inf` and 10 s never produce a non-finite value, never panic, and 10 s advances at most
   `MAX_MOTION_DT`; a non-finite target is ignored; a zero span is settled at once.
3. **Easing**: `cubic_bezier` maps 0 → 0 and 1 → 1, clamps t outside [0, 1], is monotonic for `EASE_STANDARD`,
   `EASE_ENTER`, `EASE_EXIT`; `NaN` input yields a finite value in [0, 1].
4. **Reduced-motion resolution** (`resolve_motion_prefs`): the CLI switch wins over everything; `SOOS_REDUCE_MOTION`
   `1` → reduced and `0` → full even when the portal says reduce; other or over-long values are ignored; portal
   `Reduce` → reduced, `NoPreference`/`Unavailable` → full; `Pending` → reduced; `GuiArgs` parses
   `--reduce-motion`.
5. **Portal read**: runs off the UI thread; a fake bus that never answers yields `Unavailable` within the 200 ms
   deadline (assert ≤ deadline + generous CI margin, never a tighter wall-clock bound); a reader thread that
   ends or panics without answering (disconnected channel) yields `Unavailable`; the UI-side poll never blocks
   (`try_recv`); no retry after a failure; no process spawn. `soos-gui` adds `zbus` and `tokio` as direct
   dependencies only (both already in `Cargo.lock`; `cargo deny` green; no new crate in the lockfile).
6. **Reduced behaviour**: with `reduced = true`, every helper returns its target on the first frame and
   `Style::animation_time` is 0; no repaint is requested by motion code.
7. **Idle budget**: after every animation settles, with no camera and no task pending, 120 headless frames request
   no repaint from motion code; the pending star requests repaints only through `request_repaint_after` ≥ 33 ms;
   `responsiveness_tests.rs` unmodified and green.
8. **Truthful overlay**: a frame whose PAD verdict flips from live to spoof is drawn with `FACE_SPOOF` on that same
   frame, and boxes are drawn at the latest detection (no interpolation).
9. **Spinners replaced**: no `egui::Spinner` remains in `camera_status.rs` and `header.rs`; the authorization text
   stays visible while the privileged action is pending.
10. **Hygiene**: no `unwrap`/`expect`/indexing panic in `crates/gui/src/motion.rs`; no allocation in the per-frame
    spring path; `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`,
    `cargo fmt --all -- --check` and the workspace tests pass.

Owner rows (pending until the owner check PR): GNOME Wayland and X11 at 60 Hz without visible dropped frames during
the state-tile and tab transitions, with reduced motion on and off (UIM11).

### B.2a Planned Contract Migration (mandated by the motion ADR, item (6), and owner decision 4)

| Test | Old assertion | New assertion | Mandating line |
|---|---|---|---|
| `soos-invariants::presence_unlock_contract::test_pau_zbus_is_used_only_by_the_daemon` (PAU17) | `ALLOWED` = `crates/daemon/Cargo.toml`, `crates/remote/Cargo.toml` | `ALLOWED` gains `crates/gui/Cargo.toml` (exactly `zbus = { workspace = true }`, named once); every other assertion and the test name unchanged | ADR 2026-10-13 "Motion System for soos-gui and soos-remote" item (6) (read-only session-bus portal exception), #358 owner decision 4 of 2026-10-13 (XDG portal over zbus) |

`feat/gui-motion` also adds a GUI invariant: the only zbus calls in `crates/gui/src` are `Settings.ReadOne` / `Read`
on `org.freedesktop.portal.Desktop`, on the portal reader thread; no `ObjectServer`, `request_name` or match rule.
`Docs/SECURITY_AND_QUALITY_GUIDELINES.md` "Audited D-Bus client" is updated in the same PR and still names
`soos-remote`.

### B.3 Approval gate

Neither `feat/remote-motion` nor `feat/gui-motion` merges before the owner approves the prototype
`Docs/prototypes/ui_motion_prototype.html` (matrix UIM7, owner decision 3 of 2026-10-13).
