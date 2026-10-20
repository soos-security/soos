# Candid Review Report

- **Date**: 2026-10-20
- **Target Branch**: `test/setup-scanners`
- **Base (merge-base)**: `6c4ecd4`
- **Reviewed-Diff-Fingerprint**: `1db57be7ab1e41fc18ba11b9608784e0fa366d8fc9c918e8bec3e6701f9b0510`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/245_setup_scanners.md`,
  `tests/invariants/src/lib.rs`, `tests/invariants/src/setup_scanner_contract.rs` (new),
  `tests/invariants/src/setup_user_phase_contract.rs`

Reviewer note: no separate sub-agent tool was available in this run, so the review was performed
in-session with the candid-reviewer procedure on the frozen patch only; a first pass found the
two MINOR issues listed in section 4, fixed before this fingerprint was taken.

## 1. Executive Summary

Tests-only change for GitHub #459: a new invariant module with a shell lexer that joins backslash
continuations and three new scanner checks (Tailscale by path or quoted name in a substitution,
Tailscale held in a variable, daemon unit bound with continuation lines) plus a traceability test.
No production code, no workflow, no dependency change. SET40 and SET41 are untouched. Every new
check has rejected and accepted synthetic inputs and passes on the real `scripts/setup.sh`.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/lib.rs` (module registration only),
  `tests/invariants/src/setup_scanner_contract.rs` (new), `tests/invariants/src/setup_user_phase_contract.rs`.
- Removed/changed lines with `assert`/`#[test]`/`proptest!`/`should_panic`: none.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance): none.
- Removed lines in the whole patch: two, both comments in `setup_user_phase_contract.rs`
  ("rows SRP1-SRP6" to "rows SRP1-SRP10"; the stale "when this checkout already has it" note on
  the `PRESENCE_READ_FLAG` rewrite). Comment-only; no code or assertion changed.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Tried: a call split between `systemctl` and its verb (`systemctl \` + `stop "${UNIT}"`) and
  between `run_bounded "${QUERY_TIMEOUT_S}" \` and `systemctl start` → rejected by SCN4 (snippets).
- Tried: `$(/usr/bin/tailscale serve)`, `$("tailscale" serve)`, `tail\` + newline + `scale` →
  rejected by SCN2.
- Tried: `TS=tailscale`, arrays, `+=`, `[i]=`, `alias`, `${X:-tailscale}`, `printf -v`, `read <<<`
  → rejected by SCN3; `${x#tailscale }`, `tailscale_version`, `--tailscale-login)` accepted.
- Tried: a lexer that silently loses context — unbalanced quotes, substitutions, backticks,
  subshells and here-documents panic (SCN1), and SCN2/SCN4 assert the lexer actually reaches the
  `tailscale version` substitution and counts at least as many bounded daemon calls as SET40's
  line count (6 today, matching `grep`: lines 1853, 1890, 2105, 2109, 2169, 2181).
- Unknown daemon verbs (`enable` without `--now`, `reload`, `kill`) fall into the "unexpected
  verb" branch and fail: conservative. PASS.

### PAM Concurrency & Deadlines
Not touched (no `crates/pam` change). PASS.

### Panic Safety & Fail-Closed
Test-only code; panics are the intended failure mode and the module carries the same
`#![allow(clippy::panic, ...)]` as its siblings. Every lexer loop advances or returns; EOF inside
a construct panics. PASS.

### Test Integrity & Anti-Weakening
No existing assertion changed (section 2). Each new check can fail: synthetic bad snippets are
asserted rejected. SCN5 also asserts SET40 and SET41 still exist. PASS.

### Memory, Bounds & Secrets
Reads one repository file; no I/O beyond it, no secrets. PASS.

### Supply Chain & Automation
No `Cargo.*`, `.github/`, `scripts/` or hook change; no new dependency. PASS.

### English-Only Policy
All code, comments, matrix rows and walkthrough in English. PASS.

## 4. Detailed Findings & Action Items

Fixed before this fingerprint (first pass):
- **[MINOR]** `AI/walkthroughs/245_setup_scanners.md` section 7 claimed the checks cover "every
  form the script can spell itself"; a name assembled from pieces or read from a here-document
  escapes. Rewritten to list the residual forms and name the behavioral backstop.
- **[MINOR]** `tests/invariants/src/setup_scanner_contract.rs` `unit_assignments` missed a bare
  `local UNIT` / `unset UNIT`; extended, with a self-test.

Open: none. SUGGESTION only: SCN4 knows the daemon unit only by `UNIT` and its literal names
(documented in the walkthrough).

## 5. Final Verdict

**VERDICT: APPROVED**
