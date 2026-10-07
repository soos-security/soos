# Auditor Constraints — GitHub #339 follow-up: soos Brand Direction Applied to the `soos-remote` Web App

- **Date**: 2026-10-06
- **Branch**: `feat/remote-auth-alerts` (base `ed52b02`; nothing committed, pushed, stashed, deployed or restarted)
- **Owner request (2026-10-06, binding)**: completely redesign the web app to follow the soos brand direction
  ("follow everything": palette, logo, style); the brand becomes the brand of the whole project.
- **Audited**: `AI/architect_spec_remote_brand.md` (round 2), `AI/design_brief_remote_brand.md`, the drafted ADR
  "[2026-10-06] soos Brand Direction Applied to the `soos-remote` Web App" (`AI/DECISIONS.md`),
  `AI/plan_evaluator_report.md` (round 2, `APPROVED` with MINOR N1–N4), `AI/tester_contract_brand.md` and its tests
  (`tests/invariants/src/remote_brand_contract.rs`, test 75 and CM-1 in `tests/invariants/src/remote_alerts_contract.rs`,
  the module registration in `tests/invariants/src/lib.rs`), and the files about to change:
  `crates/remote/assets/{index.html, style.css, icon.svg, apple-touch-icon.png, manifest.webmanifest}`, plus the files
  that must not change: `crates/remote/assets/{app.js, sw.js}`, `crates/remote/src/{assets.rs, http.rs}`.
- **Brand sources inspected (read-only, scratchpad)**: `Icon_logo.svg` (397 bytes) and `text_logo.svg` (1 472 bytes)
  contain only `<svg>`, `<rect>` and `<path>` elements with `fill` attributes: no `<script>`, `<style>`, `href`,
  `<image>`, `<foreignObject>` or event attribute (grep count 0). Their path data may be copied. No raster of the
  archive (`New_Gui_Interface.png`, `Artistic_charts_assets/*.png`) is referenced by any file in the tree
  (`git grep New_Gui_Interface` hits only the plan evaluator report, as a prohibition).

## Audit checklist results

| # | Check | Command / evidence | Result |
|---|---|---|---|
| 1 | Panic paths | `grep -rnE '\.unwrap\(\|\.expect\(\|panic!\|todo!\|unimplemented!\|unreachable!' crates/remote/src` | 0 hits. No production Rust file changes (spec D1). Test modules carry the usual scoped `#![allow(...)]` with a `reason`. |
| 2 | Raw-memory code | No Rust production change; the invariants crate change is test-only | Nothing to audit. The word is absent from every new text (grep over the spec, brief, tester contract, new test module and ADR diff: 0 hits). |
| 3 | Output isolation | No PAM or binary code touched | n/a |
| 4 | Bounded I/O and deadlines | No server, socket or loop change; `Cache-Control: no-store` and the CSP in `http.rs::MANDATORY_HEADERS` stay byte-identical | n/a. `sw.js` has no fetch handler and no cache, so a redeployed page is served fresh (no stale-asset risk). |
| 5 | Arithmetic | Test helpers only (`mix` in `u32`, contrast in `f64`) | Tests are allowed to panic; the integer `mix` cannot overflow (`255 * 100 + 255 * 100 + 50 < u32::MAX`). |
| 6 | Filesystem | No runtime file access added; assets are `include_bytes!` | n/a |
| 7 | Secrets and privacy | Static assets only; no data path changed. Risk is the owner's face or private identifiers entering the repository through the redesign or its screenshots | Constrained by C-1 to C-4. |
| 8 | Fail-closed | The page never decides authorization; the server does. UI risk: a CSS rule revealing a `hidden` control or a color that implies a state the text does not state | Constrained by C-9, C-10, C-12. |
| 9 | Supply chain | No dependency, font, CDN or tool added to the build; `rsvg-convert` and `magick` run only on the developer machine to produce the committed PNG | `Cargo.toml`/`Cargo.lock`/`deny.toml` must stay unchanged (C-20); `cargo deny --locked check` still required. |
| 10 | CI / workflows | `.github/`, `scripts/`, `.githooks/` not touched | n/a (C-20). |

