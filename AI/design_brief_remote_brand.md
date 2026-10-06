# Design Brief — soos Brand for the `soos-remote` Web App

- **Date**: 2026-10-06
- **Issue**: GitHub #339 (owner request 2026-10-06: "follow everything" of the soos brand
  direction: palette, logo, style; the brand becomes the brand of the whole project)
- **Branch**: `feat/remote-auth-alerts`
- **Scope**: `crates/remote/assets/{index.html, style.css, icon.svg, apple-touch-icon.png,
  manifest.webmanifest}`. `app.js` and `sw.js` keep their behavior (see section 11).
- **Status**: brand study only. This file changes no code. Revised in round 2 of the spec
  (plan evaluation F1, F3, F10): tint values and rounding rule, push switch removed, banner
  glyph colors. Where this brief and `AI/architect_spec_remote_brand.md` differ, the spec wins.

## 1. Sources Studied

| Source | What it fixes |
|---|---|
| `Colors_Direction.svg` (owner archive) | The four brand colors: `#0047BB` Pantone 2728 C, `#EDF1FF` Pantone 11-4001 TCX Brilliant White, `#101820` Pantone Black 6 C, `#E59BDC` Pantone 244 C |
| `Icon_logo.svg` (508 x 508) | The star mark: pale square, blue top-left and bottom-right squares, blue quarter shapes in the other two quadrants; the pale space left between them is a four-point star with concave sides |
| `text_logo.svg` (514 x 64) | The "SOOS" wordmark: six filled paths, pale fill, geometric letters with 16-unit strokes and a slanted bar in each S |
| `Artistic_charts_assets/Frame *.png` (18 lockups) | How the marks are combined: wordmark on a blue band; star on a blue square over a pale or ink field; pink star spikes on blue; wordmark crossing the star; ink wordmark on blue. No gradients, no shadows, flat color blocks only |
| `New_Gui_Interface.png` (1512 x 982 GUI mockup) | The UI style: blue top band with the wordmark; active tab as a pale "folder" joined to a pale body with 20-unit rounded top corners; rounded cards with a 2-unit blue border; an inner table on a slightly darker pale; a blue stat tile with a pale caption strip and a huge pale number; a blue tile carrying the pink star; a pale status pill with a label and a green switch with a blue knob. **Reference only: it shows the owner's face and must never be copied into the repository.** |
| `origin/main:crates/gui/src/theme.rs` | Exact token values, radii, stroke, font sizes (section 2 copies them) |
| `origin/main:crates/gui/src/brand.rs` | Wordmark and star path data, star spikes (`STAR_SPIKES`), procedural window icon |
| `origin/main:crates/gui/src/widgets.rs`, `header.rs` | Card frame, stat tile, star tile, banners (info, success, warn, danger), `BrandButton` (primary, secondary, danger, danger outline), toggle switch, header band and daemon pill |
| `origin/main:Docs/GUI_APPLICATION.md` §5, `AI/walkthroughs/183_gui_brand_redesign.md` | The GUI redesign rules: presentation only, no new font, every message and behavior unchanged |
| `Docs/REMOTE_COMPANION.md` §2a–2d, §4 and the `soos-remote` ADRs in `AI/DECISIONS.md` | Every UI state that must stay reachable (section 10) |

Brand character, in one line: flat blocks of saturated blue on a cool pale field, ink for
text, pink used sparingly as the one warm accent, generous round corners, no gradients and no
decorative shadows.

## 2. Tokens

### 2.1 Palette (identical to `theme.rs`)

CSS custom properties are the kebab-case names of the `theme.rs` constants, so phone and
desktop share one vocabulary. These values never change between light and dark: only the role
tokens of section 2.3 are remapped.

| CSS property | `theme.rs` | Hex | Use |
|---|---|---|---|
| `--blue` | `BLUE` | `#0047BB` | Header band, card borders (light), primary buttons, stat tile, focus ring (light) |
| `--blue-hover` | `BLUE_HOVER` | `#1D5CC3` | Hover of blue surfaces |
| `--blue-pressed` | `BLUE_PRESSED` | `#003A99` | Pressed (`:active`) blue surfaces |
| `--pale` | `PALE` | `#EDF1FF` | Page body, cards, caption strip, text and wordmark on blue |
| `--pale-2` | `PALE_2` | `#E4EAFD` | Inner tables (lists), info banner, inline code |
| `--pale-3` | `PALE_3` | `#DCE4FC` | Hover fill of secondary buttons |
| `--pale-4` | `PALE_4` | `#C9D6FA` | Pressed fill of secondary buttons |
| `--line` | `LINE` | `#C9D0E1` | Dividers, disabled button fill |
| `--ink` | `INK` | `#101820` | Primary text (light), page body (dark) |
| `--ink-muted` | `INK_MUTED` | `#5A6270` | Secondary text, placeholders |
| `--ink-weak` | `INK_WEAK` | `#7A818E` | Disabled text, input border, unknown-state dot |
| `--pink` | `PINK` | `#E59BDC` | Star mark only (never text on pale, never a state color) |
| `--white` | `WHITE` | `#FFFFFF` | Input background (light), text on danger buttons |
| `--success` | `SUCCESS` | `#29B53B` | Success banner border and icon |
| `--success-toggle` | `SUCCESS_TOGGLE` | `#46BE82` | Declared for parity; unused on the phone (no switch) |
| `--success-bg` | `SUCCESS_BG` | `#E6F6EA` | Success banner background |
| `--success-text` | `SUCCESS_TEXT` | `#146C2A` | Success banner text, "locked" dot (light) |
| `--danger` | `DANGER` | `#D92D45` | Danger button fill, danger banner border and icon |
| `--danger-hover` | `DANGER_HOVER` | `#B81F35` | Hover of danger buttons |
| `--danger-bg` | `DANGER_BG` | `#FDE8EB` | Danger banner background, hover of danger-outline buttons |
| `--danger-text` | `DANGER_TEXT` | `#8A1426` | Danger banner text, danger-outline text and border (light) |
| `--warn` | `WARN` | `#F59E0B` | Warn banner icon |
| `--warn-border` | `WARN_BORDER` | `#F5B547` | Warn banner border |
| `--warn-bg` | `WARN_BG` | `#FFF4E0` | Warn banner background |
| `--warn-text` | `WARN_TEXT` | `#8A4B00` | Warn banner text, "unlocked" dot (light) |

