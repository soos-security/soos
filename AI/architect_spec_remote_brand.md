# Architect Spec — GitHub #339 follow-up: soos Brand Direction Applied to the `soos-remote` Web App

- **Date**: 2026-10-06
- **Round**: 2 (answers `AI/plan_evaluator_report.md` round 1: F1–F3 MAJOR, F4–F12 MINOR; see §16)
- **Branch**: `feat/remote-auth-alerts` (base `ed52b02`); nothing is committed, pushed, stashed, deployed, installed or
  restarted by agents. No agent runs `tailscale`, touches `~/.config` or restarts a host service.
- **Owner request (2026-10-06, binding)**: completely redesign the web app to follow the soos brand direction, which
  becomes the brand of the whole soos project; "follow everything": palette, logo, style.
- **Inputs**: `AI/design_brief_remote_brand.md` (the design study; this spec turns it into file changes and tests and
  overrides it where §0.3 says so), the owner archive (`Colors_Direction.svg`, `Icon_logo.svg`, `text_logo.svg`,
  `Artistic_charts_assets/*.png`, `New_Gui_Interface.png`, all read-only in the scratchpad), `origin/main` at `47ab53e`:
  `crates/gui/src/theme.rs`, `brand.rs`, `header.rs`, `widgets.rs`, `Docs/GUI_APPLICATION.md` §5,
  `AI/walkthroughs/183_gui_brand_redesign.md`; on this branch: `crates/remote/assets/*`, `crates/remote/src/assets.rs`,
  `crates/remote/src/http.rs` (CSP), `Docs/REMOTE_COMPANION.md` §2a–2d and §6, the five `soos-remote` ADRs,
  `tests/invariants/src/remote_{companion,passkey,alerts,push}_contract.rs`, `crates/remote/tests/{routes,server}_tests.rs`.
- **ADR**: "[2026-10-06] soos Brand Direction Applied to the `soos-remote` Web App" (drafted in `AI/DECISIONS.md`;
  the text there is authoritative, §12 summarises it).
- **Matrix**: new rows RMC76–RMC88 (§11), after RMC75.
- **Walkthrough**: `AI/walkthroughs/190_remote_brand_redesign.md` (traceability phase).
- **Test prefix**: `test_rmc_s44`–`test_rmc_s56` (static invariants, tests 63–75: test N is `test_rmc_s(N − 19)_…`);
  tests 63–74 in the new module `tests/invariants/src/remote_brand_contract.rs`, test 75 in
  `tests/invariants/src/remote_alerts_contract.rs`.

## 0. Decisions

### 0.1 Owner constraints restated (binding)

1. Palette, logo and style of the soos brand, identical token values to the desktop GUI (`theme.rs` on `origin/main`),
   so phone and desktop read as one product.
2. Mobile first (iPhone, home-screen standalone, safe-area insets); light is the brand default; a dark variant built
   from the same palette (ink `#101820` body) under `prefers-color-scheme`.
3. WCAG AA contrast for text, visible focus, 44 px touch targets.
4. Header: the SOOS wordmark (inline SVG) on the blue band. Star mark as `icon.svg`, `apple-touch-icon.png` (180 x 180,
   opaque, regenerated from the vector) and favicon. Manifest colors from the palette. Status as a big blue tile.
