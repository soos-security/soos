# Candid Review Report

- **Date**: 2026-10-20
- **Target Branch**: `test/setup-scanners`
- **Base (merge-base)**: `6c4ecd4` (equal to `origin/main` after `git fetch origin`; no rebase needed)
- **Reviewed-Diff-Fingerprint**: `1db57be7ab1e41fc18ba11b9608784e0fa366d8fc9c918e8bec3e6701f9b0510`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/245_setup_scanners.md`,
  `tests/invariants/src/lib.rs`, `tests/invariants/src/setup_scanner_contract.rs` (new),
  `tests/invariants/src/setup_user_phase_contract.rs`

Reviewer note: this is an **independent review** by a separate reviewer agent with no prior
context of the change. It **replaces the author's in-session self-review** previously recorded
in this file. Only the frozen patch (`target/candid_diff.patch`) and the code it lands in were
trusted; the walkthrough and commit message were treated as claims to verify.

## 1. Executive Summary

The change is test-only and additive: a new module `setup_scanner_contract.rs` with a small
shell lexer (quotes, `$'...'`, `${...}`, `$(...)`, backticks, process substitution, here-strings,
here-documents, arrays, comments, subshells, continuation lines) and five checks SCN1-SCN5 that
close the documented SET40 (continuation lines) and SET41 (absolute path / quoted name / variable)
gaps for `scripts/setup.sh`. The existing contract file receives two comment edits only; no
assertion anywhere is added to, removed from or changed in pre-existing tests. The new checks
genuinely run on the real `scripts/setup.sh` and were shown to bite on it by mutation (section 3).
No production code changes. No CRITICAL or MAJOR finding; three MINOR items and one SUGGESTION
are recorded as known limits of a static scanner.

## 2. Test Changes

Mechanical listing from the frozen patch:

- Test files touched: `tests/invariants/src/lib.rs` (module registration, `#[cfg(all(test, unix))]`),
  `tests/invariants/src/setup_scanner_contract.rs` (new), `tests/invariants/src/setup_user_phase_contract.rs`.
- Removed/changed assertion lines (`^-` with `assert`/`#[test]`/`proptest!`/`should_panic`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance/epsilon): **none**.
- Inline `mod tests` changes: **none**.
- `setup_user_phase_contract.rs` hunks: (1) banner comment `rows SRP1-SRP6` -> `rows SRP1-SRP10`
  (verified accurate: SRP1-SRP10 exist in that file and in the matrix); (2) comment above the
  `PRESENCE_READ_FLAG` rewrite updated to say #454 is merged (accurate: `4e0c02a` on main). Both
  are comment-only; the surrounding code is byte-identical.
- `setup_contract.rs` (SET40 / SET41) is not in the patch: byte-identical.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Ran the full suite: `cargo test --locked -p soos-invariants -- --test-threads=3` -> 847 passed,
  0 failed (SCN1-SCN5 included). `cargo clippy --locked -p soos-invariants --all-targets -- -D warnings`
  clean; `cargo fmt --all -- --check` clean.