The GUI video overlay colors (`FACE_LIVE`, `EYE`, ...) and `SHADOW` have no use on the phone
and are not declared.

### 2.2 Dark tints (new, web only)

`theme.rs` has no dark theme (`theme::apply` installs the light visuals for both preferences).
The phone needs one (owner requirement), so it is built from the same four colors: every tint
is a linear sRGB mix of two palette colors, written as `mix(a, b, t)` with `t = n / 100`
(`n` an integer percentage). Each 8-bit channel is computed in integer arithmetic as
`(a * (100 - n) + b * n + 50) / 100` with truncating division, i.e. `a + (b - a) * t` rounded
half up (the same as half away from zero, since channels are non-negative); exact `.5` cases
(`--pale-weak` all three channels, `--blue-soft` green) therefore round up, with no
floating-point error.

| CSS property | Recipe | Hex | Use in dark |
|---|---|---|---|
| `--ink-2` | `mix(INK, PALE, 0.05)` | `#1B232B` | Card surface, caption strip |
| `--ink-3` | `mix(INK, PALE, 0.10)` | `#262E36` | Inner tables, inline code, secondary hover |
| `--ink-4` | `mix(INK, PALE, 0.16)` | `#333B44` | Disabled button fill, secondary pressed |
| `--ink-line` | `mix(INK, PALE, 0.22)` | `#414851` | Dividers |
| `--pale-muted` | `mix(PALE, INK, 0.30)` | `#ABB0BC` | Secondary text, placeholders |
| `--pale-weak` | `mix(PALE, INK, 0.50)` | `#7F8590` | Disabled text, input border, unknown dot |
| `--blue-edge` | `mix(BLUE, PALE, 0.30)` | `#477ACF` | Card border |
| `--blue-soft` | `mix(BLUE, PALE, 0.55)` | `#82A5E0` | Links, secondary button text and border, focus ring |
| `--danger-soft` | `mix(DANGER, PALE, 0.45)` | `#E28599` | Danger banner text and border, danger-outline |
| `--danger-bg-dark` | `mix(INK, DANGER, 0.18)` | `#341C27` | Danger banner background |
| `--success-soft` | `mix(SUCCESS, PALE, 0.45)` | `#81D093` | Success banner text and border |
| `--success-bg-dark` | `mix(INK, SUCCESS, 0.16)` | `#143124` | Success banner background |
| `--warn-soft` | `mix(WARN, PALE, 0.35)` | `#F2BB60` | Warn banner text and border |
| `--warn-bg-dark` | `mix(INK, WARN, 0.16)` | `#352D1D` | Warn banner background |

### 2.3 Role tokens and the two mappings

Components use role tokens only; the palette is never referenced directly in a component rule.
Light is the default (`:root`); dark overrides the roles under
`@media (prefers-color-scheme: dark)`.

