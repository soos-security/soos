# Walkthrough 188 — soos Brand Redesign of the `soos-remote` Web App

- **Date**: 2026-10-06
- **Issue**: GitHub-only follow-up of #339 (owner request of 2026-10-06). **Branch**: `feat/remote-auth-alerts`
  (base `ed52b02`). Nothing is committed, pushed, deployed, installed or restarted by the agents.
- **ADR**: "[2026-10-06] soos Brand Direction Applied to the `soos-remote` Web App" (`AI/DECISIONS.md`).
- **Documents**: design brief `AI/design_brief_remote_brand.md`; spec `AI/architect_spec_remote_brand.md` (round 2);
  plan evaluation `AI/plan_evaluator_report.md`; tester contract `AI/tester_contract_brand.md`; auditor constraints
  `AI/auditor_constraints_brand.md` (C-1 to C-23); candid review `AI/candid_review_report.md`.
- **Matrix criteria**: RMC76–RMC88 (RMC88 is the owner's iPhone check).

## 1. Context & Objectives

The owner asked to "completely redesign the web app to follow the soos brand direction" and to "follow everything":
palette, logo and style. That direction becomes the brand of the whole project. The desktop GUI on `main` already
implements it (`crates/gui/src/theme.rs`, `brand.rs`, walkthrough 183 on `main`), so the phone page must use the same
token values and look like the same product.

Brand inputs: the four colors brand blue `#0047BB` (Pantone 2728 C), Brilliant White `#EDF1FF`, ink `#101820`
(Pantone Black 6 C) and pink `#E59BDC` (Pantone 244 C); the four-point star mark (`Icon_logo.svg`); the SOOS
wordmark (`text_logo.svg`); the GUI mockup (blue top band, pale body, rounded cards with a 2 px blue border, blue stat
tiles with large white text, pink star). Only the two owner vectors were copied (path data); no raster from the
owner's archive entered the repository, and the GUI mockup (it shows the owner's face) was never copied in any form.

Constraints kept from the earlier rounds: CSP `default-src 'self'` with no inline script or style, `textContent`-only
rendering, no storage, an unchanged service worker, and every UI state of `Docs/REMOTE_COMPANION.md` reachable.

## 2. Architect Design

- **Presentation only (D1, D2)**: `app.js`, `sw.js` and every Rust file under `crates/remote/src/` stay
  byte-identical. Styling hangs only on the hooks `app.js` already sets: `#state.className = "state state-<s>"`,
  `#alerts.classList.toggle("alerts-attention")`, `[hidden]`, `:disabled`, plus `:has()` for the status dot.
- **Tokens**: `style.css` declares the 25 palette constants of `theme.rs` in kebab case with identical values
  (`--blue`, `--blue-hover`, `--pale`, `--pale-2`…`--pale-4`, `--line`, `--ink`, `--ink-muted`, `--ink-weak`, `--pink`,
  `--success*`, `--danger*`, `--warn*`), the GUI radii and strokes (`--r-card` 20 px, `--r-table` 14 px,
  `--r-input` 10 px, `--stroke-card` 2 px), 14 web-only dark tints (integer-percentage mixes of two palette colors,
  rounded half up) and role tokens (`--page`, `--card`, `--tile`, `--primary`, `--danger-fg`, `--focus`, …).
  Components use role or scale tokens only.
- **Themes**: light is the default; `prefers-color-scheme: dark` redefines role tokens only (ink body, `--ink-2`
  cards, a softer blue border; the blue band, blue tile and pink star stay).
- **Layout**: blue header band with the inline SVG wordmark (accessible name "soos remote") and a static "Remote"
  pill; the pale body is cut out of the band with 20 px top corners; the lock state is a blue stat tile (pale caption
  strip with a decorative state dot, the state in large white type, the pink star); the Funnel login card carries a
  blue tile with the pink star; alerts use banners (info, danger while attempts wait for acknowledgement, warn for
  the lock-screen coverage note); lists sit in pale inner tables.
- **Buttons (D5, D12)**: `Lock now` primary blue; `Unlock now` a danger-outline button in `--danger-fg`
  (`DANGER_TEXT` light, a soft danger tint dark) so it meets WCAG AA. This deliberately differs from the GUI danger
  outline (`DANGER`, 4.22:1 on `PALE`). Labels, the confirmation dialog and Face ID are unchanged. Disabled buttons
  use a grey fill, never an opacity trick.
- **No push switch (D11)**: the only state CSS can observe (`#push-disable` visible) means "the PC has at least one
  registered device", not "this phone receives alerts", so a switch would claim a delivery guarantee the page does not
  have. `#push-state` stays the only statement of the push state.
