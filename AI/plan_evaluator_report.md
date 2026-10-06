# Plan Evaluation Report
- **Date**: 2026-10-06
- **Issue**: GitHub #339 follow-up — soos brand direction applied to the `soos-remote` web app (owner request 2026-10-06)
- **Branch**: `feat/remote-auth-alerts` (base `ed52b02`)
- **Base commit**: `47ab53e` (`origin/main`)
- **Spec evaluated**: `AI/architect_spec_remote_brand.md`, round 2 (with `AI/design_brief_remote_brand.md` and the drafted ADR in `AI/DECISIONS.md`)
- **Previous round**: round 1, REVISION_REQUIRED (F1–F3 MAJOR, F4–F12 MINOR)

## 1. Coverage Matrix

| Owner requirement / acceptance line | Spec element | Status |
|---|---|---|
| Palette identical to `origin/main:crates/gui/src/theme.rs`, CSS custom properties named after the constants | §2.2, D3, D4, test 63 | Covered (live parse now accepts every form `theme.rs` uses, §2) |
| Logo: SOOS wordmark inline SVG on the blue band | §4.2, test 70 | Covered |
| Star mark as `icon.svg` + favicon; `apple-touch-icon.png` 180 x 180 opaque, regenerated from the vector | §6.1, §6.2, D9, test 71 (color type 2 exactly) | Covered |
| Manifest `theme_color` / `background_color` from the palette | §6.3, D6, test 72 (per-entry icon check) | Covered |
| Mobile first, standalone, safe-area insets | §2.4 `--gutter`, band/main/footer `env()`, D13, test 67 | Covered |
| Light default + dark variant from the same palette (ink body) | §2.2b, §2.3, test 65 | Covered (tints recomputed, §2) |
| WCAG AA text contrast | §8, test 66 (pairs + selector-to-role bindings) | Covered (recomputed, §2) |
| Visible focus, 44 px touch targets | §3 Focus/Buttons/Footer, §8, test 67 | Covered |
| Status as a big blue tile with white text like the GUI stat tiles | §3 Stat tile, §4.2, `clamp()` display sizes | Covered |
| Button semantics (primary blue, danger for unlock) | D5, D12, §3 | Covered; deviation from the GUI recorded (D12, ADR, open point 4) |
| Alerts banner and push card in card style | §3 Banner/Table, §4.2, D11 (no switch) | Covered |
| Every id, label, behaviour kept; every UI state reachable | §4.3, §5, test 69, D1 | Covered (29 ids, `[hidden]` rule kept, `app.js` frozen) |
| CSP unchanged; no inline script/style, no remote font/CDN, no `data:` | §0.1.6, §7, tests 68, 73 | Covered; N1 (test 68 regex wording) |
| `textContent` only, no storage, service worker unchanged | D1 (`app.js`, `sw.js` byte-identical) | Covered |
| System font stack only | §2.4, test 73 | Covered |
| No archive raster; never `New_Gui_Interface.png` | §0.1.5, ADR item 4, test 73 (fixed asset set), test 68 (no `data:`) | Covered |
| Rust untouched beyond `assets.rs` needs | D1, §1.1 (no Rust change at all) | Covered |
| Playwright visual checks: 5 states x light/dark at 390 x 844 @3x, stub server, scratchpad only | §9.1 (+ 375/320 px narrow checks) | Covered |
| Contract Migrations in `AI/tester_contract_brand.md` | §13 (CM-1), test 74 | Covered |
| Optional scanner fix (`br`/`cr`), self-test, walkthrough 186 R3.7 reword | D10, test 75, CM-1, §1.1 | Covered; N2 (one case without power) |
| Docs `Docs/REMOTE_COMPANION.md` §2e + §4 icon-cache note | §10, test 74 | Covered |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| 25 palette names and values | `origin/main:crates/gui/src/theme.rs` l.24–73 | 24 `Color32::from_rgb(0x.., 0x.., 0x..)` equal to §2.2, `WHITE` = `Color32::WHITE` | Yes |
| Numeric constants and their forms | `theme.rs` l.103–117 | `BODY_TOP_RADIUS: u8 = 20`, `GAP_CARD: f32 = 14.0`, `R_CARD: u8 = 20`, `R_TABLE: u8 = 14`, `R_INPUT: u8 = 10`, `STROKE_CARD: f32 = 2.0`; neighbours `R_VIDEO`, `R_BUTTON`, `EYE_AXIS` (`from_rgba_premultiplied`) | Yes — every form is accepted by test 63's parser (round-1 F2 resolved) |
| 14 dark tints with `(a·(100−n) + b·n + 50) / 100` | §2.2b | Recomputed in integer arithmetic (scratchpad script): all 14 equal the table, incl. `--pale-weak #7F8590`, `--blue-soft #82A5E0` | Yes (round-1 F1 resolved) |
| Contrast, light | recomputed (WCAG 2.x) | text pairs min 4.76 (`on-danger`/`danger-fill`); `danger-fg`/`card` 8.43; `secondary-fg`/`secondary-pressed` 5.53; non-text: `dot-idle`/`strip` 3.48, `focus`/`card` 7.11; banner glyph cut-outs (`-bg` on `-icon`) all ≥ 3 | Yes |
| Contrast, dark | recomputed | text min 4.55 (`secondary-fg`/`secondary-pressed`), `danger-fg`/`card` 6.09, `link`/`page` 7.16; non-text `dot-idle`/`strip` 4.28, `focus`/`card` 6.36 | Yes (matches §2.2b) |
| 29 ids = the `getElementById` set of `app.js` | `app.js` l.120–148 | 29 `getElementById("…")` calls; no `querySelector`, no other `classList`/`className` write | Yes |
| `#state` class replaced; `.alerts-attention` toggled | `app.js` l.275, l.467 | `stateNode.className = "state state-" + …`; `classList.toggle("alerts-attention", …)` | Yes (D2 hooks exist) |
| Funnel login hides status card and Sign out | `app.js` l.252–266 | `statusCard.hidden = true; logoutButton.hidden = true` / restored on session | Yes (§4.3) |
| Push "on" condition is device-agnostic | `app.js` l.925 | `pushDisableButton.hidden = count === 0` with `count = view.subscriptions` | Yes; switch dropped (D11, round-1 F3 resolved) |
| GUI danger outline color | `origin/main:crates/gui/src/widgets.rs` l.712–719 | `DANGER` text and border | Deviation recorded (D12) |
| `STAR_SPIKES`, `Icon_logo.svg`, `text_logo.svg` geometry | `brand.rs` l.157–170, owner archive | as §4.2/§6.1 (verified round 1, unchanged) | Yes |
| Touch-icon command determinism (2 742 B, `3442af18…`) | re-run in round 1 | identical | Yes |
| Current markup needles of existing tests | `index.html`, `remote_*_contract.rs`, `crates/remote/tests` | no test pins `h1`, `main.card`, `#1f2937`, `detail` on `#alerts-coverage`, meta values or manifest colors; S8 checks ` onclick=`/` onload=`/` onerror=` with a leading space | Yes (§13: CM-1 only) |
| `blank_string_literals` current behaviour | `remote_alerts_contract.rs` l.424–484 | raw start requires `r` at an identifier boundary, so `br"C:\"` opens an escaped string; char literals (incl. `'"'`) kept whole | Yes (test 75 red on current helper, both the positive and the `br#"x"acknowledged"#` negative case) |
| Walkthrough 186 R3.7 claim | `AI/walkthroughs/186_remote_auth_alerts.md` l.190–191 | "the failure direction is a false positive, never a hidden violation" | Yes (claim wrong, reword planned) |
| Brief and ADR consistent with round 2 | `AI/design_brief_remote_brand.md`, `AI/DECISIONS.md` diff | no `#7E8490`/`#82A4E0`; switch removed (brief §5.8); ADR states integer rounding, no switch, D12 deviation | Yes |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario considered: the redesign needs a CSP relaxation (inline style, web font, remote image) on a Funnel-exposed origin.
- Design excludes it: `http.rs` frozen (D1); tests 68 and 73 forbid `url(`, `@import`, `@font-face`, `<style`, ` style=`, event attributes, `data:`/`blob:`/`javascript:`, and pin the seven-file asset set; inline SVGs carry no `href`/`<use>`/`<foreignObject>`. No PAM, daemon, IPC or socket path is touched.
- Result: PASS.