| Role | Light | Dark | Ratio light | Ratio dark |
|---|---|---|---|---|
| `--page` (body) | `--pale` | `--ink` | — | — |
| `--band` (header) | `--blue` | `--blue` | — | — |
| `--on-band` (wordmark, pill text on band) | `--pale` | `--pale` | 7.11 on blue | 7.11 on blue |
| `--card` | `--pale` | `--ink-2` | — | — |
| `--card-border` | `--blue` | `--blue-edge` | 7.11 vs pale | 3.75 vs ink-2 |
| `--text` | `--ink` | `--pale` | 15.87 on pale | 15.87 on ink, 14.09 on ink-2 |
| `--text-muted` | `--ink-muted` | `--pale-muted` | 5.45 on pale, 5.12 on pale-2 | 8.24 on ink, 7.31 on ink-2, 6.33 on ink-3 |
| `--text-disabled` | `--ink-weak` | `--pale-weak` | 3.48 (exempt) | 4.28 on ink-2 |
| `--link` | `--blue` | `--blue-soft` | 7.11 on pale | 7.16 on ink, 6.36 on ink-2 |
| `--table` (lists) | `--pale-2` | `--ink-3` | ink 14.90 | pale 12.20 |
| `--divider` | `--line` | `--ink-line` | decorative | decorative |
| `--tile` (stat tile body) | `--blue` | `--blue` | — | — |
| `--on-tile` (big value) | `--pale` | `--pale` | 7.11 | 7.11 |
| `--strip` (tile caption strip) | `--pale` | `--ink-2` | ink 15.87 | pale 14.09 |
| `--on-strip` | `--ink` | `--pale` | 15.87 | 14.09 |
| `--primary` / `--on-primary` | `--blue` / `--pale` | `--blue` / `--pale` | 7.11 | 7.11 |
| `--primary-hover` | `--blue-hover` | `--blue-hover` | 5.53 | 5.53 |
| `--primary-pressed` | `--blue-pressed` | `--blue-pressed` | 9.02 | 9.02 |
| `--secondary-fg` (text and 2 px border) | `--blue` | `--blue-soft` | 7.11 on pale, 6.32 on pale-3, 5.53 on pale-4 | 6.36 on ink-2, 5.51 on ink-3, 4.55 on ink-4 |
| `--secondary-hover` / `--secondary-pressed` | `--pale-3` / `--pale-4` | `--ink-3` / `--ink-4` | — | — |
| `--danger-fill` / `--on-danger` | `--danger` / `--white` | `--danger` / `--white` | 4.76 (hover 6.39) | 4.76 |
| `--danger-fg` (danger outline) | `--danger-text` | `--danger-soft` | 8.43 on pale | 6.09 on ink-2 |
| `--danger-outline-hover` | `--danger-bg` | `--danger-bg-dark` | 8.11 | 6.00 |
| `--disabled-fill` / `--disabled-fg` | `--line` / `--ink-weak` | `--ink-4` / `--pale-weak` | 2.54 (exempt) | 3.06 (exempt) |
| `--input-bg` | `--white` | `--ink` | ink 17.89 | pale 15.87 |
| `--input-border` | `--ink-weak` | `--pale-weak` | 3.92 vs white | 4.82 vs ink |
| `--placeholder` | `--ink-muted` | `--pale-muted` | 6.15 on white | 8.24 on ink |
| `--focus` (ring) | `--blue` | `--blue-soft` | 7.11 vs pale | 6.36 vs ink-2, 7.16 vs ink |
| `--focus-on-blue` (ring over a blue fill) | `--pink` | `--pink` | 3.84 vs blue | 3.84 vs blue |
| `--info-bg` / `--info-border` / `--info-fg` / `--info-icon` | `--pale-2` / `--line` / `--ink` / `--blue` | `--ink-3` / `--ink-line` / `--pale` / `--blue-soft` | ink 14.90 | pale 12.20, icon 5.51 |
| `--success-bg-r` / `--success-border` / `--success-fg` | `--success-bg` / `--success` / `--success-text` | `--success-bg-dark` / `--success-soft` / `--success-soft` | 5.84 | 7.61 |
| `--warn-bg-r` / `--warn-border-r` / `--warn-fg` | `--warn-bg` / `--warn-border` / `--warn-text` | `--warn-bg-dark` / `--warn-soft` / `--warn-soft` | 6.24 | 7.80 |
| `--danger-bg-r` / `--danger-border` / `--danger-fg-r` | `--danger-bg` / `--danger` / `--danger-text` | `--danger-bg-dark` / `--danger-soft` / `--danger-soft` | 8.11 | 6.00 |
| `--dot-locked` | `--success-text` | `--success` | 5.81 vs pale strip | 5.88 vs ink-2 strip |
| `--dot-unlocked` | `--warn-text` | `--warn` | 6.03 | 7.40 |
| `--dot-alarm` (unavailable, unreachable) | `--danger-text` | `--danger-soft` | 8.43 | 6.09 |
| `--dot-idle` (unknown, no_session) | `--ink-weak` | `--pale-weak` | 3.48 | 4.28 |
| `--star` | `--pink` | `--pink` | 3.84 vs blue (decorative) | 3.84 |

Ratios are WCAG 2.x relative-luminance contrast ratios, computed with the sRGB transfer
function (script kept in the scratchpad, not in the repository). Every text pair is at least
4.5:1 except disabled text, which WCAG exempts. Every non-text boundary that identifies a
control (input border, secondary and danger-outline border, focus ring) is at least 3:1.

Pairs that fail and are therefore **forbidden**:

| Pair | Ratio | Rule |
|---|---|---|
| `--danger` text on `--pale` | 4.22 | Danger-colored text on light surfaces uses `--danger-text` |
| `--success` / `--warn` text or dots on `--pale` | 2.40 / 1.90 | Light state dots use the `*-text` tone |
| `--pale` text on `--pink` | 1.85 | Pink never carries text |
| `--blue` text or border on `--ink` / `--ink-2` | 2.23 / 1.98 | In dark, blue is a fill only; outlines and text use `--blue-edge` / `--blue-soft` |
| `--danger` text on `--ink-2` | 3.34 | Dark danger text uses `--danger-soft` |

Accepted below 3:1, all decorative and never the only cue: `--card-border` is not a control
boundary; the blue primary button fill against the dark page (2.23) is identified by its 7.11
label. (Round 2: the push switch of the first draft is removed, see section 5.8.)

