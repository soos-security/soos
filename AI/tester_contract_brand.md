# Tester Contract — GitHub #339 follow-up: soos Brand Direction Applied to the `soos-remote` Web App

- **Date**: 2026-10-06
- **Branch**: `feat/remote-auth-alerts` (base `ed52b02`)
- **Spec**: `AI/architect_spec_remote_brand.md` round 2 (plan evaluation `APPROVED`), §9 tests 63–75
- **ADR**: "[2026-10-06] soos Brand Direction Applied to the `soos-remote` Web App"
- **Matrix rows**: RMC76–RMC87 (RMC88 is the owner's iPhone check, no automated test)
- **Owner request (2026-10-06, binding)**: completely redesign the web app to follow the soos brand direction
  ("follow everything": palette, logo, style).

## Tester Contract

All tests are static invariants in the dependency-free `soos-invariants` crate. Tests 63–74 live in the new module
`tests/invariants/src/remote_brand_contract.rs` (registered in `tests/invariants/src/lib.rs` under
`#[cfg(all(test, unix))]`); test 75 lives in `tests/invariants/src/remote_alerts_contract.rs`.

Red run: `cargo test --locked -p soos-invariants --all-features` on the current assets gives
`508 passed; 11 failed` and the 11 failures are exactly the ones in the table below.

| Test (path::name) | Spec test / matrix | Red evidence (failure message on the current assets) |
|---|---|---|
| `remote_brand_contract::test_rmc_s44_brand_tokens_match_the_gui_theme` | 63 / RMC76 | `style.css must declare --blue (theme.rs BLUE) exactly once, found 0` |
| `remote_brand_contract::test_rmc_s45_style_colors_come_from_the_palette_only` | 64 / RMC77 | `--ink-2 declared once in a top-level :root block` (left 0, right 1) |
| `remote_brand_contract::test_rmc_s46_light_default_and_dark_variant` | 65 / RMC78 | `light :root must define --page exactly once` (left 0, right 1) |
| `remote_brand_contract::test_rmc_s47_text_contrast_meets_wcag_aa` | 66 / RMC79 | `role --page is not defined` |
| `remote_brand_contract::test_rmc_s48_mobile_frame_focus_touch_motion` | 67 / RMC80 | `--touch declared in px in :root` |
| `remote_brand_contract::test_rmc_s49_no_inline_style_script_or_data_uri` | 68 / RMC81 | **Passes already** (see note 1) |
| `remote_brand_contract::test_rmc_s50_ui_contract_ids_labels_and_attributes_are_kept` | 69 / RMC82 | **Passes already, by design** (regression guard, spec §9) |
| `remote_brand_contract::test_rmc_s51_header_band_carries_the_wordmark` | 70 / RMC83 | `index.html must open <header class="band">` |
| `remote_brand_contract::test_rmc_s52_icons_are_the_brand_mark` | 71 / RMC84 | `icon.svg viewBox` (left `0 0 180 180`, right `0 0 508 508`) |
| `remote_brand_contract::test_rmc_s53_manifest_and_meta_colors` | 72 / RMC85 | `manifest must contain "theme_color":"#0047BB"` |
| `remote_brand_contract::test_rmc_s54_system_fonts_and_fixed_asset_set` | 73 / RMC86 | `--font must start with -apple-system, got None` |
| `remote_brand_contract::test_rmc_s55_brand_is_documented` | 74 / RMC87 | ``Docs/REMOTE_COMPANION.md needs a section `## 2e.` `` |
| `remote_alerts_contract::test_rmc_s56_scanner_handles_raw_byte_and_c_strings` | 75 / RMC87 | `words inside raw byte, raw C and byte strings are not code` (the `br#"x"acknowledged"#` case; the `br"C:\"` false-negative cases follow) |
| `remote_brand_contract::test_rmc_brand_helper_theme_parser_self_test` | 63 helper | Green (helper self-test, spec §9 test 63) |
| `remote_brand_contract::test_rmc_brand_helper_mix_contrast_and_color_rule_self_test` | 64 / 66 helpers | Green (helper self-test) |
| `remote_brand_contract::test_rmc_brand_helper_html_self_test` | 69 helper | Green (helper self-test) |
| `remote_brand_contract::test_rmc_brand_helper_png_self_test` | 71 helper | Green (helper self-test: rejects color types 0 and 6, `tRNS`, `tIME`) |

Notes:

1. Spec §9 expected test 68 to be red. It is green on the current page because the current assets already contain
   no inline style, handler, `data:` URI or inline SVG; test 68 is a guard that the redesign (which adds inline SVGs)
   keeps it that way. This is recorded here and does not change the test.
2. Test 69 is green on the current page by design (spec §9): it pins the UI contract the redesign must keep.

### Positive control (the contract is satisfiable)

A minimal spec-conforming prototype (palette, tints, scale and role blocks; the §4.2 markup with the wordmark and the
two star SVGs; the owner's `Icon_logo.svg`; the manifest colors; the touch icon from the §6.2 command, 2 742 bytes)
was built in the scratchpad, copied over the assets temporarily, and the suite run: tests 63–73 all pass (only 74,
documentation, and 75, helper, stayed red). The tracked assets were restored with `git checkout -- crates/remote/assets`
right after. The CM-1 helper change below was tried the same way: tests 62, the 62 self-test and 75 pass with it; the
file was restored afterwards, so test 75 is red in the hand-off.

### Reading guide for the developer (what the tests accept)

- **CSS parsing**: comments are removed; one level of at-rule nesting; selector items are whitespace-normalized
  and compared exactly. A `var()` with a fallback (`var(--a, b)`) is rejected.
- **Palette and tints** (63, 64): each declared once in a top-level `:root` rule, value exactly `#RRGGBB` in uppercase;
  never redeclared in the dark block. Hex literals anywhere else in `style.css` (outside comments) fail.
- **Roles** (65, 66): every role of spec §2.3 exactly once in the top-level `:root`, as exactly `var(--palette-or-tint)`.
  The dark `:root` may hold only role tokens (and `color-scheme`); every role whose dark value differs must be there.
- **Color-bearing properties outside `:root`** (64): `var()` only of roles (§2.3) or scale tokens (§2.4); other words
  must be lengths/numbers or one of the non-color keywords in `NON_COLOR_KEYWORDS` (`solid`, `inset`, `none`,
  `transparent`, `currentColor`, `calc`, `env`, `safe-area-inset-*`, …). `linear-gradient(` only in the `html` rule
  as `linear-gradient(var(--band) 50%, var(--page) 50%)`.
- **Bindings** (66): accepted selector spellings are listed in `BINDINGS` (for example
  `.alerts-attention .alerts-summary` or `#alerts.alerts-attention .alerts-summary`; `.banner-warn` or
  `.banner.banner-warn`). Every rule with such a selector item that declares the property (for `background`, also
  `background-color`) must use exactly the listed `var()`. `color:` never uses `--danger-fill`, `--dot-locked`,
  `--dot-unlocked` or `--star`.
- **Focus and touch** (67): `button { min-height: var(--touch) }`; the code input rule (`input`, `#enroll-code` or
  `input#enroll-code`) and `footer a` have `min-height` ≥ 44 px (or `var(--touch)`); a `:focus-visible` rule has
  `outline: <n>px … var(--focus)` with n ≥ 2; `outline: none`/`0` only in selectors ending with
  `:focus:not(:focus-visible)`.
- **HTML** (68–71): attribute names are matched lowercase (`viewBox` is read as `viewbox`). Star SVGs are recognised by
  `viewBox="0 0 508 508"`; one must have the class `stat-star`, one must sit directly inside the `.login-tile`
  element; each is `aria-hidden="true"` itself or through that direct parent.
- **Fonts** (73): every `font-family` declaration outside `:root` is exactly `var(--font)` or `var(--font-mono)`.

## Migrated existing tests

| ID | Test / helper | Old behaviour | New behaviour | Mandating line |
|---|---|---|---|---|
| CM-1 | `tests/invariants/src/remote_alerts_contract.rs::blank_string_literals` (helper of test 62 `test_rmc_s43_acknowledged_entries_are_not_kept`) | Only `r"…"` / `r#"…"#` were raw strings; `br"C:\"; let acknowledged = true; let q = "x";` was scanned as one escaped string, hiding the binding (false negative) | `br`, `cr` (at an identifier boundary, before `"` or `#…"`) also open raw strings | Spec D10 and §13 CM-1 (owner-optional small fix accepted in scope) |

CM-1 strengthens the scanner: no assertion of test 62 or of its self-test changes, and test 75 is new. The **only**
edit the developer may make to an existing test file is the body of `blank_string_literals` (raw-string prefix
detection), so that test 75 turns green while `test_rmc_s43_acknowledged_entries_are_not_kept` and
`test_rmc_s43_scanner_self_test` stay green unchanged. Walkthrough 188 §R3.7 must be reworded in the traceability
phase (its "false positive only" claim is wrong, spec D10).

No other existing test is migrated: no test in `tests/` or `crates/remote/tests/` pins the old colors, the `h1`
placement, the `main.card` class, the `#alerts-coverage` `detail` class, the meta values or the manifest colors
(checked with `grep` for `1f2937`, `main class`, `<h1>`, `status-bar-style`, `theme_color`, `background_color`).

## Flakiness check

All tests are static file checks with no timing, network or randomness. `cargo test --locked -p soos-invariants
--all-features -q -- rmc_` was run 10 times in a row: 10 × `59 passed; 11 failed`, identical failure set.

## Quality

`cargo fmt -p soos-invariants` applied; `cargo clippy --locked -p soos-invariants --all-features --all-targets -- -D
warnings` clean. No dependency added (`Cargo.toml`/`Cargo.lock` unchanged). No production file, asset, `app.js`,
`sw.js` or Rust source under `crates/` was changed by the tester.