5. No raster from the owner archive in the repository; `New_Gui_Interface.png` (shows the owner's face) is never copied,
   embedded, traced or described pixel by pixel in the repository. `Icon_logo.svg` and `text_logo.svg` path data may be
   copied (owner-provided brand vectors; `brand.rs` on `origin/main` already carries them).
6. Every element id, text label and behaviour that tests or docs rely on stays (§5). CSP unchanged (`default-src 'self'`,
   `style-src 'self'`, `img-src 'self'`): no inline `<script>`, no `style=""` attribute, no `<style>` element, no remote
   font or CDN, no `data:` URI; `textContent` only; no storage; `sw.js` and `app.js` unchanged.
7. System font stack only (no font file).

### 0.2 Spec-level decisions

| # | Decision | Why |
|---|---|---|
| D1 | `app.js`, `sw.js` and every Rust source file stay **byte-identical**. `assets.rs` needs no change: no asset is added, renamed or removed (the asset directory keeps exactly its seven files). | Presentation-only redesign, exactly like the GUI redesign of walkthrough 183; every state is already exposed through ids, `hidden`, `.state-*`, `data-state` and `.alerts-attention`. |
| D2 | All styling hangs on hooks `app.js` already sets: `#state.state-<s>` (note: `app.js` **replaces** `className` of `#state`, so `#state` must not carry any other class), `#alerts.alerts-attention`, `[hidden]`, `:disabled`; the one derived look (status dot) uses `:has()` (Safari 15.4+, below the iOS 16.4 already required by Web Push). | No JavaScript change (D1). |
| D3 | Tokens: 25 **palette** custom properties named after the `theme.rs` constants (kebab case, same hex), 14 **dark tints** (each a mix of two palette colors at an integer percentage, recomputable in integer arithmetic, §2.2b), and **role** tokens (§2.3). Components reference role tokens only. | One vocabulary with the GUI; a test can prove parity and palette-only colors. |
| D4 | Token parity source of truth: `crates/gui/src/theme.rs` on `origin/main` (`47ab53e`). This branch does not contain that file yet, so the test pins the 25 values in a table copied from it **and**, whenever `crates/gui/src/theme.rs` exists in the tree (after this branch is rebased on or merged with `main`), parses the 25 color constants and the six numeric constants in every declaration form `theme.rs` uses at `47ab53e` (§9, test 63: `Color32::from_rgb(..)`, `Color32::WHITE`, `u8` integer literals, `f32` literals such as `2.0`) and requires equality with the pinned table; an unrecognised form or a missing constant fails the test (never a skip). The parser is exercised now by a self-test on an embedded excerpt of that file, so the branch that is dormant before the merge is not untested. | The test is strict now (pinned values) and becomes a live cross-crate parity check after the merge, with no test edit. |
| D5 | `#lock` "Lock now" = **primary** (blue). `#unlock` "Unlock now" = **danger outline** (`class="secondary danger"`). `#logout`, `#alerts-ack`, `#push-test`, `#push-disable` = secondary. `#login-button`, `#enroll-button`, `#push-enable` = primary. | Brief §5.4: locking is the frequent, safe action; unlocking lowers protection (already behind `confirm()` and Face ID); only one of the two is enabled at a time, so the enabled one is always the only colored control. Labels unchanged. |
| D6 | `theme-color` meta `#0047BB` (one value, no media variants), `apple-mobile-web-app-status-bar-style` `black-translucent`; manifest `theme_color` `#0047BB`, `background_color` `#EDF1FF`. | The band is blue in both schemes; the splash uses the brand default (pale). |
| D7 | Banner glyphs are CSS generated content with empty alternative text (`content: "!"; content: "!" / "";`), so screen readers do not read "i" / "!" (the first declaration is the fallback for engines without the alt syntax). | The text of the paragraph is the information. |
| D8 | The status dot is a decorative `aria-hidden="true"` span driven only by CSS from `#state.state-<s>`, which is the same state `app.js` writes as text in `#state`; it never carries information that is not already written in text. | Accessibility; no JavaScript change. |
| D9 | `apple-touch-icon.png` is generated deterministically (fixed command, no time chunks, only `IHDR`/`IDAT`/`IEND`); the candid fingerprint and the CI fingerprint of binary assets stay stable (ADR of `ea862cc`). | Reproducible binary. Verified in the scratchpad: two runs give identical bytes (2 742 bytes, RGB, 180 x 180). |
| D10 | Optional fix accepted in scope: `blank_string_literals` in `tests/invariants/src/remote_alerts_contract.rs` treats `br"…"`, `br#"…"#`, `cr"…"`, `cr#"…"#` as raw strings; new self-test; walkthrough 188 §R3.7 reworded (its claim "the failure direction is a false positive, never a hidden violation" is wrong: `br"C:\"; let acknowledged = true; let q = "x";` is scanned as one escaped string and hides the `acknowledged` binding, a false negative). Recorded as Contract Migration CM-1 (helper strengthened, no assertion weakened). | Owner-optional small fix; the scanner protects test 62. |
| D11 | **No push switch** (round 2, F3). The only CSS-observable push condition is `#push-disable` visible, which `app.js` sets from `view.subscriptions > 0`: *the PC has at least one registered device*, not *this phone receives alerts*; it stays true after `syncSubscription` drops this phone's stale subscription. A green switch would therefore state a per-device delivery guarantee the page does not have, in a failed-password alerting feature. The `#push` card keeps a plain `h2` `Notifications`; `#push-state` (written by `app.js`) stays the only statement of the push state. Tested (test 69). | Misleading security state is worse than a missing decoration; `app.js` is frozen (D1), so a per-device indicator is not possible here. |
| D12 | **Danger outline deviates from the GUI on purpose** (round 2, F8). The GUI `ButtonKind::DangerOutline` (`origin/main:crates/gui/src/widgets.rs` l.712–719) paints text and border in `DANGER` `#D92D45`, 4.22:1 on `PALE` (below AA for 17 px text). The web uses `--danger-fg` = `--danger-text` `#8A1426` (8.43:1) in light and `--danger-soft` (6.09:1 on `--ink-2`) in dark. A later parity review must not "fix" the web back; the GUI should follow (open point §15.4). | WCAG AA is an owner requirement for the phone. |
| D13 | **Body cut out of the band** (round 2, F6): `body` background `var(--band)`, flex column, `min-height: 100dvh`; `main.page` (`--page`, top radius `--r-body`) grows with `flex: 1`; `footer` background `var(--page)`. The rounded top corners of `main` thus sit on blue, like the GUI body under its band, and no blue shows below the content. | The round-1 frame put `main`'s corners on a pale body, so they were invisible. |

### 0.3 Deviations from the design brief

1. Banner role tokens are renamed `--banner-<kind>-<part>` (brief: `--success-bg-r`, `--warn-border-r`, …) so no role
   name can be confused with a palette name.
2. A role `--danger-fill-hover` (light and dark: `--danger-hover`) is added; the brief used the palette name directly.
3. Banner glyph alternative text (D7) is added.
4. Round 2: the brief was corrected in place for the tint values and rounding rule (F1), the switch removal (F3), the
   banner glyph colors (F10) and the band-colored body (F6); the brief and this spec now agree on those points.
5. Display sizes use `clamp()` (§2.4, F11) instead of the brief's fixed 64 px / 44 px.
6. Everything else in the brief stands (sizes, radii, components, wireframes §9, dark recipes §2.2, ratios §2.3).

## 1. Scope & Blast Radius

### 1.1 Files

| File | Change |
|---|---|
| `crates/remote/assets/index.html` | Rewritten markup (§4); all 29 ids, labels and attributes of §5 kept |
| `crates/remote/assets/style.css` | Rewritten (§2, §3) |
| `crates/remote/assets/icon.svg` | Replaced by the `Icon_logo.svg` mark (§6.1) |
| `crates/remote/assets/apple-touch-icon.png` | Regenerated from the new `icon.svg` (§6.2) |
| `crates/remote/assets/manifest.webmanifest` | `theme_color`, `background_color` only (§6.3) |
| `crates/remote/assets/app.js`, `sw.js` | **unchanged** |
| `crates/remote/src/*.rs` (incl. `assets.rs`, `http.rs`) | **unchanged** |
| `tests/invariants/src/remote_brand_contract.rs` | New (tests 63–74) |
| `tests/invariants/src/lib.rs` | Register `mod remote_brand_contract;` (`#[cfg(all(test, unix))]`, doc comment like the other remote modules) |
| `tests/invariants/src/remote_alerts_contract.rs` | CM-1: `blank_string_literals` raw prefixes + test 75 |
| `Docs/REMOTE_COMPANION.md` | New section `## 2e. Page design (soos brand)` after §2d (§10); §6 table row text unchanged |
| `AI/DECISIONS.md` | New ADR (drafted with this spec) |
| `AI/tester_contract_brand.md` | New (tester phase): test list, CM-1, any further migration |
| `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/188_remote_auth_alerts.md` (§R3.7), `AI/walkthroughs/190_remote_brand_redesign.md` | Traceability phase |

Not touched: `crates/gui/**` (the GUI brand lives on `main`; editing it here would conflict), `Docs/GUI_APPLICATION.md`,
packaging, installers, units, `deny.toml`, `Cargo.toml`/`Cargo.lock` (no dependency: the invariants crate stays
dependency-free; PNG checks parse chunk headers by hand).

### 1.2 Consumers of the changed files

- `crates/remote/src/assets.rs` (`include_bytes!` of every asset: unchanged paths).
- `crates/remote/tests/server_tests.rs::test_rmc_assets_are_served_with_content_types` (needles `app.js`,
  `style.css`, `manifest.webmanifest`, `apple-touch-icon.png`, PNG magic), `routes_tests.rs` (manifest `display`,
  `start_url`, `scope`; doctype) — all kept.
- `tests/invariants`: `test_rmc_s8_*` (no inline script body, no absolute URL in `script src`/`link href`/`img src`,
  no `onclick=`/`onload=`/`onerror=`, needles incl. `apple-mobile-web-app-capable` and the plain source `<a href>`,
  no remote `url()`, no `@import`, `prefers-color-scheme` in CSS), `test_rmc_unlock_is_opt_in_and_documented`
  (`id="unlock"`), `test_rmc_s18_*` (ids `login`, `login-button`, `enroll`, `enroll-code`, `enroll-button`, `logout`,
  `unlock`; no `domain=`), `test_rmc_s30_*` (`id="alerts"`), `test_rmc_s38_*` (`id="push"`), `test_rmc_s43_*`
  (no `.acknowledged` in CSS) — all kept by construction (§5, §7).
- `scripts/candid_review.sh` fingerprint (binary PNG: deterministic by D9).

### 1.3 Out of scope

`app.js`/`sw.js` behaviour, a manual theme toggle (would need storage), notification icons, a font file, any Rust
change, the GUI, packaging. A future frontend (docs site, installer UI) must reuse the same tokens (ADR item 1).

## 2. CSS Architecture (`style.css`)

### 2.1 File layout (in this order)

1. Header comment: purpose, ADR title, "tokens mirror `crates/gui/src/theme.rs`".
2. `:root` **palette** block — exactly the 25 declarations of §2.2, one per line, form `  --name: #RRGGBB;`
   (uppercase hex, 6 digits). Nothing else in this block.
3. `:root` **dark tints** block — exactly the 14 declarations of §2.2b, same form, each preceded on the same line or
   the line above by a comment `/* mix(A, B, t) */` (documentation only; the test recomputes).
4. `:root` **scale** block — typography, radii, strokes, spacing, motion tokens (§2.4) and `color-scheme: light dark`.
5. `:root` **light roles** block — every role of §2.3 as `--role: var(--palette-or-tint);`.
6. `@media (prefers-color-scheme: dark) { :root { … } }` — every role whose dark value differs, same form.
7. Base (reset, `[hidden] { display: none !important; }` kept, `html`, `body`), then components in the order of
   §3, then `@media (prefers-reduced-motion: reduce)`.

Rules:

- **Hex literals appear only in blocks 2 and 3**, only as the full value of a custom property. No `rgb(`, `rgba(`,
  `hsl(`, `hsla(`, `hwb(`, `lab(`, `lch(`, `oklab(`, `oklch(`, `color-mix(`, `color(`; no named color keyword
  except `transparent`, `currentColor`, `inherit`. Translucency uses the `opacity` property only.
- Outside blocks 2–3 (and outside the role blocks 5–6), a color-bearing property (`color`, `background`,
  `background-color`, `border*`, `outline*`, `fill`, `stroke`, `box-shadow`, `caret-color`, `accent-color`,
  `text-decoration-color`, `-webkit-tap-highlight-color`) is checked token by token (round 2, F4):
  - every `var(--x)` in the value names either a **role** of §2.3 or a **scale** token of §2.4 (e.g. `--stroke-card`,
    `--stroke-thin` for widths); a palette (§2.2) or tint (§2.2b) name is never allowed there;
  - every other word is a length, a number, a non-color keyword (`solid`, `inset`, `none`, `auto`, `0`, …) or one of
    the allowed color keywords `transparent`, `currentColor`, `inherit`;
  - `linear-gradient(` is allowed only in the `html` rule, with exactly `linear-gradient(var(--band) 50%, var(--page) 50%)`.
  Palette and tint names appear only as the value of a role declaration in blocks 5–6.
- No `url(` at all, no `@import`, no `@font-face`, no `.acknowledged`, no `outline: none` / `outline: 0` except in the
  single rule `:focus:not(:focus-visible)`.
- `box-shadow` is only used for inset borders (`inset 0 0 0 var(--stroke-card) var(--role)`), never for elevation.

### 2.2 Palette (identical to `origin/main:crates/gui/src/theme.rs`)

`--blue #0047BB`, `--blue-hover #1D5CC3`, `--blue-pressed #003A99`, `--pale #EDF1FF`, `--pale-2 #E4EAFD`,
`--pale-3 #DCE4FC`, `--pale-4 #C9D6FA`, `--line #C9D0E1`, `--ink #101820`, `--ink-muted #5A6270`, `--ink-weak #7A818E`,
`--pink #E59BDC`, `--white #FFFFFF`, `--success #29B53B`, `--success-toggle #46BE82`, `--success-bg #E6F6EA`,
`--success-text #146C2A`, `--danger #D92D45`, `--danger-hover #B81F35`, `--danger-bg #FDE8EB`, `--danger-text #8A1426`,
`--warn #F59E0B`, `--warn-border #F5B547`, `--warn-bg #FFF4E0`, `--warn-text #8A4B00`.

Not declared (no use on the phone): `FACE_LIVE`, `FACE_SPOOF`, `EYE`, `NOSE`, `MOUTH`, `EYE_AXIS`, `GUIDE`, `PAD_BOX`,
`INSET_LABEL`, `SHADOW`.

### 2.2b Dark tints (web only)

Rule (round 2, F1): `t = n / 100` with `n` an integer percentage. Each 8-bit sRGB channel is computed in **integer
arithmetic** as

```text
mix(a, b, n) = (a · (100 − n) + b · n + 50) / 100      (u32, truncating division)
```

which is `a + (b − a) · t` rounded half up; since channels are non-negative this equals half away from zero, and exact
`.5` results (all three channels of `--pale-weak`, the green channel of `--blue-soft`) round up with no floating-point
sensitivity. The test (64) uses exactly this formula; no `f64` is involved.

| Token | Recipe | Hex |
|---|---|---|
| `--ink-2` | `mix(INK, PALE, 5)` | `#1B232B` |
| `--ink-3` | `mix(INK, PALE, 10)` | `#262E36` |
| `--ink-4` | `mix(INK, PALE, 16)` | `#333B44` |
| `--ink-line` | `mix(INK, PALE, 22)` | `#414851` |
| `--pale-muted` | `mix(PALE, INK, 30)` | `#ABB0BC` |
| `--pale-weak` | `mix(PALE, INK, 50)` | `#7F8590` |
| `--blue-edge` | `mix(BLUE, PALE, 30)` | `#477ACF` |
| `--blue-soft` | `mix(BLUE, PALE, 55)` | `#82A5E0` |
| `--danger-soft` | `mix(DANGER, PALE, 45)` | `#E28599` |
| `--danger-bg-dark` | `mix(INK, DANGER, 18)` | `#341C27` |
| `--success-soft` | `mix(SUCCESS, PALE, 45)` | `#81D093` |
| `--success-bg-dark` | `mix(INK, SUCCESS, 16)` | `#143124` |
| `--warn-soft` | `mix(WARN, PALE, 35)` | `#F2BB60` |
| `--warn-bg-dark` | `mix(INK, WARN, 16)` | `#352D1D` |

All 14 values were recomputed in round 2 with the integer rule above (script in the scratchpad) and agree with the
evaluator's independent computation; the round-1 values `#7E8490` and `#82A4E0` were wrong and are corrected here and
in the brief. Contrast pairs that use the two corrected tints (dark theme): `--link`/`--focus` (`--blue-soft`) 7.16 on
`--ink`, 6.36 on `--ink-2`; `--secondary-fg` 5.51 on `--ink-3`, 4.55 on `--ink-4`; `--input-border` (`--pale-weak`)
4.82 on `--ink`; `--dot-idle` 4.28 on `--ink-2`; disabled text 3.06 on `--ink-4` (exempt). All still meet §8.

### 2.3 Role tokens (light in `:root`, dark in the media block)

| Role | Light | Dark |
|---|---|---|
| `--page` | `--pale` | `--ink` |
| `--band` | `--blue` | `--blue` |
| `--on-band` | `--pale` | `--pale` |
| `--card` | `--pale` | `--ink-2` |
| `--card-border` | `--blue` | `--blue-edge` |
| `--text` | `--ink` | `--pale` |
| `--text-muted` | `--ink-muted` | `--pale-muted` |
| `--text-disabled` | `--ink-weak` | `--pale-weak` |
| `--link` | `--blue` | `--blue-soft` |
| `--table` | `--pale-2` | `--ink-3` |
| `--divider` | `--line` | `--ink-line` |
| `--tile` | `--blue` | `--blue` |
| `--on-tile` | `--pale` | `--pale` |
| `--strip` | `--pale` | `--ink-2` |
| `--on-strip` | `--ink` | `--pale` |
| `--primary` / `--on-primary` | `--blue` / `--pale` | `--blue` / `--pale` |
| `--primary-hover` / `--primary-pressed` | `--blue-hover` / `--blue-pressed` | same |
| `--secondary-fg` | `--blue` | `--blue-soft` |
| `--secondary-hover` / `--secondary-pressed` | `--pale-3` / `--pale-4` | `--ink-3` / `--ink-4` |
| `--danger-fill` / `--danger-fill-hover` / `--on-danger` | `--danger` / `--danger-hover` / `--white` | same |
| `--danger-fg` | `--danger-text` | `--danger-soft` |
| `--danger-outline-hover` | `--danger-bg` | `--danger-bg-dark` |
| `--disabled-fill` / `--disabled-fg` | `--line` / `--ink-weak` | `--ink-4` / `--pale-weak` |
| `--input-bg` / `--input-border` / `--placeholder` | `--white` / `--ink-weak` / `--ink-muted` | `--ink` / `--pale-weak` / `--pale-muted` |
| `--focus` / `--focus-on-blue` | `--blue` / `--pink` | `--blue-soft` / `--pink` |
| `--banner-info-bg` / `-border` / `-fg` / `-icon` | `--pale-2` / `--line` / `--ink` / `--blue` | `--ink-3` / `--ink-line` / `--pale` / `--blue-soft` |
| `--banner-success-bg` / `-border` / `-fg` / `-icon` | `--success-bg` / `--success` / `--success-text` / `--success-text` | `--success-bg-dark` / `--success-soft` / `--success-soft` / `--success-soft` |
| `--banner-warn-bg` / `-border` / `-fg` / `-icon` | `--warn-bg` / `--warn-border` / `--warn-text` / `--warn-text` | `--warn-bg-dark` / `--warn-soft` / `--warn-soft` / `--warn-soft` |
| `--banner-danger-bg` / `-border` / `-fg` / `-icon` | `--danger-bg` / `--danger` / `--danger-text` / `--danger-text` | `--danger-bg-dark` / `--danger-soft` / `--danger-soft` / `--danger-soft` |
| `--dot-locked` / `--dot-unlocked` / `--dot-alarm` / `--dot-idle` | `--success-text` / `--warn-text` / `--danger-text` / `--ink-weak` | `--success` / `--warn` / `--danger-soft` / `--pale-weak` |
| `--star` | `--pink` | `--pink` |

A role value is always exactly `var(--x)` with `--x` a palette or tint token (no chains of roles), so the test resolves
every role in two lookups.

`--success-toggle` stays declared in the palette (parity with `theme.rs`) but no role uses it (no switch, D11).

Forbidden pairs (brief §2.3; enforced by test 66 through the pairs it checks **and** the selector-to-role bindings it
pins, round 2 F5): danger text on pale
(`--danger`, 4.22), `--success`/`--warn` text or dots on pale, any text on pink, `--blue` text/borders on ink surfaces,
`--danger` text on `--ink-2`.

### 2.4 Scale tokens

Typography (brief §3): `--font: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto,
"Helvetica Neue", Arial, sans-serif;` `--font-mono: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;`
`--f-display: clamp(48px, 16vw, 64px)`, `--f-display-long: clamp(32px, 11vw, 44px)` (round 2, F11: 62 px / 42 px at
390 px, 48 px / 35 px at 320 px), `--f-title: 20px`, `--f-section: 17px`, `--f-body: 16px`,
`--f-button: 17px`, `--f-small: 13px`, `--f-caption: 12px`, `--f-code: 18px`. Inputs never below 16 px (no Safari
zoom).

Radii and strokes (GUI names): `--r-body: 20px` (`BODY_TOP_RADIUS`), `--r-card: 20px` (`R_CARD`), `--r-table: 14px`
(`R_TABLE`), `--r-banner: 12px`, `--r-input: 10px` (`R_INPUT`), `--r-pill: 999px`, `--stroke-card: 2px`
(`STROKE_CARD`), `--stroke-thin: 1px`. Spacing: `--gap-card: 14px` (`GAP_CARD`), `--pad-card: 20px`, `--gap-item: 8px`,
`--gutter: max(16px, env(safe-area-inset-left), env(safe-area-inset-right))`, `--band-h: 56px`, `--strip-h: 44px`,
`--touch: 48px`. Motion: `--t-fast: 120ms`.

The numeric GUI-named tokens (`--r-body`, `--r-card`, `--r-table`, `--r-input`, `--stroke-card`, `--gap-card`) carry
the `theme.rs` values (20, 20, 14, 10, 2, 14) and are pinned by test 63.

## 3. Components (selectors → roles)

| Component | Selector(s) | Spec |
|---|---|---|
| Page | `html`, `body` | `html` band/page gradient (overscroll); `body` `background: var(--band)` (D13), `display: flex; flex-direction: column; min-height: 100dvh`, `color: var(--text)`, `font-family: var(--font)`, margin 0, `-webkit-text-size-adjust: 100%`, `-webkit-tap-highlight-color: transparent` |
| Header band | `.band` | `background: var(--band)`; `padding-top: env(safe-area-inset-top)`; inner row height `--band-h`, side padding `--gutter`, flex, space-between, center |
| Wordmark | `.wordmark` (svg), `.wordmark path` | `height: 18px; width: auto;` `fill: var(--on-band)` |
| Header pill | `.band-pill` | `background: var(--card)`, `color: var(--text)`, `--f-caption` 700 uppercase, padding 6 px 12 px, `--r-pill` (static text, not interactive) |
| Visually hidden | `.visually-hidden` | standard clip pattern (position absolute, 1 px, clip, overflow hidden, white-space nowrap) |
| Body | `main.page` | `flex: 1 0 auto`; `width: 100%`; `box-sizing: border-box`; `background: var(--page)`; `border-radius: var(--r-body) var(--r-body) 0 0`; `margin: -1px auto 0`; `max-width: 480px` (on wider screens the band color shows on both sides of the body, as in the GUI); padding `20px var(--gutter) calc(24px + env(safe-area-inset-bottom))`; flex column, `gap: var(--gap-card)` |
| Card | `.card` | `background: var(--card)`; `border: var(--stroke-card) solid var(--card-border)`; `border-radius: var(--r-card)`; `padding: var(--pad-card)`; text left |
| Card title | `.card h2` | `--f-title` 700, `color: var(--text)`, margin 0 0 8 px |
| Detail | `.detail` | `--f-body`, `color: var(--text-muted)` |
| Stat tile | `.stat-tile` | `background: var(--tile)`; `color: var(--on-tile)`; `--r-card`; min-height 196 px; overflow hidden; position relative; padding-bottom 20 px |
| Tile strip | `.stat-strip` | height `--strip-h`; `background: var(--strip)`; `color: var(--on-strip)`; margin `var(--stroke-card)`; radius `calc(var(--r-card) - var(--stroke-card))` top corners; caption `--f-caption`; flex row space-between |
| Status dot | `.stat-dot` | 10 px circle, `background: var(--dot-idle)`; `.stat-tile:has(.state-locked) .stat-dot { background: var(--dot-locked) }`, `:has(.state-unlocked)` → `--dot-unlocked`, `:has(.state-unavailable)`, `:has(.state-unreachable)` → `--dot-alarm`; `no_session`/`unknown` keep idle |
| State value | `#state` (via `.state`, `.state-*`) | `--f-display` 800, line-height 1, letter-spacing −0.02em for `.state-locked`, `.state-unlocked`; `--f-display-long` for every other `.state-*`; `.state-unknown`, `.state-unreachable` `opacity: 0.7`; color `var(--on-tile)`; margin 20 px 20 px 6 px; `overflow-wrap: anywhere` (F11: never wider than the tile) |
| Tile lines | `.stat-tile .detail` (`#activity` 0.9, `#updated` 0.8 opacity, `--f-small` for `#updated`, `tabular-nums`) | `color: var(--on-tile)`; padding-right 76 px so text never runs under the star |
| Tile star | `.stat-star` (svg) | 56 px, absolute bottom 16 px right 16 px, `fill: var(--star)` |
| Buttons | `button` | full width; `min-height: var(--touch)`; padding 12 px 20 px; `--r-pill`; `--f-button` 600; `background: var(--primary)`; `color: var(--on-primary)`; border 0; `touch-action: manipulation`; transitions `background-color, box-shadow, transform var(--t-fast)`; `:hover` `--primary-hover`; `:active` `--primary-pressed` + `transform: scale(0.98)`; buttons stacked 12 px apart (`.card > button + button`, `.actions` flex column gap 12 px) |
| Secondary | `button.secondary` | `background: transparent`; `color: var(--secondary-fg)`; `box-shadow: inset 0 0 0 var(--stroke-card) var(--secondary-fg)`; hover/active fills `--secondary-hover`/`--secondary-pressed` |
| Danger outline | `button.secondary.danger` | `color: var(--danger-fg)`; inset border `var(--danger-fg)`; hover/active `--danger-outline-hover` (deliberate deviation from the GUI `DANGER` outline, D12) |
| Danger filled | `button.danger:not(.secondary)` | `background: var(--danger-fill)`, `color: var(--on-danger)`, hover `--danger-fill-hover` (declared for completeness; unused today) |
| Disabled | `button:disabled` (+ outline variants) | `background: var(--disabled-fill)`, `color: var(--disabled-fg)`, `cursor: default`, no transform; outline variants: transparent with inset `var(--divider)`; **no opacity trick** |
| Banner | `.banner`, `.banner::before` | `--r-banner`; `border: var(--stroke-thin) solid var(--banner-<k>-border)`; `background: var(--banner-<k>-bg)`; `color: var(--banner-<k>-fg)`; padding 12 px 14 px 12 px 44 px; position relative; `--f-section` 600; glyph (F10): 20 px circle at left 14 px, `background: var(--banner-<k>-icon)`, glyph character `color: var(--banner-<k>-bg)` (a cut-out of the banner background, ≥ 3:1 on the circle in every kind and theme, test 66), `--f-small` 700, `content` per D7 |
| Info / attention | `#alerts-summary` (`.alerts-summary.banner`) default info roles; `#alerts.alerts-attention .alerts-summary` danger roles | glyph "i" info, "!" danger |
| Warn | `#alerts-coverage.banner.banner-warn` | warn roles, glyph "!" |
| Success | `.banner-success` | declared, unused today (brief §5.6 note) |
| Inner table | `.table` (on `#alerts-history`, `#push-devices`) | `background: var(--table)`; `--r-table`; padding 4 px 14 px; `list-style: none`; margin 12 px 0 0; `li` `--f-body` `color: var(--text)`, min-height 44 px, padding 10 px 0, `border-bottom: var(--stroke-thin) solid var(--divider)`, none on `:last-child`; `font-variant-numeric: tabular-nums`; `.table:empty { display: none; }` |
| Input | `#enroll-code` (`input`) | full width, min-height 52 px, `background: var(--input-bg)`, `color: var(--text)`, `border: var(--stroke-card) solid var(--input-border)`, `--r-input`, `--f-code` 600 mono uppercase, letter-spacing 0.12em, centered; `::placeholder { color: var(--placeholder); opacity: 1; }`; `:focus` border `var(--focus)` |
| Code chips | `code` | `font-family: var(--font-mono)`; `background: var(--table)`; padding 1 px 5 px; radius 6 px; `color: var(--text)` |
| Feedback | `.feedback` | `--f-small`, `color: var(--text-muted)`, `min-height: 1.4em`, margin 12 px 0 0 |
| Login tile | `.login-tile` (svg wrapper) | 96 x 96, `background: var(--tile)`, `--r-card`, centered, star 56 px `fill: var(--star)` |
| Footer | `footer` | `flex: 0 0 auto`; centered, `--f-small`, padding `12px var(--gutter) calc(12px + env(safe-area-inset-bottom))`, `background: var(--page)`; `footer a` `color: var(--link)`, underline, `display: inline-flex; align-items: center; min-height: 44px; padding: 0 12px` |
| Focus | `:focus-visible` on `button`, `a`, `input` | `outline: 3px solid var(--focus); outline-offset: 3px;` and `:focus:not(:focus-visible) { outline: none; }` |
| Reduced motion | `@media (prefers-reduced-motion: reduce)` | `*, *::before, *::after { transition: none !important; }` and no `transform` on `:active` |

## 4. Markup (`index.html`)

### 4.1 Head

Unchanged except: `apple-mobile-web-app-status-bar-style` content `black-translucent`; `theme-color` content `#0047BB`.
Kept verbatim: charset, `viewport` (with `viewport-fit=cover`), `apple-mobile-web-app-capable`, `apple-mobile-web-app-title`
`soos`, `color-scheme` `light dark`, `<title>soos remote</title>`, the manifest, icon (`icon.svg`, `image/svg+xml`),
apple-touch-icon and stylesheet links. No `<style>`, no `<script>` in the head.

### 4.2 Body skeleton (ids kept, new wrappers in bold)

```html
<body>
  <header class="band">                                          <!-- new -->
    <div class="band-row">
      <h1 class="brand">
        <svg class="wordmark" viewBox="0 0 514 64" aria-hidden="true" focusable="false">
          <path d="…6 paths of text_logo.svg, d strings verbatim…"/>
        </svg>
        <span class="visually-hidden">soos remote</span>
      </h1>
      <span class="band-pill">Remote</span>
    </div>
  </header>
  <main class="page" aria-live="polite">                           <!-- was class="card"; h1 moved out -->
    <section id="login" class="card login" hidden>
      <div class="login-tile" aria-hidden="true"><svg class="star" viewBox="0 0 508 508" focusable="false">STAR</svg></div>
      <h2>Sign in</h2>                                             <!-- new static heading -->
      <p class="detail">Sign in with your passkey to reach this PC over the internet.</p>
      <button id="login-button" type="button">Sign in with Face ID</button>
      <p id="login-feedback" class="feedback" role="status">&nbsp;</p>
    </section>
    <section id="status-card" class="status">
      <div class="stat-tile">
        <div class="stat-strip"><span class="stat-caption">PC status</span><span class="stat-dot" aria-hidden="true"></span></div>
        <p id="state" class="state state-unknown" data-state="unknown">Connecting</p>
        <p id="activity" class="detail">&nbsp;</p>
        <p id="updated" class="detail">&nbsp;</p>
        <svg class="stat-star star" viewBox="0 0 508 508" aria-hidden="true" focusable="false">STAR</svg>
      </div>
      <div class="card actions">
        <button id="lock" type="button" disabled>Lock now</button>
        <button id="unlock" type="button" class="secondary danger" disabled>Unlock now</button>
        <p id="feedback" class="feedback" role="status">&nbsp;</p>
        <button id="logout" type="button" class="secondary" hidden>Sign out</button>
      </div>
    </section>
    <section id="alerts" class="card alerts" hidden aria-live="polite">
      <h2>Failed passwords on the PC</h2>
      <p id="alerts-summary" class="alerts-summary banner">&nbsp;</p>
      <p id="alerts-coverage" class="banner banner-warn" hidden>&nbsp;</p>
      <ul id="alerts-history" class="alerts-history table"></ul>
      <button id="alerts-ack" type="button" class="secondary" hidden>Acknowledge</button>
      <p id="alerts-feedback" class="feedback" role="status">&nbsp;</p>
    </section>
    <section id="push" class="card push" hidden aria-live="polite">
      <h2>Notifications</h2>                                       <!-- plain heading, no switch (D11) -->
      <p id="push-state" class="detail">&nbsp;</p>
      <ul id="push-devices" class="push-devices table"></ul>
      <p id="push-hint" class="detail" hidden>…text verbatim…</p>
      <button id="push-enable" type="button" hidden>Enable notifications</button>
      <button id="push-test" type="button" class="secondary" hidden>Send test notification</button>
      <button id="push-disable" type="button" class="secondary" hidden>Disable notifications</button>
      <p id="push-feedback" class="feedback" role="status">&nbsp;</p>
    </section>
    <section id="enroll" class="card enroll" hidden>
      <h2>Add a passkey</h2>
      <p class="detail">Run <code>soos-remote enroll-code</code> on the PC, then type the code.</p>
      <input id="enroll-code" …all attributes verbatim…>
      <button id="enroll-button" type="button">Add this device's passkey</button>
      <p id="enroll-feedback" class="feedback" role="status">&nbsp;</p>
    </section>
  </main>
  <footer>
    <a href="https://github.com/Mysticaly622/soos">source</a>
  </footer>
  <script src="app.js"></script>
</body>
```

`STAR` = the two `brand.rs` `STAR_SPIKES` paths: `<path d="M254 0C254 140.28 367.72 254 508 254L254 254Z"/>` and
`<path d="M0 254C140.28 254 254 367.72 254 508V254H0Z"/>`. Inline SVGs carry **no** `fill`, `stroke`, `style`,
`class` color or `xmlns:xlink`; colors come from `style.css`. No `<use>`, `<image>`, `<foreignObject>`, `href`,
`xlink:href`, `on*` attribute or `<script>` inside any SVG.

### 4.3 Per state (what is visible; driven by unchanged `app.js`)

| State | Visible | Look |
|---|---|---|
| Connecting (first paint) | tile (`Connecting`, idle dot, 70 %), actions card with both buttons disabled | §9.6 of the brief |
| Locked | tile `Locked` (locked dot); `#lock` disabled (grey pill), `#unlock` enabled (danger outline) | brief §9.1 |
| Unlocked | tile `Unlocked` (unlocked dot); `#lock` enabled (blue), `#unlock` disabled outline | brief §9.2 |
| No session / Unavailable / Unreachable | long display size; idle / alarm / alarm dot; Unreachable 70 % | brief §9.6 |
| Funnel login | only `#login` card (star tile, "Sign in", detail, primary button, feedback); `#status-card`, `#alerts`, `#push`, `#enroll`, `#logout` hidden | brief §9.3 |
| Funnel signed in | `#logout` secondary at the bottom of the actions card | |
| Alerts quiet | `#alerts` card, info banner, empty table collapsed, no Acknowledge | |
| Alerts with attempts | `#alerts.alerts-attention`: danger banner, table rows, Acknowledge secondary, optional warn banner | brief §9.4 |
| Push disabled / unsupported | card with state sentence only | |
| Push enabled | devices table, hint, three buttons; no switch (D11) | brief §9.5 |
| Enrollment | `#enroll` card with code input and primary button | |

The existing `[hidden] { display: none !important; }` stays, so no wrapper rule can reveal a hidden element.

## 5. UI Contract Kept (tested by test 69)

- **29 ids**, each exactly once, on the same element type as today: `section`: `login`, `status-card`, `alerts`,
  `push`, `enroll`; `p`: `login-feedback`, `state`, `activity`, `updated`, `feedback`, `alerts-summary`,
  `alerts-coverage`, `alerts-feedback`, `push-state`, `push-hint`, `push-feedback`, `enroll-feedback`; `ul`:
  `alerts-history`, `push-devices`; `button`: `login-button`, `lock`, `unlock`, `logout`, `alerts-ack`, `push-enable`,
  `push-test`, `push-disable`, `enroll-button`; `input`: `enroll-code`. No other `id` attribute in the page (the
  `getElementById` set of `app.js` equals this set).
- **Button labels** (exact text content): `Sign in with Face ID`, `Lock now`, `Unlock now`, `Sign out`, `Acknowledge`,
  `Enable notifications`, `Send test notification`, `Disable notifications`, `Add this device's passkey`; every button
  `type="button"`.
- **Initial attributes**: `hidden` on `login`, `logout`, `alerts`, `alerts-coverage`, `alerts-ack`, `push`, `push-hint`,
  `push-enable`, `push-test`, `push-disable`, `enroll`, and not on `status-card`; `disabled` on `lock`, `unlock`;
  `role="status"` on the five `*feedback` paragraphs; `aria-live="polite"` on `main`, `#alerts`, `#push`; `#state` has
  `class="state state-unknown"`, `data-state="unknown"`, text `Connecting`; `#unlock` has class `secondary`.
- **Texts kept verbatim**: `h2` `Failed passwords on the PC`, `Notifications`, `Add a passkey`; the login detail
  sentence; the enroll detail sentence with its `<code>`; the full `#push-hint` paragraph; `#enroll-code` attributes
  (`type="text"`, `inputmode="text"`, `autocomplete="one-time-code"`, `autocapitalize="characters"`,
  `spellcheck="false"`, `maxlength="11"`, `placeholder="XXXXX-XXXXX"`, `aria-label="Enrollment code"`); `<title>soos
  remote</title>`; the accessible `h1` text `soos remote`.
- **No push switch** (D11): no element with a class `switch` or `switch-knob`, no `role="switch"`, no
  `aria-checked` in `index.html`; no `.switch` selector and no `:has(#push-disable` selector in `style.css`.
- **Kept needles of existing tests** (§1.2): one `<script src="app.js"></script>` at the end of `body`, no other
  script; the plain source `<a href="https://github.com/Mysticaly622/soos">source</a>`.

Additive, unpinned changes (no Contract Migration): wrappers `header.band`, `.stat-tile`, `.actions`,
classes `card`, `table`, `banner`, `banner-warn`, `danger` on `#unlock`, `#alerts-coverage` class `detail` →
`banner banner-warn`, the static `Sign in` heading, `PC status` caption and `Remote` pill, inline SVGs, meta values,
manifest colors.

## 6. Icons and Manifest

### 6.1 `icon.svg`

Exactly the owner's `Icon_logo.svg`: `<svg width="508" height="508" viewBox="0 0 508 508" fill="none"
xmlns="http://www.w3.org/2000/svg">`, `<rect width="508" height="508" fill="#EDF1FF"/>` and the four blue paths
(`M0 0H254V254H0V0Z`, `M254 254H508V508H254V254Z`, `M0 254C140.28 254 254 367.72 254 508H0V254Z`,
`M254 0L508 0V254C367.72 254 254 140.28 254 0Z`, each `fill="#0047BB"`). Only those two colors. No script, `style`,
`href`, `<image>`, `<foreignObject>`, comment with paths of other files. It is also the favicon (link kept).

### 6.2 `apple-touch-icon.png`

Generated from the repository `icon.svg` (run from the scratchpad, output copied in):

```bash
rsvg-convert -w 180 -h 180 crates/remote/assets/icon.svg \
  | magick png:- -background '#EDF1FF' -alpha remove -alpha off -strip \
      -define png:color-type=2 -define png:exclude-chunks=date,time,tIME \
      png:"$SCRATCH/apple-touch-icon.png"
```

Acceptance: 180 x 180, bit depth 8, color type 2 (RGB, no alpha channel), no `tRNS`, chunks exactly `IHDR`, `IDAT`,
`IEND`, at most 8 KiB, identical bytes on a second run (architect run: 2 742 bytes, SHA-256 `3442af18…`). The mark is
full bleed (iOS rounds the corners). No raster from the archive is used.

### 6.3 `manifest.webmanifest`

Only `"background_color": "#EDF1FF"` and `"theme_color": "#0047BB"` change. `name`, `short_name`, `description`,
`display: standalone`, `start_url: /`, `scope: /` and both icon entries stay byte-identical.

## 7. CSP and Security

- CSP string in `http.rs` unchanged. Inline SVG is markup, not script or style; no `style=""`, no `<style>`, no inline
  event handler, no `javascript:`/`data:`/`blob:` URL anywhere in `index.html`, `style.css`, `icon.svg`.
- No network fetch added: no `url(`, `@import`, `@font-face`, web font, CDN. The asset directory contains exactly the
  seven files (`index.html`, `app.js`, `style.css`, `sw.js`, `manifest.webmanifest`, `icon.svg`,
  `apple-touch-icon.png`); no font, image or archive file is added.
- `app.js`/`sw.js` unchanged → `textContent` only, no storage, same service-worker behaviour.
- No sensitive data reaches markup or CSS (static page). The redesign changes no authorization, CSRF, rate limit or
  logging path.

## 8. Accessibility Rules

1. Text contrast ≥ 4.5:1 for every text role on its surfaces in both themes (test 66 list, §9 test 66); disabled
   text exempt. Large text (tile value, 44 px+ bold) with 70 % opacity stays ≥ 3:1 (4.27 measured).
2. Non-text control boundaries ≥ 3:1: input border on input background, focus ring on page/card, outline buttons'
   border, state dots on the strip, banner glyph on its circle (F10).
3. Focus: `:focus-visible` 3 px ring in `--focus`, offset 3 px; never removed except for pointer focus.
4. Touch: every `button` `min-height: var(--touch)` (48 px), input 52 px, footer link 44 px; full-width buttons.
5. Semantics: one `h1` (accessible name `soos remote`), `h2` per card, decorative SVGs and spans `aria-hidden="true"`
   and `focusable="false"`, banner glyphs with empty alt text (D7), live regions unchanged.
6. Motion: transitions ≤ 120 ms and none under `prefers-reduced-motion: reduce`.
7. Zoom: no `maximum-scale`/`user-scalable=no` in the viewport (kept as today).

## 9. Tests for the Tester (Phase 2)

New module `tests/invariants/src/remote_brand_contract.rs` (dependency-free; reuse `read`, `exists`,
`workspace_root` from `remote_companion_contract`; parsing helpers local to the module, each with a self-test where
the helper decides pass/fail). Shared fixtures in the test module: `PALETTE: [(&str /*css*/, &str /*theme.rs*/, u32); 25]`
copied from `origin/main:crates/gui/src/theme.rs` (`47ab53e`), `DARK_TINTS: [(&str /*css*/, &str /*a*/, &str /*b*/, u32 /*n, percent*/, u32 /*hex*/); 14]` (integer
percentages, §2.2b), `THEME_EXCERPT_47AB53E: &str` (verbatim lines of `origin/main:crates/gui/src/theme.rs` at
`47ab53e`: the doc comment and declaration of `BLUE`, `PALE`, `PINK`, `WHITE`, `DANGER_TEXT`, `BODY_TOP_RADIUS`, `GAP_CARD`,
`R_CARD`, `R_TABLE`, `R_INPUT`, `STROKE_CARD`, plus `EYE_AXIS` (`from_rgba_premultiplied`, not one of the parsed names)
and `R_VIDEO`/`R_BUTTON` (names sharing a prefix)),
`WORDMARK_PATHS: [&str; 6]` and `ICON_PATHS: [&str; 4]` (from the owner SVGs, equal to `brand.rs`),
`STAR_SPIKE_PATHS: [&str; 2]`.

| # | Test | Asserts |
|---|---|---|
| 63 | `test_rmc_s44_brand_tokens_match_the_gui_theme` | `style.css` declares each of the 25 palette tokens exactly once with the pinned hex (case-insensitive compare, uppercase required by a separate check); the six numeric tokens of §2.4 carry 20/20/14/10/2/14 px; **if** `crates/gui/src/theme.rs` exists, a local parser `theme_constant(src, name)` finds the single line `pub const <NAME>: <TYPE> = <EXPR>;` whose name matches **exactly** (followed by `:`, so `R_CARD` never matches `R_CARD_X`) and accepts exactly these forms (round 2, F2): `Color32` with `Color32::from_rgb(c, c, c)` where each `c` is a `0x` hex byte or a decimal `0..=255` literal, `Color32` with `Color32::WHITE` (→ `#FFFFFF`) or `Color32::BLACK` (→ `#000000`); `u8` (or `u16`/`u32`/`usize`) with a decimal integer literal; `f32` with a decimal literal `<int>` or `<int>.<digits>` (e.g. `2.0`, `14.0`). Each of the 25 color names must parse and equal the pinned hex; `BODY_TOP_RADIUS`, `R_CARD`, `R_TABLE`, `R_INPUT`, `STROKE_CARD`, `GAP_CARD` must parse and equal 20, 20, 14, 10, 2, 14 exactly (an `f32` such as `2.5` fails). A missing name, a duplicate, or any other form (`from_rgba_premultiplied`, `from_rgb` with an expression, a `const fn` call) fails with the constant's name in the message; it is never skipped. Helper self-test on `THEME_EXCERPT_47AB53E`: `BLUE` → `0x0047BB`, `WHITE` → `0xFFFFFF`, `PINK` → `0xE59BDC`, `BODY_TOP_RADIUS` → 20 (`u8`), `STROKE_CARD` → 2 (`f32` `2.0`), `GAP_CARD` → 14 (`f32` `14.0`), `R_CARD` → 20 despite `R_VIDEO`/`R_BUTTON` nearby; `EYE_AXIS` → error (unsupported form); `NOT_THERE` → error (missing); `pub const R_CARD: f32 = 2.5;` → value 2.5 (then unequal to 20). |
| 64 | `test_rmc_s45_style_colors_come_from_the_palette_only` | Every `#` hex literal in `style.css` (comments stripped) sits on a `--token: #RRGGBB;` line of the palette or tint set; no functional color notation of §2.1; no named color keyword (list: `white`, `black`, `red`, `blue`, `gray`, `grey`, `green`, `orange`, `pink`, `yellow`, `silver`, `navy`, …, matched as whole words in property values); each of the 14 tints is declared once and equals `mix(a, b, n)` recomputed from the pinned palette **in integer arithmetic** exactly as §2.2b (`(a·(100−n) + b·n + 50) / 100`, `u32`; no float), with a helper self-test `mix(PALE, INK, 50) == 0x7F8590`, `mix(BLUE, PALE, 55) == 0x82A5E0`, `mix(INK, PALE, 0) == INK`, `mix(INK, PALE, 100) == PALE`; outside the token and role blocks, color-bearing properties follow the token-by-token rule of §2.1 (every `var()` is a §2.3 role or a §2.4 scale token, never a palette or tint name; other words are lengths, numbers, non-color keywords or `transparent`/`currentColor`/`inherit`; `linear-gradient(` only in `html`), with a helper self-test: `border: var(--stroke-card) solid var(--card-border)` accepted, `color: var(--blue)` rejected, `box-shadow: inset 0 0 0 var(--stroke-card) var(--ink-2)` rejected, `color: red` rejected; `index.html` contains no hex literal other than `#0047BB` (theme-color); `icon.svg` only `#EDF1FF` and `#0047BB`. |
| 65 | `test_rmc_s46_light_default_and_dark_variant` | Light `:root` defines every role of §2.3 with the light value; exactly one `@media (prefers-color-scheme: dark)` block whose `:root` defines at least `--page: var(--ink)`, `--text: var(--pale)`, `--card: var(--ink-2)`, `--card-border: var(--blue-edge)`, `--link: var(--blue-soft)`, `--focus: var(--blue-soft)` and every other differing role of §2.3; `color-scheme: light dark` in `:root`; `<meta name="color-scheme" content="light dark">` kept. |
| 66 | `test_rmc_s47_text_contrast_meets_wcag_aa` | Resolves roles → hex for both themes and computes WCAG 2.x ratios (sRGB, 0.04045 threshold). Text ≥ 4.5: `text`/{`page`,`card`,`table`,`input-bg`}, `text-muted`/{`page`,`card`,`table`}, `link`/{`page`,`card`}, `on-band`/`band`, `on-tile`/`tile`, `on-strip`/`strip`, `on-primary`/{`primary`,`primary-hover`,`primary-pressed`}, `secondary-fg`/{`card`,`secondary-hover`,`secondary-pressed`}, `on-danger`/{`danger-fill`,`danger-fill-hover`}, `danger-fg`/{`card`,`danger-outline-hover`}, `placeholder`/`input-bg`, `banner-<k>-fg`/`banner-<k>-bg` for info, success, warn, danger. Non-text ≥ 3: `input-border`/`input-bg`, `focus`/{`page`,`card`}, `dot-*`/`strip`, `secondary-fg`/`card`, `danger-fg`/`card`, `banner-<k>-bg`/`banner-<k>-icon` for the four kinds (glyph on circle, F10). **Selector-to-role bindings** (round 2, F5; the declared value must be exactly the listed `var()`, checked in the rule whose selector list contains the selector): `.band` `background: var(--band)`; `.stat-tile` `background: var(--tile)`, `color: var(--on-tile)`; `.stat-strip` `background: var(--strip)`, `color: var(--on-strip)`; `button` `background: var(--primary)`, `color: var(--on-primary)`; `button.secondary` `color: var(--secondary-fg)`; `button.secondary.danger` `color: var(--danger-fg)`; `.alerts-summary` `background: var(--banner-info-bg)`, `color: var(--banner-info-fg)`; `.alerts-attention .alerts-summary` `background: var(--banner-danger-bg)`, `color: var(--banner-danger-fg)`; `.banner-warn` `background: var(--banner-warn-bg)`, `color: var(--banner-warn-fg)`; `footer a` `color: var(--link)`; `.card` `background: var(--card)`; no rule anywhere sets `color:` to `var(--danger-fill)`, `var(--dot-unlocked)`, `var(--dot-locked)` or `var(--star)` (fill-only roles). Helper self-test: ratio(`#101820`,`#EDF1FF`) ∈ [15.86, 15.88], ratio(`#FFFFFF`,`#D92D45`) ∈ [4.75, 4.77]. |
| 67 | `test_rmc_s48_mobile_frame_focus_touch_motion` | `--touch` ≥ 44 px and the `button` rule has `min-height: var(--touch)`; `input` rule `min-height` ≥ 44 px; `footer a` `min-height: 44px`; a `:focus-visible` rule with `outline:` width ≥ 2 px and `var(--focus)`; `outline: none`/`outline: 0` only inside `:focus:not(:focus-visible)`; `env(safe-area-inset-top)`, `-bottom`, `-left`, `-right` all present; one `@media (prefers-reduced-motion: reduce)` block; viewport meta keeps `viewport-fit=cover` and has no `user-scalable=no`/`maximum-scale`. |
| 68 | `test_rmc_s49_no_inline_style_script_or_data_uri` | `index.html`: no ` style=` attribute, no `<style`, no `<script` other than the single `<script src="app.js"></script>`, no `on[a-z]+=` attribute (any event, not only the three of test S8), no `javascript:`, `data:`, `blob:`; inline `<svg>`s have no `fill=`/`stroke=`/`href=`/`xlink:href`/`<use`/`<image`/`<foreignObject`/`<script`; `style.css`: no `url(`, no `@import`, no `@font-face`, no `expression(`; `icon.svg`: no `<script`, `on…=`, `href`, `style`, `<image`, `<foreignObject`. |
| 69 | `test_rmc_s50_ui_contract_ids_labels_and_attributes_are_kept` | All of §5: the 29 ids each exactly once on the listed tag, no other id; the `getElementById("…")` set of `app.js` equals the 29 ids; labels; `type="button"`; initial `hidden`/`disabled`/`role`/`aria-live`; `#state` initial class/data/text; `#unlock` classes contain `secondary`; `h2` texts; login/enroll detail sentences, the `#push-hint` paragraph and the `#enroll-code` attributes verbatim; `<title>soos remote</title>`; the no-switch rule of §5 (D11: no `switch`/`switch-knob` class, no `role="switch"`, no `aria-checked` in `index.html`; no `.switch` selector, no `:has(#push-disable` in `style.css`). Helper self-test on a small HTML string (tag of an id, label of a button, attribute presence). |
| 70 | `test_rmc_s51_header_band_carries_the_wordmark` | `index.html` has `<header class="band">` before `<main`; inside it one `h1` that contains an `<svg` with `viewBox="0 0 514 64"`, `aria-hidden="true"`, `focusable="false"` and exactly the six `WORDMARK_PATHS` `d` strings, followed by visually hidden text `soos remote`; `.wordmark` fill uses `var(--on-band)` and `.band` background `var(--band)`; `main` no longer contains an `h1`. |
| 71 | `test_rmc_s52_icons_are_the_brand_mark` | `icon.svg` has `viewBox="0 0 508 508"`, the pale `rect` and exactly the four `ICON_PATHS`; `index.html` stat tile and login tile SVGs contain exactly the two `STAR_SPIKE_PATHS` with `viewBox="0 0 508 508"` and `aria-hidden="true"`; `apple-touch-icon.png`: PNG magic, IHDR width = height = 180, bit depth 8, color type exactly 2 (RGB; round 2, F12), interlace 0, chunk list exactly `IHDR`, `IDAT`+, `IEND` (so no `tRNS`, `tIME`, text chunks), size ≤ 8 192 bytes; the icon link in `index.html` keeps `type="image/svg+xml"`. Helper self-test on a hand-built minimal chunk list (rejects color types 0 and 6 and a `tRNS` chunk). |
| 72 | `test_rmc_s53_manifest_and_meta_colors` | Manifest contains `"theme_color": "#0047BB"` and `"background_color": "#EDF1FF"`, keeps `"display": "standalone"`, `"start_url": "/"`, `"scope": "/"`, `"short_name": "soos"`, `"name": "soos remote"`, and exactly two icon entries (round 2, F9): `{"src": "icon.svg", "sizes": "any", "type": "image/svg+xml"}` and `{"src": "apple-touch-icon.png", "sizes": "180x180", "type": "image/png"}` (checked per entry: the three key/value pairs appear inside the same `{…}` object); `index.html` has `<meta name="theme-color" content="#0047BB">` (exactly one theme-color meta) and `apple-mobile-web-app-status-bar-style` `black-translucent`. |
| 73 | `test_rmc_s54_system_fonts_and_fixed_asset_set` | `style.css` declares `--font:` starting with `-apple-system` and every `font-family:` outside the tokens is `var(--font)` or `var(--font-mono)`; the asset directory listing equals the seven files of §7 (no `.woff`, `.woff2`, `.ttf`, `.otf`, `.png` other than the touch icon); `crates/remote/src/assets.rs` still lists the seven `include_bytes!` paths. |
| 74 | `test_rmc_s55_brand_is_documented` | `Docs/REMOTE_COMPANION.md` has a section starting `## 2e.` that mentions `#0047BB`, `#EDF1FF`, `#101820`, `#E59BDC`, `crates/gui/src/theme.rs`, `prefers-color-scheme`, `Unlock now`, `apple-touch-icon.png` and `rsvg-convert`; `AI/DECISIONS.md` contains the ADR title `soos Brand Direction Applied to the \`soos-remote\` Web App`; `AI/tester_contract_brand.md` exists. |
| 75 | `test_rmc_s56_scanner_handles_raw_byte_and_c_strings` (in `remote_alerts_contract.rs`) | `identifier_uses(blank_string_literals(…), "acknowledged")` is empty for `let a = br"acknowledged"; let b = br#"x"acknowledged"#; let c = cr"acknowledged"; let d = b"acknowledged"; let e = b'"';` and equals 1 for `let p = br"C:\"; let acknowledged = true; let q = "x";` and for `let p = cr"C:\"; acknowledged = 1; let q = "x";`; `let xbr = 1; let acknowledged = 2;` (an identifier ending in `br` followed by code, no quote) still yields 1. |

TDD red expectations: tests 63–68 and 70–74 fail on the current assets (old palette `#1f2937`, no band, lock icon); test 75
fails on the current helper (the `br"C:\"` case). Test 69 passes already on the current page by design (it is the
regression guard of the redesign; the current page has no switch either); the tester records this in `AI/tester_contract_brand.md`.

Existing tests stay green unchanged: `test_rmc_s8_*`, `test_rmc_s18_*`, `test_rmc_s30_*`, `test_rmc_s38_*`,
`test_rmc_s43_*`, `test_rmc_unlock_is_opt_in_and_documented`, `routes_tests`, `server_tests::test_rmc_assets_are_served_with_content_types`.

### 9.1 Visual verification (developer phase, scratchpad only)

- Stub server in `$SCRATCH/remote_brand_visual/` (Python stdlib run with `-I`, or Node): serves the files of
  `crates/remote/assets/` read-only by path argument (never copies edits back), sends the production CSP header
  verbatim from `http.rs`, and answers `/api/auth/state`, `/api/status`, `/api/events` (SSE), `/api/alerts`,
  `/api/push` with canned JSON for a scenario chosen by a command-line flag. `app.js` is never modified.
- Playwright (`~/.local/bin/playwright`, Chromium from `~/.cache/ms-playwright`): viewport 390 x 844,
  `deviceScaleFactor: 3`, `isMobile: true`, `colorScheme` `light` and `dark`; scenarios `locked`, `unlocked`,
  `funnel-login`, `alerts-3` (attention, coverage warn shown, 3 rows, Acknowledge), `push-enabled` (2 devices,
  notifications permission granted). Full-page screenshots to `$SCRATCH/remote_brand_shots/<scenario>-<scheme>.png`.
  The run fails on any console error or CSP violation report.
- Narrow widths (round 2, F11): `locked`, `unlocked` and an `unreachable` scenario (status endpoint fails) also at
  375 x 667 and 320 x 568, light only, to `$SCRATCH/remote_brand_shots/<scenario>-<width>.png`.
- The developer reads every screenshot and checks: band under the status bar area, wordmark legible, rounded top
  corners of the pale body visible against the blue band (D13), tile contrast, the state word on one line inside the
  tile at every width, dot color, buttons per D5, banners and glyph circles, push card without switch, no horizontal
  scroll (`document.documentElement.scrollWidth ≤ window.innerWidth` at every width).
- Screenshots, server and scripts never enter the repository.

## 10. Documentation

`Docs/REMOTE_COMPANION.md` new `## 2e. Page design (soos brand)`: the four brand colors with their Pantone names; the
statement that the page tokens mirror `crates/gui/src/theme.rs` (same names, same values) plus web-only dark tints;
light default and dark through `prefers-color-scheme`; the header wordmark, status tile, button semantics (`Lock now`
primary, `Unlock now` danger outline, still behind confirmation and Face ID); icons generated from `icon.svg` with the
§6.2 `rsvg-convert` command; the push card has no switch (`#push-state` is the push state, D11); the danger-outline
color deviation from the GUI and its AA reason (D12); CSP unchanged, system fonts only, no raster from the brand archive; pointer to the ADR.
Section 4 ("Web app icon") gets one sentence: an existing home-screen icon keeps the old image until it is removed and
added again (iOS caches it).

## 11. Verification Matrix Rows (traceability phase)

RMC76 tokens = GUI theme (63); RMC77 palette-only colors and recomputable dark tints (64); RMC78 light default + dark
variant (65); RMC79 WCAG AA contrast (66); RMC80 focus, touch targets, safe areas, reduced motion (67); RMC81 CSP-clean
markup and styles (68); RMC82 UI contract kept (69); RMC83 wordmark header (70); RMC84 brand icons, opaque
deterministic PNG (71); RMC85 manifest and meta colors (72); RMC86 system fonts, fixed asset set (73); RMC87
documentation and ADR (74) plus scanner fix (75); RMC88 owner check on the iPhone home-screen app, light and dark
(pending, hardware).

## 12. ADR Summary (authoritative text in `AI/DECISIONS.md`)

(1) The soos brand (blue `#0047BB`, Brilliant White `#EDF1FF`, ink `#101820`, pink `#E59BDC`, star mark, SOOS wordmark,
flat blocks, 20 px card radius, 2 px blue borders) is the brand of the whole project; `crates/gui/src/theme.rs` is the
single source of the token values and every other frontend mirrors them by name. (2) The web app applies it with CSS
custom properties named after the `theme.rs` constants, web-only dark tints computed from the palette, and role tokens;
light is the default and a dark variant follows `prefers-color-scheme`. (3) Presentation only: `app.js`, `sw.js`, Rust,
CSP, routes and every id and label are unchanged; `Unlock now` becomes a danger-outline button in `--danger-text`/`--danger-soft` (AA; the GUI outline uses `DANGER`);
the push card has no switch (a CSS-only switch could only mean "the PC has a device", not "this phone gets alerts"). (4) Icons come from the
owner's vectors; the touch icon is regenerated deterministically; no raster from the brand archive, and never the GUI
mockup, enters the repository. (5) System fonts only. (6) Tests 63–75, matrix RMC76–RMC88. Dark tints are integer-percentage mixes rounded half up in integer arithmetic.

## 13. Contract Migrations

| ID | Test / helper | Change | Justification |
|---|---|---|---|
| CM-1 | `tests/invariants/src/remote_alerts_contract.rs::blank_string_literals` (used by test 62) | Recognise `br`, `cr` (and `b`/`c` before `r#…`) as raw-string prefixes when the prefix starts at an identifier boundary | Removes a false-negative path; no assertion changes; strictly stronger; new self-test 75 |

No other migration is expected: no existing test pins the old colors, the `h1` placement, the `main.card` class, the
`#alerts-coverage` `detail` class, the meta values or the manifest colors (checked by `grep` over `tests/` and
`crates/remote/tests/`). If the tester finds one, it is recorded in `AI/tester_contract_brand.md` with this owner
request as the justification.

## 14. Invariants Touched

RMC-S8 (assets load nothing remote, no inline script) — strengthened by test 68; RMC-S18 (page ids) and the alert/push
page tests — kept by test 69; ADR 2026-10-05 item (9) (source link) — kept; CSP of `http.rs` §2.8 — unchanged;
AGENTS.md "NEVER log frames, biometric embeddings, or credentials" — not affected (static assets). No PAM, daemon,
IPC or latency path is touched (no latency budget section needed).

## 15. Open Points

1. Walkthrough numbering: `origin/main` already has `183_gui_brand_redesign.md` and `184_gui_frame_pacing_and_egui_ids.md`,
   while this branch had `183_remote_companion.md`–`187_remote_web_push.md`. The collision predated this spec; it was
   resolved on 2026-10-06 by renumbering the branch walkthroughs +2 (`185_remote_companion.md`–`189_remote_web_push.md`),
   and this change's walkthrough is `190_remote_brand_redesign.md` (planned here as `188_remote_brand.md`).
2. `Docs/GUI_APPLICATION.md` §5 (on `main`) should later point to the ADR as the project-wide brand rule; not edited
   here to avoid a conflict.
3. RMC88 needs the owner on the iPhone (home-screen icon must be re-added to refresh the cached icon).
4. The GUI `DangerOutline` (`DANGER` on `PALE`, 4.22:1) is below AA; the GUI should adopt `DANGER_TEXT` for the outline
   text in a later GUI change (D12). Not edited here (`crates/gui` is on `main` only).

## 16. Round 2 Changes (plan evaluation round 1)

| Finding | Resolution |
|---|---|
| F1 MAJOR tint values vs formula | §2.2b: integer rule stated; `--pale-weak` `#7F8590`, `--blue-soft` `#82A5E0`; brief §2.2/§2.3 corrected; contrast re-verified; test 64 integer-only with self-test |
| F2 MAJOR live parity parse | D4 and test 63: accepted forms (`from_rgb` hex/decimal, `Color32::WHITE`/`BLACK`, integer `u8`, `f32` `2.0`), exact-name match, unknown form or missing name fails; self-test on `THEME_EXCERPT_47AB53E` |
| F3 MAJOR push switch | Dropped (D11): plain `Notifications` heading; no-switch rule in §5 and test 69; brief §5.8 and wireframes updated; ADR item (3) |
| F4 color-property rule | §2.1 token-by-token rule (role or scale `var()`, never palette/tint) + test 64 self-test |
| F5 selector bindings | Test 66 pins selector-to-role bindings and forbids fill-only roles as text color |
| F6 invisible body corners | D13: `body` band-colored flex column, `main` grows, footer on `--page`; visual check |
| F7 test naming | Test 75 is `test_rmc_s56_…`; header states N → s(N − 19) |
| F8 GUI danger outline deviation | D12, §3, §10, ADR, open point 4 |
| F9 test 72 wording | Each manifest icon entry stated exactly |
| F10 banner glyph colors | `-icon` role for all four kinds; glyph in `--banner-<k>-bg`; ≥ 3:1 pairs in test 66 |
| F11 display width | `clamp()` display sizes, `overflow-wrap: anywhere`, 375/320 px visual checks |
| F12 PNG color type | Test 71 requires color type 2 exactly |


## 17. Round 3 Changes (design critique of the rendered page)

Developer-phase corrections after the Playwright critique; `style.css` only, no test changed, every gate green.

| Finding | Resolution |
|---|---|
| F1 BLOCKING star over the state word (390 px and below) | The tile star is 60 px at bottom 16 px right 18 px, and `.stat-tile` has `padding-bottom: 38px`. The detail lines under `#state` plus that padding are at least 86 px tall (the 6 px / 8 px margins collapse), more than the star and its offset (76 px), so the star stays at least 10 px under the word for any width and any number of detail lines. The `.state` column keeps its full width (reserving 92 px beside it would break `Unlocked` at 390 px). Re-rendered Unlocked/Unreachable at 320/375/390 px, light and dark, with a measured star-to-text gap check. |
| F2 bottom-heavy cards | `.feedback` keeps its reserved line (`min-height: 1.25em`, `line-height: 1.25`) with margins `-8px 0 -6px`; when it is the last visible element of a card (`.card > .feedback:last-child`, or followed only by a hidden last element such as `#logout`) it takes `margin-bottom: -16px`, so the empty line sits inside the card padding. No element removed. |
| F3 dark header pill | `.band-pill` uses `background: var(--on-band)` (pale in both themes) and `color: var(--text)`; the dark `@media` block carries one extra component rule, `.band-row .band-pill { color: var(--page); }` (ink in that block). Deviation from §2.1 item 6 ("same form"): the block still holds exactly one `:root` rule of roles; the extra rule only uses roles. Ink on pale: 15.87:1. |
| F4 unlocked dot reads brown | The unlocked dot is filled with `--banner-warn-border` (amber `#F5B547` light, `#F2BB60` dark) inside a `var(--stroke-card)` inset ring of `--dot-unlocked` (`--warn-text` light, `--warn` dark). The ring keeps the 3:1 boundary against the strip that test 66 pins; the amber fill reads as "warning" rather than brown. The role values of §2.3 are unchanged; the ring uses the inset-border `box-shadow` form of §2.1. |
| F5 push problems as a warn banner (optional) | **Not done.** It needs a class toggle in `app.js`, which D1 and auditor constraint C-5 freeze byte-identical. Left as a follow-up that needs a spec and auditor revision: `#push-state` would gain `banner banner-warn` when `view.sender === "unavailable"` or `view.last_delivery` is `rejected` / `failed`, text unchanged. |
