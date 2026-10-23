# Candid Review Report

- **Date**: 2026-10-23
- **Target Branch**: `fix/clipboard-win-yanked` (commit `7fc6068`)
- **Base (merge-base)**: `fb2a0bf` (`origin/main`)
- **Reviewed-Diff-Fingerprint**: `1eb33a6c4d45fd84ab81067fd1700cc4c668e632754c22cbb0638c831e9488a4`
- **Audited Files**: `Cargo.lock`, `AI/walkthroughs/253_clipboard_win_yanked.md`

## 1. Executive Summary

The diff changes one `Cargo.lock` entry and adds a walkthrough:

- `Cargo.lock` `clipboard-win`: version `5.4.1` → `5.4.3`, checksum
  `bde03770…` → `052af7878466feccda12ba7a6a67d019d712f296809559b47c12c8cc196d3a9a`. The dependency
  list (`error-code`) is unchanged. No other lockfile entry, no `Cargo.toml` and no `deny.toml` change.
- `AI/walkthroughs/253_clipboard_win_yanked.md`: new walkthrough (number 252 is held by the open
  `chore/rac15-bench-exception` branch, so 253 is correct).

The yank fixes a real upstream memory-safety bug. 5.4.3 is the minimal, bounds-checked fix. No blocking
findings.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: none (only `Cargo.lock` and the walkthrough appear in `diff --git` headers).
- Removed or changed assertions: none.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): none.
- Inline test modules: none.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Scenario: the lockfile checksum does not match the published crate. The crates.io API for
  `clipboard-win/5.4.3` returns checksum `052af787…3a9a` (published 2026-10-23T05:43Z by `DoumanAsh`, the
  long-standing sole publisher, not yanked). `sha256sum` of the downloaded `.crate` gives the same value.
  PASS.
- Scenario: the bump drags other packages along. `git diff fb2a0bf..HEAD -- Cargo.lock` shows only the
  version and checksum lines of `clipboard-win`. PASS.
- Scenario: the crate is reachable from a shipped Linux artifact. `cargo tree --locked -i clipboard-win
  --target all` gives a single path `soos-gui` → `eframe` 0.35.0 → `egui-winit` 0.35.0 → `arboard` 3.6.1 →
  `clipboard-win` 5.4.3, Windows-only through `arboard`. `cargo check --locked -p soos-gui` (Linux)
  finishes cleanly. PASS.

### Yank Reason (upstream)
- Upstream issue DoumanAsh/clipboard-win#42 (2026-10-23): `raw::get_html` reads past the clipboard data
  when the HTML fragment offsets point beyond it. That is an out-of-bounds read in `unsafe`
  `ptr::copy_nonoverlapping`, so a crafted `HTML Format` clipboard payload can trigger it. The
  maintainer yanked 5.2.0 to 5.4.1 and published 5.4.2 and 5.4.3. 5.1.0 and earlier stay unyanked. soos
  never compiles the crate on its Linux targets, so it has no runtime exposure.

### Supply Chain (5.4.1 → 5.4.3 source diff)
- `src/raw.rs`: the unchecked `size > data_size` test and the `copy_nonoverlapping` + `set_len` copy are
  replaced by `data.get(start_idx..start_idx+size)` with an `invalid_data()` error on `None`. This is a
  strict safety improvement.
- `src/lib.rs`: the open-retry loop sleep changes from `Sleep(0)` to `Sleep(5)` (matching Chromium), the
  clippy attribute changes, and a doc string changes. `src/formats.rs` and `README.md` only change doc text.
  `tests/test_clip.rs` adds a regression test for the invalid fragment offsets.
- No `build.rs` (`build = false` in both versions), no new or changed dependencies, no network, process
  or filesystem code added. `.cargo_vcs_info.json` points at upstream commit `ed27008a`. PASS.
- Cooldown: 5.4.3 is less than one day old. This is acceptable because it is the maintainer's targeted
  security fix, and the alternative (5.4.2 or an ignore entry) is no better. PASS.

### PAM Concurrency & Deadlines
- Not applicable: no change under `crates/pam` or any runtime code. PASS.

### Panic Safety & Fail-Closed
- Not applicable: no production Rust code in the workspace changed. PASS.

### Test Integrity & Anti-Weakening
- No test touched. `deny.toml` keeps `yanked = "deny"` and `ignore = []`, so the gate is not loosened.
  `cargo deny check`: `advisories ok, bans ok, licenses ok, sources ok`. `./scripts/candid_review.sh`
  Layer 1: PASSED. PASS.

### Memory, Bounds & Secrets
- The upstream change removes an out-of-bounds read. No secrets or biometric data are involved. PASS.

### English-Only Policy
- Walkthrough and commit message are in English, and the commit follows Conventional Commits
  (`fix(deps): …`) with the co-author trailer. PASS.

## 4. Detailed Findings & Action Items

1. (Non-blocking, documentation) The walkthrough and commit message say "every `clipboard-win` release up
   to 5.4.1 was yanked". The yanked range is actually 5.2.0 to 5.4.1; 5.1.0, 5.0.0 and 4.x are not yanked.
   The walkthrough also does not give the yank reason (upstream #42, out-of-bounds read in
   `raw::get_html`). Optional follow-up wording fix; it does not affect the change.

## 5. Final Verdict

**VERDICT: APPROVED**