## 3. Typography

System font stack only (no font file, no licence question, nothing downloaded):

```css
--font: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto,
        "Helvetica Neue", Arial, sans-serif;
--font-mono: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
```

SF Pro has real weights, so the GUI's faux-bold trick is not needed: weights 600 and 800 do the
same job. Sizes follow the GUI scale moved up to phone reading sizes (the GUI body is 13 pt on a
1512-point window; iOS body text is 17 pt and an input below 16 px makes Safari zoom).

| CSS property | Size / line height | Weight | GUI equivalent | Use |
|---|---|---|---|---|
| `--f-display` | 64 px / 1.0, letter-spacing -0.02em | 800 | stat tile value (40 pt and up) | `#state` for `Locked`, `Unlocked` |
| `--f-display-long` | 44 px / 1.05, letter-spacing -0.01em | 800 | — | `#state` for `Connecting`, `No session`, `Unavailable`, `Unreachable` (fits 322 px) |
| `--f-title` | 20 px / 1.25 | 700 | `F_CARD_TITLE` 19 | Card titles (`h2`), login heading |
| `--f-section` | 17 px / 1.3 | 600 | `F_SECTION` 15 | Alerts summary line, banner lead text |
| `--f-body` | 16 px / 1.45 | 400 | `F_BODY` 13 | Detail text, inputs, list rows |
| `--f-button` | 17 px / 1.0 | 600 | button 14 | Every button |
| `--f-small` | 13 px / 1.35 | 400 | `F_SMALL` 12 | `#updated`, footer, hints |
| `--f-caption` | 12 px / 1.0, uppercase, letter-spacing 0.08em | 700 | stat strip caption (12 pt, +1 spacing) | Tile caption, header pill |
| `--f-code` | 18 px mono, letter-spacing 0.12em, uppercase | 600 | — | `#enroll-code` |

Numbers in lists and `#updated` use `font-variant-numeric: tabular-nums`.

## 4. Radii, Borders, Spacing, Shadows, Motion

| CSS property | Value | GUI source | Use |
|---|---|---|---|
| `--r-body` | 20 px | `BODY_TOP_RADIUS` | Top corners of the pale body under the band |
| `--r-card` | 20 px | `R_CARD` | Cards, stat tile, login star tile |
| `--r-table` | 14 px | `R_TABLE` | Inner tables (lists) |
| `--r-banner` | 12 px | banner frame | Banners |
| `--r-input` | 10 px | `R_INPUT` | Enrollment code input |
| `--r-pill` | 999 px | `R_BUTTON` 18 on a 36 pt button = full pill | Buttons, header pill |
| `--stroke-card` | 2 px | `STROKE_CARD` | Card border, secondary border, input border, tile strip inset |
| `--stroke-thin` | 1 px | banner stroke | Banner border, dividers |
| `--gap-card` | 14 px | `GAP_CARD` | Between cards |
| `--gutter` | `max(16px, env(safe-area-inset-left/right))` | — | Page side padding |
| `--pad-card` | 20 px | — | Card inner padding |
| `--gap-item` | 8 px | `item_spacing` | Between text lines; 12 px between stacked buttons |
| `--band-h` | 56 px + `env(safe-area-inset-top)` | `HEADER_H` 44 | Header band (taller for a touch screen) |
| `--strip-h` | 44 px | `STRIP_H` 50 | Tile caption strip |
| `--touch` | 48 px | `interact_size` | Minimum height of every button (above the 44 px target) |

**Shadows**: none. The brand frames are flat and the GUI uses shadows only for floating
windows and popups, which the page does not have. Cards are separated by their blue border and
the 14 px gap. (Should a popup ever appear: `0 8px 24px rgb(16 24 32 / 16%)`, the GUI
`SHADOW`.)

**Motion**: 120 ms `background-color`, `border-color` and `transform` transitions on buttons; none under `prefers-reduced-motion: reduce`.

## 5. Component Specs

### 5.1 Page frame and header band

- `html` background: `linear-gradient(var(--band) 50%, var(--page) 50%)` so the iOS rubber-band
  overscroll shows blue above the band and the page color below the body (a CSS gradient, not
  a `data:` URI or an image).
- `<header class="band">`: background `--band`, padding-top `env(safe-area-inset-top)`,
  height `--band-h`, side padding `--gutter`, flex row, space-between, center.
  - Left: the `h1`. It contains the inline SVG wordmark (`text_logo.svg` paths, `viewBox="0 0
    514 64"`, height 18 px, width 145 px, `fill` set from CSS through a class to `--on-band`,
    `aria-hidden="true"`, `focusable="false"`) followed by a visually hidden span "soos remote",
    so the accessible heading stays "soos remote".
  - Right: the header pill, the phone version of the mockup's "Daemon Active" pill:
    `--on-band` background (pale in both themes) with ink text (`--text` in light, `--page`
    in the dark block; round 3, F3), `--f-caption`, padding 6 px 12 px, `--r-pill`, text
    `Remote`. Static, not interactive (no switch: the page has no daemon control).
