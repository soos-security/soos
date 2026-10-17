# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `feat/lockscreen-gnome`
- **Base (merge-base)**: `f1e7616`
- **Reviewed-Diff-Fingerprint**: `eded60160fa45ac3e2476d4c873e837cde78b4cc8b231831eaf2ea21dfd8f464`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_lockscreen_gnome.md`,
  `AI/auditor_constraints_lockscreen_gnome.md`, `AI/tester_contract_lockscreen_gnome.md`,
  `AI/walkthroughs/215_lockscreen_gnome.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md`, `Docs/LOCKSCREEN_WIDGET.md`,
  `packaging/gnome-shell/soos-lockscreen@soos/extension.js`, `packaging/gnome-shell/soos-lockscreen@soos/metadata.json`,
  `packaging/gnome-shell/soos-lockscreen@soos/stylesheet.css`, `tests/invariants/src/lib.rs`,
  `tests/invariants/src/lockscreen_gnome_contract.rs`

## 1. Executive Summary

Independent re-review of PR #407 (GNOME Shell unlock-dialog extension, GitHub #405) after its rebase
from `9e82795` onto `origin/main` `f1e7616` (which gained #411, #413, #417 zbus 6, #402 setup user
phase and #419 Android heartbeat). The merge base equals the current `origin/main` tip.

Content proof, per file, comparing the added/removed lines of the previously reviewed branch
(`9e82795..origin/feat/lockscreen-gnome`) with the rebased branch (`f1e7616..HEAD`):

- Identical: `extension.js`, `metadata.json`, `stylesheet.css`, `lockscreen_gnome_contract.rs`,
  `tests/invariants/src/lib.rs`, the spec, tester contract, auditor constraints, walkthrough 215,
  `Docs/LOCKSCREEN_WIDGET.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md` and `AI/DECISIONS.md`.
- `AI/VERIFICATION_MATRIX.md`: the only difference is that the `lockscreen-gnome` section is now
  closed by the file's usual `---` separator (two blank lines replaced by `---` plus one blank line).
- `AI/plan_evaluator_report.md` (excluded from the fingerprint): byte-identical to the reviewed
  branch version.

No code, contract or documentation content changed with the rebase. No defect found.

## 2. Test Changes

Mechanical listing on `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/lib.rs` (one new `#[cfg(all(test, unix))] mod
  lockscreen_gnome_contract;` registration with its doc comment) and the new
  `tests/invariants/src/lockscreen_gnome_contract.rs`.
- Removed or changed assertions / `#[test]`: none. The only removed lines in the whole patch are
  seven documentation lines of `Docs/LOCKSCREEN_WIDGET.md` (the "GNOME later" wording replaced by
  the shipped adapter).
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.

No pre-existing assertion changed.

## 3. Deep Reasoning Audit

### Logic & Architecture
- `AI/DECISIONS.md` against `origin/main`: `diff` shows exactly one added line, the
  "[2026-10-17] GNOME Shell Lock-Screen Extension" ADR, placed after main's #400 entry. The
  "[2026-10-15] Lock-Screen Adapters" entry appears once (line 144), in main's current post-#413
  wording; the other "Lock-Screen Adapters" hits are references inside later entries. Main's
  #410 (LockState refusal clarification), #412 (lazy second shm buffer) and #400 (setup user
  phase) entries are each present once, and #415 appears once, inside main's amended native
  Android ADR. A duplicate-heading scan (`uniq -d` over all ADR headings) returns nothing. PASS.
- Matrix section `lockscreen-gnome` is preceded and followed by `---`. The closing `---` directly
  follows the last table row; a thematic break ends a GFM table, so rendering is correct. PASS.
- Semantic interaction with main: zbus 6 (#417) is irrelevant (the extension is GJS and only
  spawns `soos-lockstatus --watch`). The `--watch` JSON line contract (`state`, `available`,
  `sequence`) and `PENDING_LOOP_MAX_MS` mirrored by LKG8 still match main's `soos-lockstatus`;
  #413 only changed lock-widget buffer allocation. The LKG7 change-set guard computes
  `merge-base HEAD origin/main` and requires no change under `crates/pam`, `crates/daemon`,
  `crates/protocol`, `crates/lockstatus`; the patch touches only `AI/`, `Docs/`, `packaging/` and
  `tests/invariants/`, and the test passes locally. PASS.
- Extension logic re-read: card built only while `Main.sessionMode.isLocked` and the kill switch
  is absent; three states, `available:false` drawn as Idle; strict three-key parse with a safe
  integer sequence; respawn back-off bounded at 8 s; stale bridge callbacks ignored by an identity
  check. PASS.

### PAM Concurrency & Deadlines
- No change in `crates/pam`. The extension runs in gnome-shell, outside the PAM stack, and cannot
  stall `login`/`sudo`/`gdm` (`session-modes` excludes `gdm`). PASS.

### Panic Safety & Fail-Closed
- Every signal, timeout and async callback is wrapped in `try`; any failure tears the card down
  (`_stopAll`) or draws Idle. No unlock path (`screenShield`, `AuthPrompt`, `LoginManager`) and no
  input signal; card actors are non-reactive and unfocusable. A broken extension leaves the stock
  unlock dialog intact. PASS.

### Test Integrity & Anti-Weakening
- Contract file identical to the reviewed version; all 746 `soos-invariants` tests pass on the
  rebased tree, including LKG1-LKG8. No assertion removed or relaxed. PASS.

### Memory, Bounds & Secrets
- stdout read in 256-byte chunks; the pending buffer is bounded by `MAX_LINE_BYTES +
  READ_CHUNK_BYTES`; an over-long line stops the bridge. Only fixed `console.warn` strings, never
  line content or exception text. The binary is accepted only from `/usr/bin` or `/usr/local/bin`
  when root-owned, not group/other-writable and executable. No biometric data crosses this path.
  PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. PASS.

### English-Only Policy
- All added text is English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `AI/VERIFICATION_MATRIX.md` (end of the `lockscreen-gnome` table) — a blank line
  between the LKG11 row and the closing `---` would match the rest of the file. Rendering is
  already correct; no action required for merge.

## 5. Final Verdict

**VERDICT: APPROVED**