Additional verification done by the auditor:

- **Touch icon recipe (spec §6.2)**: run twice in the scratchpad from the owner `Icon_logo.svg`: identical SHA-256
  `3442af18…cd46b3`, 2 742 bytes, IHDR `180 x 180`, bit depth 8, color type 2, interlace 0, chunks exactly
  `IHDR, IDAT, IEND`. Matches the architect value and the test 71 limits.
- **Red state**: `cargo test --locked -p soos-invariants --all-features -q -- rmc_` gives `59 passed; 11 failed`,
  exactly the 11 tests listed as red in `AI/tester_contract_brand.md`.
- **Hooks used by `app.js`** (`grep className|classList|dataset|innerHTML|setAttribute`): only
  `#state.className = "state state-" + state`, `#state.dataset.state` and
  `#alerts.classList.toggle("alerts-attention")`. No `innerHTML`, no inline style. The redesign must style through
  exactly these hooks (spec D2).
- **Plan evaluator MINOR items**: N1 is resolved in test 68 (event handlers are matched on parsed attribute names,
  so `content=` no longer matches). N4 is resolved by the test: `--focus-on-blue` is a pinned role in `ROLES`, so it
  stays declared and must be documented as reserved (C-13). N3 (footer width) is a developer item (C-15). N2 was not
  folded in (see Residual risks R-1).

## Audit Constraints — GitHub #339 follow-up (soos brand on `soos-remote`)