- `body`: background `--band` (round 2, F6), so the rounded top corners of `<main>` are cut
  out of blue exactly like the GUI body under its band; `body` is a flex column of at least
  `100dvh` and `<main>` grows (`flex: 1`), the footer has the `--page` background, so no blue
  shows below the content.
- `<main>`: background `--page`, `border-radius: var(--r-body) var(--r-body) 0 0`, margin-top
  -1 px under the band (the GUI paints the body one point into the band), padding 20 px
  `--gutter` `calc(24px + env(safe-area-inset-bottom))`, flex column, gap `--gap-card`,
  `max-width: 480px` centered for wide screens (the band stays full width).
- `apple-mobile-web-app-status-bar-style` becomes `black-translucent` so the blue band runs
  under the iPhone status bar (white status text on blue); `theme-color` becomes `#0047BB` for
  both schemes (the band is blue in both).

### 5.2 Card

`--card` fill, `--stroke-card` solid `--card-border` drawn inside (`border` with
`box-sizing: border-box`), `--r-card`, padding `--pad-card`, text left aligned. Title `h2` in
`--f-title`, `--text`, margin 0 0 12 px. Detail paragraphs `--f-body`, `--text-muted`.

### 5.3 Stat tile (lock status)

The mockup's stat tile, full width, height 196 px minimum:

```
+--------------------------------------------+  radius 20, fill --tile
|| PC STATUS                            (o) ||  strip: 44 px, fill --strip, inset 2 px,
|+------------------------------------------+|  top radius 18, caption --f-caption --on-strip,
|                                            |  dot 10 px at right: --dot-<state>
|  Locked                                    |  #state: --f-display, --on-tile, left aligned
|  Active session, idle for 4 min            |  #activity: --f-body, --on-tile at 90 % opacity
|  Updated 2 s ago                       *   |  #updated: --f-small, --on-tile at 80 %
+--------------------------------------------+  * star: 60 px pink star, bottom right
```

- The tile wraps the existing `#state`, `#activity` and `#updated` (ids unchanged). `app.js`
  already writes `class="state state-<state>"` and `data-state`, so all styling hangs on those:
  `.state-locked`, `.state-unlocked`, `.state-no_session`, `.state-unavailable`,
  `.state-unreachable`, `.state-unknown`.
- The dot color is chosen in CSS with `.stat-tile:has(.state-locked)` (and so on); the dot is an
  aria-hidden `span`. The state word itself is the information; the dot only repeats it.
  Round 3 (F4): the unlocked dot is an amber fill (`--banner-warn-border`) inside a 2 px ring
  of `--dot-unlocked`, so it reads amber, not brown, and keeps its 3:1 boundary on the strip.
- Round 3 (F1): the star sits below the state word, never beside it: the detail lines plus the
  38 px bottom padding are taller than the star and its offset, so even `Unlocked` at 320 px
  keeps at least 10 px between the word and the star tip.
- Long labels (`Connecting`, `No session`, `Unavailable`, `Unreachable`) use
  `--f-display-long` so the longest one fits the 322 px text column at 390 px width.
- `Unreachable` and `Connecting` also lower the value to 70 % opacity to read as "not live"
  (4.27:1 on the blue; the value is 44 px bold, large text, so 3:1 is the bar).
- The pink star is the `STAR_SPIKES` geometry of `brand.rs` as an inline SVG
  (`aria-hidden="true"`), fill `--star`, full opacity.
- `#activity` / `#updated` on blue at 90 % / 80 % opacity measure 6.05:1 / 5.11:1 (pale
  blended over blue: `#D5E0F8` / `#BECFF1`).

### 5.4 Buttons

All buttons: full width, min-height `--touch` (48 px), padding 12 px 20 px, `--r-pill`,
`--f-button`, centered label, `cursor: pointer`, `touch-action: manipulation`, 12 px apart.

| Variant | Class | Fill | Text | Border | Hover / pressed |
|---|---|---|---|---|---|
| Primary | (default `button`) | `--primary` | `--on-primary` | none | `--primary-hover` / `--primary-pressed` + `scale(0.98)` |
| Secondary | `.secondary` (existing) | transparent | `--secondary-fg` | 2 px `--secondary-fg` inside | `--secondary-hover` / `--secondary-pressed` |
| Danger | `.danger` | `--danger-fill` | `--on-danger` | none | `--danger-hover` |
| Danger outline | `.secondary.danger` | transparent | `--danger-fg` | 2 px `--danger-fg` inside | `--danger-outline-hover` |
| Disabled (any) | `:disabled` | `--disabled-fill` (outline variants: transparent with 2 px `--divider`) | `--disabled-fg` | — | none, `cursor: default`, no opacity trick |

**Decision for Lock / Unlock** (semantics kept, styling decided here):

- `#lock` "Lock now": **primary**. Locking is the safe, frequent action and the reason the page
  exists.
- `#unlock` "Unlock now": **danger outline** (`class="secondary danger"`). Unlocking lowers the
  PC's protection, already needs the `confirm()` tap and Face ID, and only one of the two
  buttons is ever enabled (lock while unlocked, unlock while locked), so the enabled one is
  always the only colored button. A filled danger button would shout on every locked-state
  visit; the outline marks the risk without inviting the tap.
- `#logout` "Sign out", `#alerts-ack` "Acknowledge", `#push-test`, `#push-disable`:
  secondary.
