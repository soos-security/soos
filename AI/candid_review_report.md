# Candid Review Report

- **Date**: 2026-10-06
- **Target Branch**: `feat/remote-auth-alerts`
- **Base (merge-base)**: `222665f`
- **Reviewed-Diff-Fingerprint**: `7bf5595f962d10d7617c55f7e560f19e6850deff615b12220d63ef3aab595746`
- **Audited Files**: full frozen patch `target/candid_diff.patch` (merge-base `222665f` to the working tree, 114 files). The last approved state is HEAD `ed52b02`; the delta reviewed in depth is the uncommitted soos brand redesign and its documentation: `crates/remote/assets/{index.html,style.css,icon.svg,apple-touch-icon.png,manifest.webmanifest}`, `tests/invariants/src/{lib.rs,remote_alerts_contract.rs,remote_brand_contract.rs}`, `AI/{ARCHITECTURE,DECISIONS,VERIFICATION_MATRIX}.md`, `AI/walkthroughs/{188_remote_auth_alerts,190_remote_brand_redesign}.md`, `Docs/REMOTE_COMPANION.md`, `AI/{architect_spec_remote_brand,auditor_constraints_brand,design_brief_remote_brand,tester_contract_brand}.md`.

## 1. Executive Summary

**Re-review (new fingerprint `7bf5595f…595746`).** The previous report approved fingerprint
`b37d69c9…` with one MINOR finding (icon command output path). Since that report, the only file
modified in the tree is `Docs/REMOTE_COMPANION.md` (checked with `find -newer` on the previous
report, excluding `.git/` and `target/`); its section 2e command now ends in
`png:crates/remote/assets/apple-touch-icon.png`. I re-ran that exact command (output redirected
to the scratchpad): SHA-256 `3442af183a9d1e3b50983612d927aa333453c56557aeb571a56950eff5cd46b3`,
byte-identical to the committed `crates/remote/assets/apple-touch-icon.png`. No stray PNG exists
in the repository root. Carried-over checks re-run on the current tree: CSP at
`crates/remote/src/http.rs:263` unchanged and no `crates/remote/src` change since HEAD; element
id set of `index.html` identical to HEAD; no inline `style=`, handler or `data:` URI added (the
only external URL is the pre-existing `source` link); no assertion removed and no `#[ignore]`
added under `tests/` since HEAD; `cargo test -p soos-invariants`: 519 passed, 0 failed. The
MINOR finding is resolved; the full-diff review below is carried over unchanged.

**Original summary (fingerprint `b37d69c9…`):**

The delta since the approved HEAD is presentation and documentation only. `git diff HEAD --stat`
shows no change to any file under `crates/remote/src/`, to `app.js` or to `sw.js`; the Content
Security Policy (`crates/remote/src/http.rs:263`, `default-src 'self'; script-src 'self';
style-src 'self'; img-src 'self'; ...`) is therefore unchanged and the new markup needs nothing
beyond it (inline SVG elements, no inline style, script, handler or `data:` URI). The new touch
icon is a flat graphic regenerated from `icon.svg`: re-running the documented `rsvg-convert` +
ImageMagick pipeline here produced a byte-identical file (SHA-256 `3442af18...cd46b3`), so it is
not a raster from the owner's archive and contains no photo. Every element id `app.js` reads
(29 `getElementById` targets) is present, the id set is identical to HEAD, and `[hidden]` is
forced to `display: none !important`, so the `.card { display: flex }` rule cannot keep a
hidden section visible. Tests: `soos-invariants` 519 passed, `soos-remote` all suites passed,
`clippy -D warnings` and `fmt --check` clean. One MINOR documentation finding; verdict APPROVED.

## 2. Test Changes

Mechanical listing (step 3) on the frozen patch:

- Test files touched: 35 (whole branch). Delta since HEAD: `tests/invariants/src/lib.rs`
  (registers `remote_brand_contract`), `tests/invariants/src/remote_alerts_contract.rs`,
  new `tests/invariants/src/remote_brand_contract.rs`.
- Removed/changed assertion lines: one hit, `presence_unlock_contract.rs`
  (`test_pau_zbus_is_used_only_by_the_daemon`), in the already approved committed part; it is a
  documented contract migration (ADR 2026-10-05 item (7)) that widens the allowed set to exactly
  two manifests and adds an exactly-once check; not a weakening.
- New escape hatches: none in code (the only hit is a walkthrough sentence).
- Inline `mod tests` changes: one hit adding `remote`, `push-protocol`, `push-sender` to the
  forbid-unsafe business crate list (committed, strengthening).
- `remote_alerts_contract.rs` delta: the `raw_start` condition of `blank_string_literals` now
  also accepts an `r` preceded by a `b`/`c` prefix at an identifier boundary (Contract Migration
  CM-1). No assertion was removed; new test 75
  `test_rmc_s56_scanner_handles_raw_byte_and_c_strings` checks four hidden-binding inputs that
  the old scanner missed and a negative case (`xbr` identifier). This strengthens the RMC-S43
  guard (closes a false negative); justified by matrix RMC87.