### Pillar 2 — PAM deadline & concurrency
- Failure scenario considered: a Rust change slipping into `crates/pam` or a latency path.
- Design excludes it: no Rust source change at all; static assets only.
- Result: PASS (not applicable).

### Pillar 3 — Panic safety & fail-closed (UI-state reachability and truthful security state)
- Scenario 1: a wrapper (`.actions { display: flex }`, `.table`) reveals an element `app.js` hid. Excluded: `[hidden] { display: none !important; }` kept; `.table:empty` only hides.
- Scenario 2: a decoration states a security property the text does not (round-1 F3). The switch is gone (D11), and the status dot derives only from `#state.state-*`, the same state written as text (D8). PASS.
- Scenario 3: the destructive action becomes the most prominent control. `Unlock now` is a danger outline; `confirm()` and Face ID unchanged (D5). PASS.
- Scenario 4: a long state word overflows the tile on a 320 px phone and is clipped (state hidden). `clamp()` sizes, `overflow-wrap: anywhere` and narrow-width visual checks (§9.1). PASS.
- Result: PASS.

### Pillar 4 — Dependencies
- Failure scenario considered: a PNG or CSS crate added to the invariants crate, or a font file.
- Design excludes it: dependency-free tests (PNG chunks parsed by hand, integer tint math, local WCAG formula), system fonts only, `Cargo.lock`/`deny.toml` untouched.
- Result: PASS.