- `#login-button`, `#enroll-button`, `#push-enable`: primary.

### 5.5 Pills

- Header pill: section 5.1.
- State chip inside banners and list rows is **not** added: every state is already a sentence
  written by `app.js`.

### 5.6 Banners

The GUI `banner`: `--r-banner`, 1 px border, padding 12 px 14 px, a 20 px round icon at left
(CSS `::before` with `content: "i"`, `"!"` or `"\2713"`, `--f-small` 700; no image and no `data:`
URI), text `--f-section` for the lead line. Round 2 (F10): the circle is filled with
`--banner-<kind>-icon` and the glyph character is drawn in `--banner-<kind>-bg`, so the glyph
is a cut-out of the banner background; every kind keeps the glyph at least 3:1 on its circle
(light / dark: info 6.68 / 5.51, success 5.84 / 7.61, warn 6.24 / 7.80, danger 8.11 / 6.00).
The warn circle is `--warn-text` (light) / `--warn-soft` (dark), not `--warn` (1.97 with its
glyph).

| Kind | Background | Border | Icon | Text | Where |
|---|---|---|---|---|---|
| Info | `--info-bg` | `--info-border` | `--info-icon` "i" | `--info-fg` | `#alerts-summary` without attention (no attempts, unavailable, starting) |
| Success | `--success-bg-r` | `--success-border` | `--success-fg` check | `--success-fg` | reserved (no current text is reliably a success; see note) |
| Warn | `--warn-bg-r` | `--warn-border-r` | `--warn-fg` "!" | `--warn-fg` | `#alerts-coverage` ("Lock screen not monitored — see setup") |
| Danger | `--danger-bg-r` | `--danger-border` | `--danger-fg-r` "!" | `--danger-fg-r` | `#alerts-summary` when `#alerts` has `.alerts-attention` |

Note: "No failed password attempts" could look like a success, but the same paragraph also
carries "Password alerts unavailable" and "starting", and only `.alerts-attention` is a class
`app.js` sets. Using info for every non-attention text keeps the color honest without a
JavaScript change.

### 5.7 Lists (inner table)

`#alerts-history`, `#push-devices`: `--table` fill, `--r-table`, padding 4 px 14 px, no
bullets. Rows: `--f-body`, `--text`, min-height 44 px, padding 10 px 0, 1 px `--divider`
between rows (none after the last), tabular numbers. An empty list collapses (`:empty`
→ `display: none`) so no empty pale box shows.

### 5.8 Switch (removed in round 2)