- `remote_brand_contract.rs` (new, 16 tests incl. 4 helper self-tests): pinned palette and
  metrics with a live cross-check against `crates/gui/src/theme.rs` when present (absent on this
  merge-base; I verified the 25 colors and 6 metrics against `origin/main:crates/gui/src/theme.rs`
  at `47ab53e`, all equal, so the live branch will pass after a rebase); palette-only colors;
  WCAG contrast from tokens; ids/labels kept; no inline style/script/`data:`; touch-icon PNG
  structure; manifest colors; asset set. The walkthrough records red evidence
  (`508 passed; 11 failed` on the old assets), so the tests can fail.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Scenario: a section the JS hides stays visible because of the new `display: flex` on `.card`.
  `[hidden] { display: none !important; }` precedes all component rules -> PASS.
- Scenario: an id renamed or dropped breaks a UI state (login, status, alerts with ack and
  coverage, push enable/test/disable/hint, enroll, logout, unlock). Id sets of HEAD and working
  tree compared with `diff`: identical; `app.js` unchanged -> every state reachable -> PASS.
- Scenario: `stateNode.className = "state state-<s>"` overwrites a styling class. The new
  markup puts tile classes on the parent `.stat-tile` and uses `:has(.state-*)` for the dot, so
  the overwrite loses nothing; on browsers without `:has()` the dot stays idle grey while the
  state is still written in words -> PASS.
- Scenario: `Unlock now` changed class to `secondary danger`; no JS sets button classes
  (only `stateNode.className` and `alerts.classList.toggle`) -> PASS.
- Scenario: `black-translucent` status bar hides content under the notch. The band pads with
  `env(safe-area-inset-top)`, gutters use left/right insets, the footer uses the bottom inset
  -> PASS (on-device check is matrix row RMC88, owner).

### PAM Concurrency & Deadlines
- No change to `crates/pam` in the branch delta; the remote crate is a leaf with no PAM link
  -> PASS (not applicable).

### Panic Safety & Fail-Closed
- No production Rust changed since HEAD. Lock/unlock authorization (CSRF header, passkey
  assertion, `allow_unlock`) lives server-side and is untouched; CSS cannot enable a disabled
  button's request path (`unlockButton.disabled` is still computed in `app.js`) -> PASS.

### Test Integrity & Anti-Weakening
- See section 2. No assertion removed or loosened in the delta; one test-only scanner made
  stricter with a regression test -> PASS.
- Scenario: contrast test passes while opacity lowers real contrast. Checked by hand: pale on
  blue at opacity 0.8 (`#updated`, 13 px) = 5.11:1, at 0.9 (`#activity`) = 6.05:1, at 0.7
  (`state-unknown/unreachable`, large bold display text) = 4.27:1 >= 3:1 -> PASS.

### Memory, Bounds & Secrets
- No secret, identity or code is rendered by the new markup; the static text is unchanged.
  Icon files contain only vector paths / a flat raster; no EXIF or text chunks
  (`IHDR`/`IDAT`/`IEND` only) -> PASS.
- Owner archive: only path data of two owner vectors entered (star, wordmark); the PNG is
  reproducible from `icon.svg`; the GUI mockup with the owner's face is not in the tree
  -> PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or hook change in the delta; no font
  file, CDN or external URL added (system font stack) -> PASS.

### English-Only Policy
- New docs, test comments and assets scanned for non-ASCII French letters: none -> PASS.

### Documentation Accuracy
- `Docs/REMOTE_COMPANION.md` section 2e: palette values, token names, radii, the claim that the
  CSP, `app.js`, `sw.js`, ids, labels and routes are unchanged, and the "DANGER is below AA on
  pale" note (computed 4.22:1) are all correct. The icon regeneration command reproduces the
  committed bytes, but writes to the current directory (see finding 1).
- `AI/ARCHITECTURE.md` "Page design" row: accurate (seven assets present, RMC76-RMC88 exist in
  the matrix, RMC88 being the manual owner row).
- `AI/DECISIONS.md` new ADR, `AI/VERIFICATION_MATRIX.md` rows RMC76-RMC88, walkthrough 190
  (SHA-256 and 519-passed figures match what I measured), walkthrough 188 correction (the
  earlier "false positive only" statement was wrong; the scanner bug was a false negative):
  accurate.
- Sections of `Docs/REMOTE_COMPANION.md` outside 2e do not describe old colors; the added
  note about iOS caching the home-screen icon is correct guidance.

## 4. Detailed Findings & Action Items

- **[MINOR — RESOLVED in this fingerprint]** `Docs/REMOTE_COMPANION.md` section 2e (icon command) and the matching
  walkthrough 190 description: the command reads `crates/remote/assets/icon.svg` from the repository
  root but writes `png:apple-touch-icon.png`, which lands in the repository root, not in
  `crates/remote/assets/`. A maintainer following it would leave a stray file and not update the
  served icon. Correction: write to `png:crates/remote/assets/apple-touch-icon.png`.
  Resolution verified: the command now writes there and reproduces the committed bytes
  (SHA-256 `3442af18…cd46b3`). Walkthrough 190 only refers to the docs command, so it needs no
  change.

No open findings.

## 5. Final Verdict

No CRITICAL, MAJOR or open MINOR finding. CSP and server code unchanged, no archive raster or photo
committed, every UI state reachable, no test weakened, documentation accurate apart from the
MINOR output path above, which is now fixed and verified.

**VERDICT: APPROVED**