- **Icons and colors (D6)**: `icon.svg` (also the favicon) is the owner's star mark; `apple-touch-icon.png` is
  regenerated from it (`rsvg-convert` then ImageMagick, opaque 180 x 180 RGB, `IHDR`/`IDAT`/`IEND` only); manifest
  `theme_color` `#0047BB`, `background_color` `#EDF1FF`; `theme-color` meta `#0047BB`, status bar
  `black-translucent`.
- **Fonts**: the system font stack (`--font` starts with `-apple-system`), no font file or CDN.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`: round 1 `REVISION_REQUIRED` (F1–F3 MAJOR: dark tints not recomputable, `theme.rs`
declaration forms not accepted by the parser, and a push switch that stated a property the page cannot know;
F4–F12 MINOR). Round 2 `VALIDATION_VERDICT: APPROVED` with four MINOR items: N1 (test 68 attribute-name boundary,
resolved in the test), N2 (test 75 `b'"'` case power, accepted as residual risk R-1), N3 (footer width on wide
viewports, resolved in `style.css` and checked at 1024 px), N4 (`--focus-on-blue` documented as reserved).

## 4. Tester Contract

Static invariants in the dependency-free `soos-invariants` crate (`AI/tester_contract_brand.md`):

| Test | Matrix |
|---|---|
| `remote_brand_contract::test_rmc_s44_brand_tokens_match_the_gui_theme` | RMC76 |
| `remote_brand_contract::test_rmc_s45_style_colors_come_from_the_palette_only` | RMC77 |
| `remote_brand_contract::test_rmc_s46_light_default_and_dark_variant` | RMC78 |
| `remote_brand_contract::test_rmc_s47_text_contrast_meets_wcag_aa` | RMC79 |
| `remote_brand_contract::test_rmc_s48_mobile_frame_focus_touch_motion` | RMC80 |
| `remote_brand_contract::test_rmc_s49_no_inline_style_script_or_data_uri` | RMC81 |
| `remote_brand_contract::test_rmc_s50_ui_contract_ids_labels_and_attributes_are_kept` | RMC82 |
| `remote_brand_contract::test_rmc_s51_header_band_carries_the_wordmark` | RMC83 |
| `remote_brand_contract::test_rmc_s52_icons_are_the_brand_mark` | RMC84 |
| `remote_brand_contract::test_rmc_s53_manifest_and_meta_colors` | RMC85 |
| `remote_brand_contract::test_rmc_s54_system_fonts_and_fixed_asset_set` | RMC86 |
| `remote_brand_contract::test_rmc_s55_brand_is_documented` | RMC87 |
| `remote_alerts_contract::test_rmc_s56_scanner_handles_raw_byte_and_c_strings` | RMC87 |

Plus four helper self-tests (`test_rmc_brand_helper_*`: theme parser, color mix and contrast, HTML, PNG).

Red evidence: `cargo test --locked -p soos-invariants --all-features` on the old assets gave `508 passed; 11 failed`,
exactly the expected set (tests 68 and 69 were green by design: 68 guards against regressions the new inline SVGs
could introduce, 69 pins the UI contract that must be kept). Ten repeated runs gave the identical failure set.

Contract Migration **CM-1** (the owner's optional small fix): `blank_string_literals`, the helper of test 62
(`test_rmc_s43_acknowledged_entries_are_not_kept`), only knew `r"…"`/`r#"…"#` as raw strings; a raw byte string ending
in a backslash (`br"C:\"; let acknowledged = true; let q = "x";`) was read as one escaped string, hiding the code after
it (a false negative). `br` and `cr` at an identifier boundary now open raw strings too. This strengthens the scanner;
no assertion of test 62 or its self-test changed. Walkthrough 186 §R3.7, which called the flaw "a false positive only",
was reworded.

## 5. Auditor Constraints

`AI/auditor_constraints_brand.md`, clearance `CLEARED`, C-1 to C-23. How the main ones were met:

- **C-1 to C-4 (privacy, brand material)**: only the vector path data of `Icon_logo.svg` and `text_logo.svg` is in the
  tree; the only raster is the regenerated `apple-touch-icon.png`; screenshots, the stub server and mock data live in
  the scratchpad only, with synthetic values; the docs describe the design by tokens and values, without screenshots.
- **C-5 to C-8 (CSP, scripts)**: `git diff --exit-code -- crates/remote/assets/app.js crates/remote/assets/sw.js
  crates/remote/src Cargo.toml Cargo.lock deny.toml` prints nothing; tests 68, 69, 73 and the RMC-S8 tests are green.