| # | Constraint | Applies to (file::fn) | Verified by (test / lint / invariant / grep) |
|---|---|---|---|
| **A. Privacy and brand material** | | | |
| C-1 | `New_Gui_Interface.png` (shows the owner's face) and every other raster of the owner archive are never copied, embedded (no `data:`), traced or described pixel by pixel in the repository. The only raster in `crates/remote/assets/` is `apple-touch-icon.png`, regenerated from the repository `icon.svg`. | whole tree | test 73 (`test_rmc_s54_system_fonts_and_fixed_asset_set`, fixed seven-file set), test 68 (no `data:`); grep: `git status --porcelain` lists no new binary file other than `crates/remote/assets/apple-touch-icon.png`; `git grep -n New_Gui_Interface` adds no hit outside AI/ prohibition text |
| C-2 | Only the path data of `Icon_logo.svg` and `text_logo.svg` is copied (the six wordmark `d` strings, the four icon `d` strings, the two `brand.rs` star spikes). Inline SVGs carry no `fill`, `stroke`, `style`, `href`, `xlink:href`, `<use>`, `<image>`, `<foreignObject>`, `<script>` or `on*` attribute; `icon.svg` has only `#EDF1FF` and `#0047BB` and no `<script>`, `href`, `style`, `<image>`, `<foreignObject>`, `on*`. | `index.html`, `icon.svg` | tests 68, 70 (`test_rmc_s51_header_band_carries_the_wordmark`), 71 (`test_rmc_s52_icons_are_the_brand_mark`), 64 (hex literals of `icon.svg`) |
| C-3 | Visual-check material (stub server, mocked `/api` JSON, Playwright scripts, screenshots) lives only under the scratchpad (`…/scratchpad/remote_brand_visual/`, `…/scratchpad/remote_brand_shots/`) and never enters the repository. Mock data is synthetic: no real tailnet name, host name, login, passkey id, push endpoint or IP of the owner. | developer visual pass | grep: `git status --porcelain` shows no file outside the spec §1.1 list; `git grep -nE 'ts\.net' crates/remote/assets Docs/REMOTE_COMPANION.md` adds no hit |
| C-4 | `Docs/REMOTE_COMPANION.md` §2e and the walkthrough describe the design with token names and values only; no screenshot is linked or embedded, no personal identifier is written. | `Docs/REMOTE_COMPANION.md`, `AI/walkthroughs/190_remote_brand_redesign.md` | test 74 (`test_rmc_s55_brand_is_documented`); review |
| **B. CSP, scripts and network** | | | |
| C-5 | `app.js`, `sw.js` and every file under `crates/remote/src/` (including `assets.rs` and the CSP string in `http.rs::MANDATORY_HEADERS`) stay byte-identical. | `crates/remote/assets/{app.js,sw.js}`, `crates/remote/src/*.rs` | grep: `git diff --exit-code -- crates/remote/assets/app.js crates/remote/assets/sw.js crates/remote/src` must print nothing; test 73 (seven `include_bytes!` paths); existing `test_rmc_s8_*` |
| C-6 | `index.html`: no `<style>`, no `style=` attribute, no inline event attribute, exactly one `<script src="app.js"></script>` at the end of `body`, no `javascript:`, `data:` or `blob:`; the source link stays the plain `<a href="https://github.com/Mysticaly622/soos">source</a>`. | `index.html` | tests 68, 69; existing `test_rmc_s8_assets_exist_load_nothing_remote_and_have_no_inline_script` |
| C-7 | `style.css`: no `url(`, `@import`, `@font-face`, `expression(`; no web font, CDN or font file; `font-family` outside `:root` is only `var(--font)` / `var(--font-mono)`, with `--font` starting with `-apple-system`. | `style.css` | tests 68, 73; existing RMC-S8 |
| C-8 | No storage, no new script, no service-worker change: the page keeps `textContent`-only rendering because `app.js` is unchanged (C-5). | `app.js` | existing `test_rmc_s8_app_js_is_textcontent_only_and_carries_the_ui_constants`, `test_rmc_s18_page_uses_modal_webauthn_without_storage` |
| **C. State integrity of the page (fail-closed presentation)** | | | |
| C-9 | The rule `[hidden] { display: none !important; }` is kept verbatim in `style.css`, before the component rules, so no component rule (`.card`, `.actions`, `.table`, `.banner`, `button`) can reveal a control or card that `app.js` hides (`#login`, `#logout`, `#alerts`, `#alerts-ack`, `#push-*`, `#enroll`, `#status-card` on the Funnel login screen). No test pins this rule. | `style.css` | grep: `grep -n '^\[hidden\] {' -A2 crates/remote/assets/style.css` shows `display: none !important;`; visual check: Funnel login screenshot shows only the `#login` card; locked/unlocked screenshots show no `#login`, `#enroll` or `#push-enable` that the scenario hides |
| C-10 | State is always written in text by `app.js` (`#state`); color (dot, tile) is decorative only. The status dot is `aria-hidden="true"`, driven only by `.stat-tile:has(.state-<s>)`; `unknown` and `no_session` keep the idle dot, `unavailable` and `unreachable` the alarm dot; no state shows the locked (green) dot except `state-locked`. `#state` carries no class other than the ones `app.js` writes (it replaces `className`). | `index.html`, `style.css` | test 69 (`#state` class `state state-unknown`, `data-state`, text `Connecting`); grep: `grep -n 'dot-locked' crates/remote/assets/style.css` appears only in the role block and the `:has(.state-locked)` rule; visual check of `locked`, `unlocked`, `unreachable` |
| C-11 | Button semantics (spec D5, D12): `#lock` primary, `#unlock` `class="secondary danger"` in `--danger-fg` (`--danger-text` light, `--danger-soft` dark), never the filled danger style; labels unchanged; unlock still behind `confirm()` and the Face ID step in unchanged `app.js`. Disabled buttons use `--disabled-fill`/`--disabled-fg` (no opacity trick), so a disabled `Unlock now` never looks enabled. | `index.html`, `style.css` | tests 66 (bindings `button.secondary.danger` → `var(--danger-fg)`), 69 (labels, `secondary` on `#unlock`); existing `test_rmc_unlock_is_opt_in_and_documented` |
| C-12 | No push switch, and no other CSS-only indicator that claims "this phone receives alerts" (spec D11): no `switch`/`switch-knob` class, `role="switch"`, `aria-checked`, `.switch` selector or `:has(#push-disable` selector. `#push-state` stays the only statement of the push state. | `index.html`, `style.css` | test 69 |
| C-13 | `--focus-on-blue` stays declared (pinned in test `ROLES`) and is documented in a CSS comment and §2e as reserved for a future focusable element on the band or tile; no focusable element is placed on the band or tile in this change (the band pill is static text). | `style.css`, `index.html` | tests 65/66 (role present); grep: no `<button`, `<a ` or `<input` inside `<header class="band">` or `.stat-tile` |
| **D. Accessibility (owner requirement)** | | | |
| C-14 | Contrast, focus and touch rules of spec §8 hold in light and dark: text pairs ≥ 4.5:1, control boundaries ≥ 3:1, the selector-to-role bindings of test 66 exactly; `:focus-visible` ring ≥ 2 px in `var(--focus)`; `outline: none`/`0` only under `:focus:not(:focus-visible)`; `button` `min-height: var(--touch)` (48 px), input ≥ 44 px, footer link 44 px; safe-area insets on all four sides; reduced-motion block; viewport keeps zoom (no `maximum-scale`, no `user-scalable=no`). | `style.css`, `index.html` | tests 66 (`test_rmc_s47_text_contrast_meets_wcag_aa`), 67 (`test_rmc_s48_mobile_frame_focus_touch_motion`) |
| C-15 | Footer on wide viewports (plan evaluator N3): `footer` gets the same `max-width: 480px`, `width: 100%`, `margin: 0 auto` as `main.page`, so no full-width pale strip sits under the narrow body. | `style.css` | visual check at one wide width (for example 1024 x 768, light) in the scratchpad |
| C-16 | No horizontal scroll and the state word stays inside the tile at 390, 375 and 320 px (spec F11: `clamp()` display sizes, `overflow-wrap: anywhere`). | `style.css` | visual check: `document.documentElement.scrollWidth <= window.innerWidth` asserted by the Playwright script at every width |
| **E. Icons, manifest, meta** | | | |
| C-17 | `apple-touch-icon.png` is produced by exactly the spec §6.2 command from the repository `icon.svg`, run in the scratchpad and copied in: 180 x 180, bit depth 8, color type 2, no `tRNS`/`tIME`/text chunks, ≤ 8 KiB. Expected bytes: 2 742, SHA-256 starting `3442af18` (auditor reproduction). It is never regenerated in CI. | `crates/remote/assets/apple-touch-icon.png` | test 71; existing `server_tests::test_rmc_assets_are_served_with_content_types` (PNG magic); grep: `sha256sum crates/remote/assets/apple-touch-icon.png` |
| C-18 | Manifest: only `theme_color` `#0047BB` and `background_color` `#EDF1FF` change; `name`, `short_name`, `description`, `display`, `start_url`, `scope` and both icon entries stay identical. `index.html`: one `theme-color` meta `#0047BB`, status bar style `black-translucent`, `color-scheme` meta kept. | `manifest.webmanifest`, `index.html` | test 72 (`test_rmc_s53_manifest_and_meta_colors`); existing `routes_tests` (manifest fields) |
| **F. Tests, contract migration, gates** | | | |
| C-19 | CM-1 is the **only** edit to an existing test file: the body of `blank_string_literals` in `tests/invariants/src/remote_alerts_contract.rs`. Rule: `r` opens a raw string when followed by `#*"` and either (a) the byte before `r` is not an identifier byte, or (b) the byte before `r` is `b` or `c` and the byte before that is not an identifier byte (or `r` is at index 1). Index arithmetic is guarded (`i >= 1` / `i >= 2` checked before `bytes[i - 1]` / `bytes[i - 2]`), so a snippet starting with `r`, `br` or `cr` cannot panic. `let xbr = 1;` stays code. No assertion of `test_rmc_s43_*` changes. | `tests/invariants/src/remote_alerts_contract.rs::blank_string_literals` | tests 62 (`test_rmc_s43_acknowledged_entries_are_not_kept`), `test_rmc_s43_scanner_self_test`, 75 (`test_rmc_s56_scanner_handles_raw_byte_and_c_strings`); grep: `git diff tests/invariants/src/remote_alerts_contract.rs` changes no line inside a `#[test]` function body other than the new test 75 already written by the tester |
| C-20 | No test of `tests/invariants/src/remote_brand_contract.rs` or of any other existing test file is modified, weakened or deleted by the developer; no dependency, `deny.toml`, workflow, script or hook change; `Cargo.toml`/`Cargo.lock` unchanged. | whole tree | grep: `git diff --stat -- tests/invariants/src/remote_brand_contract.rs Cargo.toml Cargo.lock deny.toml .github scripts .githooks` empty after the developer phase (the tester's untracked module is compared with `sha256sum` before and after) |
| C-21 | Walkthrough 188 §R3.7 is reworded: the scanner flaw could give a false negative (a trailing backslash in `br"…"` hid the code after it), now fixed by CM-1. English only; the word for raw-memory code is not used in any added text. | `AI/walkthroughs/188_remote_auth_alerts.md` | review; grep: `git diff` contains no occurrence of that keyword (case-insensitive) |
| C-22 | Gates before hand-off: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `cargo test --workspace --locked --all-features`, `cargo deny --locked check`; plus the Playwright visual pass (390 x 844, DPR 3, light and dark, scenarios `locked`, `unlocked`, `funnel-login`, `alerts-3`, `push-enabled`; narrow widths per spec §9.1) with the production CSP header sent by the stub server and zero console errors or CSP violations; every screenshot read before finishing. The stub server binds `127.0.0.1` only, is a Python stdlib script run with `-I` (or Node) from the scratchpad, and serves `crates/remote/assets/` read-only by path argument. | developer phase | command output recorded in the walkthrough; screenshots in the scratchpad only |
| C-23 | Process: no commit, push, stash, PR, `tailscale`, host-service restart or `~/.config` change by any agent. | all phases | review |

### Pre-existing violations found (not introduced by this change)

- None in the audited scope. Walkthrough 188 §R3.7 contains an incorrect claim ("the failure direction is a false
  positive, never a hidden violation"); it is documentation, corrected by C-21.
- Walkthrough numbering collision with `origin/main` (`183_*`, `184_*` existed on both lines) predated this change
  (spec open point 1); resolved on 2026-10-06 by renumbering this branch's walkthroughs +2 (now `185_*`–`190_*`).

### Residual risks accepted (documented, not blocking)

- **R-1 (plan evaluator N2, test strength)**: in test 75 the `b'"'` case is the last statement of the negative
  snippet, so a regression that lets `b'"'` open a string would swallow nothing and pass. The current helper already
  keeps `'"'` whole (char-literal branch), so behaviour is correct today. Recommended to the tester (not to the
  developer, who may only edit the helper body) to add the positive case `let e = b'"'; let acknowledged = 1;` → 1
  use; not a precondition for implementation.
- **R-2**: `[hidden] { display: none !important; }` is protected by C-9 (grep plus visual check) but by no automated
  test; a later tester round may pin it in test 69 or 68.
- **R-3**: CSS generated content with alternative text (`content: "!" / "";`) is ignored by older Safari, which then
  reads the glyph; harmless (the fallback declaration comes first, the paragraph text carries the information).
- **R-4**: iOS caches the home-screen icon; the owner must remove and re-add the web app to see the new star mark
  (documented in `Docs/REMOTE_COMPANION.md` §4, matrix RMC88 pending on hardware).
- **R-5**: the danger-outline color deliberately differs from the GUI `DangerOutline` (`DANGER`, 4.22:1, below AA);
  the GUI should follow later (spec D12, open point 4). Recorded in the ADR.

### Clearance: CLEARED

The change is presentation only, adds no dependency, network path, script or Rust code, keeps CSP, routes, ids,
labels and the service worker byte-identical, and the test contract covers the security-relevant properties (no
inline script or style, no `data:` URI, fixed asset set, brand vectors only, no misleading push switch). The developer
may start, bound by C-1 to C-23.

status: cleared
