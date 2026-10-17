# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `feat/lockscreen-gnome`
- **Base (merge-base)**: `9e82795`
- **Reviewed-Diff-Fingerprint**: `fcea5b8a017743569a71e0dd6034adc2518d764ebeca4ed2e3e65baa85c2c61d`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_lockscreen_gnome.md`,
  `AI/auditor_constraints_lockscreen_gnome.md`, `AI/tester_contract_lockscreen_gnome.md`,
  `AI/walkthroughs/215_lockscreen_gnome.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md`, `Docs/LOCKSCREEN_WIDGET.md`,
  `packaging/gnome-shell/soos-lockscreen@soos/extension.js`, `packaging/gnome-shell/soos-lockscreen@soos/metadata.json`,
  `packaging/gnome-shell/soos-lockscreen@soos/stylesheet.css`, `tests/invariants/src/lib.rs`,
  `tests/invariants/src/lockscreen_gnome_contract.rs`

## 1. Executive Summary

The change adds an opt-in GNOME Shell system extension (`soos-lockscreen@soos`, session modes `user`
and `unlock-dialog`) that, while the session is locked, spawns the root-owned `soos-lockstatus --watch`
with a fixed argv (no shell, no PATH lookup), parses its JSON lines with a strict three-key / three-state
validator and a bounded line buffer, and draws one non-reactive card. No Rust production code, PAM,
daemon, protocol or bridge file is touched. The extension has no unlock, input or IPC-write path. No
CRITICAL or MAJOR defect was found; two MINOR and one SUGGESTION item are recorded below.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/lib.rs` (one added `#[cfg(all(test, unix))] mod lockscreen_gnome_contract;`
  declaration with a doc comment, no other line changed) and `tests/invariants/src/lockscreen_gnome_contract.rs` (new file).
- Removed or changed assertions (`^-...assert|#[test]|proptest!|should_panic`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: none.
- `git diff $(git merge-base origin/main HEAD) HEAD -- '*tests*' 'tests/'` contains no removed line other than
  the `--- a/` / `--- /dev/null` headers: no pre-existing assertion was removed or changed.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Strict parsing: `parseLine` rejects non-objects, arrays, extra/missing keys (sorted key comparison),
  unknown states, non-boolean `available`, negative or non-safe-integer `sequence`. `available:false`
  is drawn as Idle via `present()`. PASS.
- Lifecycle: `_sync()` builds the card and starts the bridge only when `Main.sessionMode.isLocked` and the
  kill switch is absent; on unlock (`updated` with `isLocked` false) `_stopAll()` removes timers, kills the
  bridge and destroys the actor. Double exit notification (EOF and `wait_async`) is de-duplicated by the
  `bridge !== this._bridge` identity guard. PASS.
- Respawn: back-off 1/2/4/8 s capped, reset to 0 only on a valid line; respawn is not scheduled when the
  card is gone, and a missing root-owned binary logs one fixed warning without a respawn loop. PASS.
- Star loop: bounded by `PENDING_LOOP_MAX_MS` after the scan start and always rests on a half turn
  (at most one extra quarter step). `--watch` emits only on change (Docs/LOCKSCREEN_WIDGET.md section 6),
  so no heartbeat re-arms the loop. PASS.
- Sequence restart after a daemon restart can suppress one shake (see MINOR 2). Cosmetic only.

### PAM Concurrency & Deadlines
- `crates/pam` untouched; the extension never talks to PAM or the daemon socket directly, only via the
  existing read-only bridge. All I/O is async (`read_bytes_async`, `wait_async`) on the shell main loop,
  so a hung bridge cannot block gnome-shell. PASS.

### Panic Safety & Fail-Closed
- Every signal callback, GLib source callback, async callback and `enable()`/`_sync()` is wrapped in
  `try/catch`; failures fall back to `_stopAll()` / Idle. The extension has no path to an unlock, so a
  failure can only hide the card; the stock unlock dialog is never replaced or wrapped. PASS.

### Test Integrity & Anti-Weakening
- No existing assertion changed (section 2). New contracts LKG1-LKG8 are static checks that would fail on a
  plausible wrong implementation (e.g. adding `gdm` to session modes, a shell spawn, an input signal,
  an `unlock` reference, a missing `GLib.Source.remove`, a palette drift). `node --check` is skipped
  when node is absent and says so. Re-run locally: `cargo test -p soos-invariants lockscreen -- --test-threads=3`
  22 passed; `cargo clippy -p soos-invariants --all-targets -- -D warnings` clean; `cargo fmt --check` clean;
  `node --check` on an `.mjs` copy of `extension.js` OK. PASS.

### Memory, Bounds & Secrets
- Reads are 256-byte chunks; `_consume` fails the bridge on any line over 256 bytes and on a pending
  remainder over 256 bytes, so the buffer never exceeds 512 bytes. UTF-8 is decoded per complete line with
  a fatal decoder (errors caught and ignored). Logs are fixed `console.warn` strings only, never a line,
  state or exception text. No frames, embeddings or credentials are reachable. PASS.
- Binary trust: candidates restricted to `/usr/bin` and `/usr/local/bin`, root-owned, no group/other write,
  executable; absolute argv, `STDERR_SILENCE`, `force_exit` on stop; GSubprocess reaps the child. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or hook change. No new dependency. PASS.

### English-Only Policy
- Diff scanned for non-English text and accented characters: none. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `tests/invariants/src/lockscreen_gnome_contract.rs:453` — the change-set guard fails any
  future branch that touches both `packaging/gnome-shell/` and `crates/daemon|pam|protocol|lockstatus`
  (for instance a later installer opt-in or a joint fix). It is correct for this PR, but a future
  legitimate combined change will have to be split or the contract migrated through a backlog line.
  No action required now; record it for the KDE/installer PRs.
- **[MINOR]** `packaging/gnome-shell/soos-lockscreen@soos/extension.js:371` — `sequence` restarts with
  the daemon, so a Not recognized after a daemon restart that reuses an already-shaken sequence number
  does not shake the card. Cosmetic only (label and ring colour are still correct).
- **[SUGGESTION]** `packaging/gnome-shell/soos-lockscreen@soos/extension.js:246` — the kill switch is read
  only on session-mode transitions; this matches the documented "applies from the next lock" behaviour.

## 5. Final Verdict

**VERDICT: APPROVED**