The first draft put the mockup's green switch next to the Notifications title, on when
`#push-disable` is visible. That condition means "the PC has at least one registered device",
not "this phone receives alerts" (it stays on after this phone's subscription was dropped), so
in a failed-password alerting feature a green switch would state a delivery guarantee the page
does not have. The switch is dropped: the card keeps its plain `Notifications` heading and
`#push-state` stays the only statement of the push state. The GUI daemon pill switch has no
phone equivalent either (the header pill is static text).

### 5.9 Input

`#enroll-code`: full width, min-height 52 px, `--input-bg`, 2 px `--input-border`, `--r-input`,
`--f-code` centered, `--text`, placeholder `--placeholder`. Focus: border `--focus` plus the
focus ring.

### 5.10 Focus, touch and other states

- `:focus-visible` on buttons, the input and links: `outline: 3px solid var(--focus);
  outline-offset: 3px` (the offset lands on the card or page, never on blue). A focusable
  element over a blue fill (none today) uses `--focus-on-blue`.
- `:focus:not(:focus-visible)`: no outline (taps on iPhone do not draw a ring).
- `-webkit-tap-highlight-color: transparent`, replaced by the pressed styles.
- `[hidden] { display: none !important; }` stays (the screen logic depends on it).
- Feedback lines (`role="status"` paragraphs): `--f-small`, `--text-muted`, min-height 1.4em
  so the layout does not jump.

### 5.11 Login tile

The Funnel login card shows a 96 px star tile (the GUI `star_tile`: blue rounded square,
`--r-card`, pink spikes) above the title. Purely decorative, inline SVG, aria-hidden.

### 5.12 Footer

`source` link, `--f-small`, `--link`, underlined, centered, padding 12 px so the target is
44 px tall, bottom padding `env(safe-area-inset-bottom)`.

## 6. Icon Plan

| File | Content | How |
|---|---|---|
| `icon.svg` | The owner's `Icon_logo.svg` vector as provided (`viewBox="0 0 508 508"`, pale square, two blue squares, two blue quarter shapes) | Copied path for path; no script, no external reference; also the favicon (`<link rel="icon" href="icon.svg" type="image/svg+xml">`, already present) |
| `apple-touch-icon.png` | 180 x 180, RGB without alpha, rendered from `icon.svg` | `rsvg-convert -w 180 -h 180 icon.svg \| magick png:- -background '#EDF1FF' -alpha remove -alpha off -strip -define png:color-type=2 -define png:exclude-chunks=date,time apple-touch-icon.png`; deterministic bytes (no time chunks), since the CI fingerprints binary assets |
| Inline wordmark (index.html) | `text_logo.svg` paths, class-driven fill | Header band |
| Inline star (index.html) | `brand.rs` `STAR_SPIKES` on a transparent box (`viewBox="0 0 508 508"`, two paths: `M254 0C254 140.28 367.72 254 508 254L254 254Z` and `M0 254C140.28 254 254 367.72 254 508V254H0Z`), class-driven `--star` fill | Stat tile corner, login tile |

No raster from the archive enters the repository; the PNG is generated from the vector.
iOS rounds the icon corners itself; the mark is full bleed exactly as `Icon_logo.svg`.

Manifest: `"theme_color": "#0047BB"`, `"background_color": "#EDF1FF"` (splash color: the
manifest cannot vary with the scheme, and pale is the brand default). Name, short name,
`display`, `start_url`, `scope` and both icon entries stay.

## 7. CSP and Security Fit

The response CSP is `default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self';
...`. The design fits it without a change:

- Inline SVG is markup, not a script or a style; colors come from classes in `style.css`
  (`fill: var(--...)`), so no `style=""` attribute is needed anywhere.
- No `data:` URI, no `url()` outside the same origin, no `@import`, no web font.
- Icons inside banners are CSS generated content (text glyphs), not images.
- `:has()` selectors (Safari 15.4 and later; Web Push already needs iOS 16.4) replace any
  JavaScript class toggling, so `app.js` does not change.

## 8. Dark Mode Summary

Same layout, same blue band, same blue stat tile and pink star. The body turns ink, cards
`--ink-2` with a `--blue-edge` border, lists `--ink-3`, text pale, links and secondary controls
`--blue-soft`, banners on ink-tinted backgrounds with soft state text. The primary button stays
brand blue with pale text (7.11:1). The tile caption strip turns `--ink-2` so the tile does not
show a bright stripe on a dark page.

## 9. Wireframes (390 x 844, iPhone 14; light shown, dark identical in layout)

Legend: `###` blue band or blue fill, `[ Primary ]` filled button, `( Secondary )` outline
button, `{ Danger outline }` danger outline, `[~ disabled ~]` disabled, `*` pink star,
`(o)` state dot.

### 9.1 Locked (tailnet, alerts quiet, push active)

```
#############################################  status bar (blue, black-translucent)
## SOOS                           [ REMOTE ] ##  band 56 px
#############################################
 .-----------------------------------------.   body: pale, top radius 20
 | ######################################## |
 | #| PC STATUS                      (o) |# |   dot: --dot-locked
 | #+-------------------------------------+# |
 | #  Locked                               # |   64 px / 800
 | #  Active session, idle for 4 min       # |
 | #  Updated 2 s ago                   *  # |
 | ######################################## |
 | +--------------------------------------+ |   card (2 px blue)
 | | [~~~~~~~~~~~ Lock now ~~~~~~~~~~~~~] | |   disabled while locked
 | | {           Unlock now             } | |   danger outline, enabled
 | |  Locked                              | |   #feedback
 | +--------------------------------------+ |
 | +--------------------------------------+ |
 | | Failed passwords on the PC           | |
 | | (i) No failed password attempts      | |   info banner
 | +--------------------------------------+ |
 | +--------------------------------------+ |
 | | Notifications                        | |
 | |  1 device receives notifications     | |
 | |  .--------------------------------.  | |
 | |  | Apple device, enabled 06/10 ... |  | |   inner table
 | |  '--------------------------------'  | |
 | |  Not yours? Run soos-remote push ... | |
 | | [     Enable notifications       ]   | |
 | | (     Send test notification     )   | |
 | | (     Disable notifications      )   | |
 | +--------------------------------------+ |
 |                 source                   |
 '------------------------------------------'
```

### 9.2 Unlocked

```
## SOOS                           [ REMOTE ] ##
 | #| PC STATUS                      (o) |# |   dot: --dot-unlocked
 | #  Unlocked                             # |
 | #  Active session                       # |
 | #  Updated 1 s ago                   *  # |
 | +--------------------------------------+ |
 | | [            Lock now              ] | |   primary, enabled
 | | {~~~~~~~~~~ Unlock now ~~~~~~~~~~~~} | |   disabled outline
 | |                                      | |
 | +--------------------------------------+ |
 | (alerts card, push card, footer as 9.1)  |
```

### 9.3 Funnel login screen

```
#############################################
## SOOS                           [ REMOTE ] ##
#############################################
 .-----------------------------------------.
 | +--------------------------------------+ |
 | |              #########               | |
 | |              ##  *  ##               | |   96 px star tile
 | |              #########               | |
 | |  Sign in                             | |   card title (visible label of #login)
 | |  Sign in with your passkey to reach  | |
 | |  this PC over the internet.          | |
 | | [      Sign in with Face ID       ]  | |   primary
 | |  Your session ended, please sign ... | |   #login-feedback
 | +--------------------------------------+ |
 |                 source                   |
 '------------------------------------------'
 (#status-card, #alerts, #push, #enroll hidden; #logout hidden)
```

The "Sign in" title is a new static `h2` inside `#login` (no id; no test or doc relies on
the section having no heading).

### 9.4 Alerts with 3 entries (attention)

```
 | +--------------------------------------+ |   card border stays blue
 | | Failed passwords on the PC           | |
 | | .----------------------------------. | |
 | | |(!) 3 failed password attempts,   | | |   danger banner (#alerts-summary)
 | | |    last at 14:02 (sudo)          | | |
 | | '----------------------------------' | |
 | | .----------------------------------. | |
 | | |(!) Lock screen not monitored —   | | |   warn banner (#alerts-coverage), when shown
 | | |    see setup                     | | |
 | | '----------------------------------' | |
 | |  .--------------------------------.  | |
 | |  | 14:02 sudo, your account, 1    |  | |   inner table, 3 rows
 | |  |   wrong password               |  | |
 | |  |--------------------------------|  | |
 | |  | 13:58 lock screen, your        |  | |
 | |  |   account, 1 wrong password    |  | |
 | |  |--------------------------------|  | |
 | |  | 13:41 login, root, 1 wrong     |  | |
 | |  |   password                     |  | |
 | |  '--------------------------------'  | |
 | | (           Acknowledge            ) | |   secondary
 | |  Acknowledged                        | |   #alerts-feedback
 | +--------------------------------------+ |
```

### 9.5 Push card enabled (2 devices, sender warning)

```
 | +--------------------------------------+ |
 | | Notifications                        | |   plain heading, no switch
 | |  2 devices receive notifications.    | |
 | |  Push sender not running on the PC   | |
 | |  .--------------------------------.  | |
 | |  | Apple device, enabled 06/10 ...|  | |
 | |  |--------------------------------|  | |
 | |  | Apple device, enabled 05/10 ...|  | |
 | |  '--------------------------------'  | |
 | |  Not yours? Run soos-remote push     | |   --f-small, code in --table chips
 | |  list and push remove N on the PC... | |
 | | [     Enable notifications       ]   | |
 | | (     Send test notification     )   | |
 | | (     Disable notifications      )   | |
 | |  Test notification sent              | |
 | +--------------------------------------+ |
```

With no subscribed device: list and hint hidden, only "Enable notifications".
Without push support (Safari tab, not the home-screen app): the state sentence only, no button.

### 9.6 Other states (same frame)

- **Connecting** (first paint): tile value `Connecting` at `--f-display-long`, 70 % opacity,
  `--dot-idle`; both buttons disabled.
- **Unreachable**: `Unreachable` at `--f-display-long`, 70 %, `--dot-alarm`; activity line "The
  PC is off, asleep, or unreachable".
- **No session / Unavailable**: `--f-display-long`, full opacity, `--dot-idle` /
  `--dot-alarm`.
- **Enrollment (tailnet with a pending code)**: an extra card "Add a passkey" after the push
  card: detail with inline code, the code input, primary "Add this device's passkey",
  feedback.
- **Funnel signed in**: `( Sign out )` secondary at the bottom of the status card.

## 10. UI Contract To Preserve

Every id of the current `index.html` stays, on the same element type, with the same text:
`login`, `login-button`, `login-feedback`, `status-card`, `state`, `activity`, `updated`, `lock`,
`unlock`, `feedback`, `logout`, `alerts`, `alerts-summary`, `alerts-coverage`,
`alerts-history`, `alerts-ack`, `alerts-feedback`, `push`, `push-state`, `push-devices`,
`push-hint`, `push-enable`, `push-test`, `push-disable`, `push-feedback`, `enroll`,
`enroll-code`, `enroll-button`, `enroll-feedback`. The `hidden` attributes, the
`aria-live="polite"` / `role="status"` attributes, the `<script src="app.js">`, the
`apple-mobile-web-app-capable` meta, the manifest, touch icon and stylesheet links and the plain
`<a href="https://github.com/Mysticaly622/soos">source</a>` stay.

Invariant needles the redesign must keep (from `tests/invariants/src/remote_*_contract.rs`):
`prefers-color-scheme` in `style.css`; no `url()` to an absolute URL and no `@import` in
`style.css`; no `.acknowledged` selector; no inline `<script>` body and no `on*=` handlers in
`index.html`; `icon.svg` contains `<svg` and no `<script`; `apple-touch-icon.png` is a PNG with
a 180 x 180 IHDR; manifest keeps `"display": "standalone"`, `"start_url": "/"`, `"scope": "/"`,
`icon.svg` and no absolute URL; the `app.js` constants and labels are not touched.

Changes this brief introduces that no test pins (no Contract Migration expected): the
`.danger` class on `#unlock`, wrapper elements (`header.band`, `.stat-tile`, `.actions`), the
inline wordmark and star SVGs, the static "Sign in" heading and "Remote" pill, the
`status-bar-style` and `theme-color` values, the manifest colors. If a test turns out to pin
one of these, the migration is recorded in `AI/tester_contract_brand.md`.

## 11. Out of Scope

- `app.js` and `sw.js` behavior (no new class toggling, no new text, no notification icon).
  The round 3 critique suggestion F5 (push sender or delivery problems as a warn banner) needs
  a class toggle in `app.js` and is left as a follow-up.
- Any Rust change beyond `assets.rs` (none is needed: no new asset file is introduced).
- A font file, a CDN, a raster from the owner archive, `New_Gui_Interface.png` in any form.
- A light/dark manual toggle (the page follows `prefers-color-scheme`, no storage).