- **Mutation on the real script** (throwaway detached worktree, removed afterwards):
  - replaced line 2169 with `run_bounded "${UNIT_START_TIMEOUT_S}" true; systemctl \` + newline +
    `stop "${UNIT}"` -> SCN4 failed with `line 2170: systemctl stop of the daemon unit is not
    bounded by UNIT_START_TIMEOUT_S`.
  - replaced the `tailscale version` substitution with `/usr/bin/tailscale serve status` -> SCN2
    failed (`line 804`) while **SET41 still passed**, which confirms the gap is real and closed.
  - added `TS_BIN="tailscale"` -> SCN3 failed (`line 805`).
  The checks therefore read and evaluate `scripts/setup.sh`, not only synthetic snippets. SCN2
  also asserts the lexer reaches the `tailscale version` substitution, and SCN4 asserts at least
  six bounded calls and at least SET40's own line count, so a lexer that silently skipped the
  script would fail. Verified the six bounded calls exist at lines 1853, 1890, 2105, 2109, 2169,
  2181 as the walkthrough states.
- Lexer false-pass analysis (could a real bad form slip through?):
  - Continuation inside a word (`tail\` + newline + `scale`): joined, caught (SCN1/SCN2/SCN3 cases).
  - Quoted name / absolute path / backticks / `<(...)` / unquoted here-document body: caught.
  - Verb after options (`systemctl --no-block start`), `enable --now`, absolute `/usr/bin/systemctl`,
    a `$verb` variable, a mismatched bound, a bound separated by `;`: all rejected in SCN4 snippets.
  - An option value taken as the verb (e.g. `-t service start`) yields an "unexpected systemctl"
    violation: fails closed.
  - Residual forms that still pass are listed as MINOR findings 1-2; they are obfuscations rather
    than plausible accidental regressions, and the walkthrough documents most of them honestly.
- Lexer panic analysis (could it panic on valid bash?): every unbalanced or unknown construct ends
  in an `assert!`/`panic!` with a line number, i.e. fails loudly, never passes silently. Valid
  bash that would trip it: an arithmetic left shift `(( x << 2 ))` / `$(( x << 2 ))` (read as a
  here-document with delimiter `2`), a `<< \EOF` delimiter, and a `case ... in pat) ...` inside a
  `$(...)` (the `)` closes the substitution). None occur in `setup.sh` today (MINOR finding 3).
  Every loop advances at least one character or returns; indexing is bounded by `i < len` checks
  or `get`.
- Traceability: matrix rows SCN1-SCN5 present with correct test paths; walkthrough snippet counts
  verified (SCN2 10/9, SCN3 15/8, SCN4 13/7 rejected/accepted); SCN5 asserts SET40/SET41 still exist.
  PASS.

### PAM Concurrency & Deadlines

No change to `crates/pam` or any runtime code. Not applicable. PASS.

### Panic Safety & Fail-Closed

Panics are confined to `#[cfg(test)]` contract code with a scoped `#![allow(...)]` carrying a
`reason`, consistent with other contract modules. The lexer's panics are the intended fail-loud
behaviour (asserted by SCN1's five unbalanced inputs under `catch_unwind`). PASS.

### Test Integrity & Anti-Weakening

Section 2: zero pre-existing assertions changed; the new tests fail against plausible wrong
implementations (demonstrated by the mutations above and by each scanner's rejected-snippet
self-test). PASS.

### Memory, Bounds & Secrets

Reads one repository file; no process, network or write. No secrets. PASS.

### Supply Chain & Automation

No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. PASS.

### English-Only Policy

All code, comments, matrix rows and the walkthrough are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `tests/invariants/src/setup_scanner_contract.rs` (`tailscale_tokens` / `tailscale_commands`)
  — the token search runs on raw words, so a name split by quotes or a backslash inside a word
  (`$(tail"scale" serve)`, `$(tailscal\e serve)`) still passes SCN2/SCN3. Optional hardening:
  also search `unquote(word)` per word. Obfuscation only; not blocking.
- **[MINOR]** same file, SCN3/SCN4 — a Tailscale command or daemon unit reached through a variable
  the script never assigns (inherited environment) or a differently named variable holding
  `soos-daemon` is not detected. Documented in walkthrough 245 section 7; the behavioural harness
  remains the backstop. Not blocking.
- **[MINOR]** same file, `Lexer::list` — valid bash such as `(( x << 2 ))`, `<< \EOF` or a `case`
  inside `$(...)` would make the lexer panic (fail loudly, never pass). Absent from `setup.sh`
  today; a future edit introducing one would get a clear panic message pointing at the line.
- **[SUGGESTION]** SCN5 checks SET40/SET41 by function name only; that suffices for "kept" but not
  for "byte-identical". Acceptable as is.

## 5. Final Verdict

**VERDICT: APPROVED**