- **C-9 to C-13 (state integrity)**: `[hidden] { display: none !important; }` is kept before the component rules (the
  Funnel login screenshot shows only the login card); the status dot is `aria-hidden` and derives only from
  `.state-*`; `Unlock now` is a danger outline, never a filled danger button; no push switch; `--focus-on-blue` is
  reserved and no focusable element sits on the band or tile.
- **C-14 to C-16 (accessibility, layout)**: covered by tests 66 and 67; the footer shares the 480 px column; the
  visual pass asserted no horizontal scroll and no overflowing state word at 390, 375 and 320 px.
- **C-17, C-18 (icon, manifest)**: `apple-touch-icon.png` 2 742 bytes, SHA-256 `3442af18…cd46b3`, identical to the
  auditor's reproduction; only `theme_color` and `background_color` changed in the manifest.
- **C-19 to C-23 (tests, gates, process)**: CM-1 is the only edit to an existing test file; no dependency, workflow,
  script or hook change; no commit, push, stash, PR, `tailscale` call or service restart.

## 6. Implementation

Files changed:

- `crates/remote/assets/style.css`: rewritten around the token layers (palette, tints, scale, roles; dark role block;
  components; reduced-motion block).
- `crates/remote/assets/index.html`: header band with the inline wordmark, `.stat-tile` around `#state` with the
  decorative dot and star, `.login-tile` star on the Funnel login card, `theme-color` and status-bar meta values; every
  id, label, initial attribute and live region unchanged.
- `crates/remote/assets/icon.svg`: the owner's star mark (`viewBox="0 0 508 508"`, colors `#EDF1FF` and `#0047BB`).
- `crates/remote/assets/apple-touch-icon.png`: regenerated with the command of `Docs/REMOTE_COMPANION.md` §2e.
- `crates/remote/assets/manifest.webmanifest`: `theme_color` and `background_color`.
- `tests/invariants/src/remote_brand_contract.rs` (new), `tests/invariants/src/lib.rs` (registration),
  `tests/invariants/src/remote_alerts_contract.rs` (CM-1 and test 75).
- Documentation: `AI/DECISIONS.md` (ADR), `Docs/REMOTE_COMPANION.md` §2e (page design) and §4 (re-add the home-screen
  app to refresh the cached icon), `AI/ARCHITECTURE.md` §13 ("Page design" row), `AI/VERIFICATION_MATRIX.md`
  (RMC76–RMC88), `AI/walkthroughs/186_remote_auth_alerts.md` §R3.7.

`assets.rs` needed no change: the asset set is still the same seven embedded files.

## 7. Candid Review

`AI/candid_review_report.md` (2026-10-06, fingerprint `6edc9661…bca8d37`): **VERDICT: APPROVED**, no CRITICAL or MAJOR
finding. Two SUGGESTIONS, no action required: the status dot relies on `:has()` (Safari 15.4 and later; older engines
keep the neutral dot while the state stays written in words), and the GUI danger outline below WCAG AA should be
tracked as a follow-up (see §9).

## 8. Verification Results

- `cargo test --locked -p soos-invariants --all-features`: `519 passed; 0 failed`.
- Visual pass (Playwright, Chromium headless, stub server on `127.0.0.1` sending the production CSP and canned `/api`
  answers, `app.js` untouched): iPhone 14 size (390 x 844, device scale factor 3) in light and dark for the states
  locked, unlocked, Funnel login, alerts with three entries, push enabled and passkey enrollment; narrow widths 375 and
  320 px for locked, unlocked and unreachable; 1024 x 768 for the footer column; a keyboard focus capture. Result:
  "no problems" (no console error or CSP violation, no horizontal scroll, the state word never overflows the tile).
  Every screenshot was inspected; they stay outside the repository.

### 8.1 Gate output

Final run of this traceability phase (2026-10-06):

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: clean.
- `cargo test --workspace --locked --all-features`: 382 test result lines, 3 161 passed, 0 failed.
- `cargo deny --locked check`: `advisories ok, bans ok, licenses ok, sources ok`.

## 9. Known Limitations / Follow-ups

- **RMC88 pending**: the owner checks the look on the iPhone home-screen app in light and dark; iOS caches the icon,
  so the app must be removed from the home screen and added again to see the star mark.
- The desktop GUI danger outline uses `DANGER` (4.22:1 on `PALE`, below AA); the GUI should adopt the web value
  (`DANGER_TEXT` in light). Not in scope of this change.
- `[hidden] { display: none !important; }` is protected by review and the visual pass, not by an automated test
  (residual risk R-2); test 75's `b'"'` case has no positive follow-up statement (R-1).
- The walkthrough numbers 183 and 184 exist on both this line and `main`; the collision predates this change and is
  resolved at rebase.