### Pillar 5 — Data confidentiality
- Failure scenario considered: the owner's face (`New_Gui_Interface.png`) or another archive raster enters the repository, directly or as a `data:` URI or traced image.
- Design excludes it: §0.1.5, ADR item 4, touch icon rendered only from the repository `icon.svg`, test 73 pins the asset set, test 68 forbids `data:`; screenshots stay in the scratchpad (§9.1).
- Result: PASS.

### Pillar 6 — Test integrity and test power
- Wrong implementation A — tint computed with float and truncation (`#7E8490`): test 64's integer recomputation and self-test (`mix(PALE, INK, 50) == 0x7F8590`) fail it. PASS.
- Wrong implementation B — after rebase, `theme.rs` changes `BLUE` but the CSS does not: test 63's live branch compares against the pinned table and fails; the parser is exercised now by the excerpt self-test (incl. `WHITE`, `u8`, `f32`, prefix neighbours, unsupported form). PASS.
- Wrong implementation C — `.alerts-attention .alerts-summary { color: var(--danger-fill) }`: test 66 binding requires `var(--banner-danger-fg)` and forbids `--danger-fill` as `color:`. PASS.
- Wrong implementation D — `color: var(--blue)` in a component: test 64 token-by-token rule rejects palette names outside role blocks (self-test covers it). PASS.
- Wrong implementation E — an `onfocus=` handler added to a button: test 68's "any `on[a-z]+=` attribute" catches it, **but** as worded the pattern also matches `content=` of every `<meta>` (`c` + `ontent=`), so a literal implementation is red against a correct page and could only be fixed by editing an immutable test. → N1 (MINOR).
- Wrong implementation F — a regression making `b'"'` open a string literal: in test 75 that case is the last statement of the negative snippet, so the swallowed tail contains nothing and the test still passes. → N2 (MINOR).
- Test 69 passes on the current page by design (regression guard), declared and recorded. Existing tests unchanged; CM-1 only strengthens a helper.
- Result: FINDING (N1, N2, both MINOR).

## 4. Findings

Round-1 findings: F1–F12 all resolved as stated in spec §16 (verified: tints, parser forms, switch removal in spec/brief/ADR, token-by-token rule with self-test, selector bindings, D13 frame, test names s44–s56, D12, manifest entries, glyph cut-out roles ≥ 3:1, `clamp()` + narrow checks, color type 2).

New in round 2 (none blocking):

- **[MINOR] N1 — Test 68 event-attribute pattern needs an attribute-name boundary.** "no `on[a-z]+=` attribute" read as a plain regex over `index.html` matches `content=` in every `<meta>` tag. Required wording for the tester: match only attribute names, i.e. `on[a-z]+\s*=` preceded by whitespace inside a tag (lowercased source), with a self-test: `<button onfocus="x">` rejected, `<meta name="a" content="b">` accepted, `<p>button=</p>` text accepted.
- **[MINOR] N2 — Test 75's `b'"'` case has no power.** Placed last in the negative snippet, a mis-handled `b'"'` swallows nothing. Add a positive case where code follows it, e.g. `let e = b'"'; let acknowledged = 1;` → exactly 1 use.
- **[MINOR] N3 — Footer width on wide viewports.** `main.page` is capped at 480 px and centered on the blue body, but `footer` spans the full width with `background: var(--page)`, so on a tablet/desktop a full-width pale strip sits under a narrow pale body. Give `footer` the same `max-width: 480px; margin: 0 auto; width: 100%` (it stays on `--page`, so `--link` contrast is unchanged). No effect at 390 px; check once in the visual pass at a wide width.
- **[MINOR] N4 — `--focus-on-blue` is declared but bound to nothing.** No focusable element sits on blue today (the band pill is static), so it is harmless; either drop the role or state in §2.3 that it is reserved for a future focusable element on the band/tile (light `--focus` on blue is 1.0:1, so it must be used if one is ever added).

Observations (no change required): disabled outline `#unlock` uses an inset `--divider` border at 1.37:1 — inactive controls are exempt from WCAG 1.4.11 and the label stays at 3.48 (light) / 4.28 (dark); banner borders are low-contrast but decorative (the banner text and glyph carry the meaning).

## 5. Verdict

No CRITICAL or MAJOR finding. The round-2 spec is faithful to the brand sources and to `origin/main:crates/gui/src/theme.rs` (every value and declaration form verified), keeps every UI state reachable through the unchanged `app.js` hooks, leaves CSP and the asset set untouched, meets WCAG AA in both themes (independently recomputed), and its tests fail plausible wrong implementations. The four MINOR items (N1–N4) should be folded in by the tester (N1, N2 as test wording) and developer (N3, N4) without another evaluation round.

VALIDATION_VERDICT: APPROVED
