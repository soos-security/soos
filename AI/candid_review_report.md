# Candid Review Report

- **Date**: 2026-10-21
- **Target Branch**: `test/setup-scanner-lexer`
- **Base (merge-base)**: `6492eb1`
- **Reviewed-Diff-Fingerprint**: `f426aadc491edaf66320a16d633a2080ceaebcb2a7c22223d83ea772b0bbd64c`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/251_setup_scanner_lexer.md`, `tests/invariants/src/setup_scanner_contract.rs`

## 1. Executive Summary

Test-only change (commit 2b57f7f: 5fef852, an amendment of 980b2ca after a first
CHANGES_REQUESTED review, rebased onto `origin/main` 6492eb1 after #468 merged). The static shell lexer behind SCN1-SCN5 now returns `Result` (`try_lex`, nesting
capped at 64), reads `(( ))` / `$(( ))` as arithmetic, follows `case ... esac` inside `$( )`,
keeps a command-position `[[ ... ]]` as one command, checks Tailscale words after quote removal
too, and adds SCN7 (command words built from variables must expand only script-assigned
variables). New tests SCN6-SCN12, matrix rows SCN6-SCN12, walkthrough 251. No production code,
PAM, IPC, daemon or `scripts/setup.sh` change. No CRITICAL or MAJOR finding remains.

## 2. Test Changes

- Rebase check (ecfefa4 to 6492eb1): the sorted `+`/`-` lines of the review diff are identical
  before and after the rebase; `setup_scanner_contract.rs` and walkthrough 251 are
  byte-identical to the reviewed 5fef852; the matrix diff is still the 7 added rows SCN6-SCN12
  (no line removed, RAC25 from #468 intact); no conflict marker. Only diff context moved.

- Test files touched: `tests/invariants/src/setup_scanner_contract.rs` only.
- SCN1-SCN5 and the `words` helper are byte-identical to `origin/main` (the region from the
  `// Tests` banner to the end of `test_scn5_traceability` diffs empty).
- Removed `assert!`/`assert_eq!` lines in the patch are all inside the lexer implementation
  (the old `list`, `heredoc_bodies` and `lex` internal panics), replaced by `Err` returns that
  `lex` turns back into a panic with the reason. No test assertion was removed or relaxed.
- New escape hatches (`#[ignore]`, skips, tolerances): **none**.
- Added tests: SCN6-SCN12, each with rejected and accepted synthetic inputs, then run on the
  real `setup.sh` where relevant.

## 3. Deep Reasoning Audit

### Logic & Architecture

Differential check, old lexer (`origin/main`) versus new (5fef852), on SCN2, SCN3, SCN4 and
`unit_assignments`, over about 50 adversarial inputs, with the bash semantics confirmed using
stub functions:

- First review's blocking input (`echo [[` then `run_bounded "${UNIT_START_TIMEOUT_S}"` then
  `systemctl start soos-daemon` on separate lines) at top level, in `$( )` and in backticks:
  SCN4 now rejects it, as on `origin/main`. Variations `then [[`, `! [[`, `{ [[ a ]]; }`,
  `time [[`, `[[ a ]];systemctl start`, `[[ a ]]&&`, `]]||`, `]]|`, `echo ]]`, `echo ]] [[`,
  `[[ $x == ]] ]]`, `echo "[["`, a `[[)` case pattern, `$( [[ $(systemctl start ...) ]] )` and
  nested `$( [[ $( [[ a ]] && tailscale up ) ]] )`: identical verdicts old versus new.
- Arithmetic: `(( $(tailscale up) ))`, `$(( $(tailscale) ))`, `` `(( $(tailscale up) ))` ``,
  `for (( ...; i<$(tailscale up); ... ))` are still rejected. `$((tailscale up) )`,
  `$((echo a);(tailscale up))` and `((a) ; tailscale up)` fall back to subshells as bash does.
  `echo $((tailscale up))` and `$(((echo a);(tailscale up)))` are no longer SCN2 findings, which
  is correct: bash reads both as arithmetic and runs nothing (verified).
- `case` inside `$( )`: three bodies the old lexer missed silently are now rejected.
- Quote-removal union in `tailscale_commands` only adds findings.
- SCN7: unquoted expansions no longer qualify for the fixed-program exception
  (`$TS/x run` and `X="$TS/bin/tool"; $X up` both run `tailscale` when
  `TS='tailscale serve '`). `eval`, `xargs`, `nice`, `setsid` and `stdbuf` are followed.

### PAM Concurrency & Deadlines

Not applicable (no PAM code).

### Panic Safety & Fail-Closed

`try_lex` never panics: recursion is bounded by `MAX_NESTING` through `nested()`, the cache of
start positions found not to be arithmetic prevents re-reading, and index reads are
bounds-checked. The new `Err` paths (`;` inside an open test, a test or `case` left open when
`$( )`, backticks or the script end) are bash syntax errors too. `setup.sh`, which has 326 lines
containing `[[`, lexes without error (SCN7 and SCN12 read it).

### Test Integrity & Anti-Weakening

Confirmed: no existing test changed, and no regression where the old scanners flagged an input
the new ones accept, except the arithmetic cases above that bash does not execute.

### Memory, Bounds & Secrets

Reads one repository file; no process, network or write; no secret handled.

### Supply Chain & Automation

No dependency or workflow change.

### English-Only Policy

All code, docs and the commit message are in English; the commit follows Conventional Commits
with the trailer.

## 4. Detailed Findings & Action Items

- RESOLVED (was MAJOR, 980b2ca): `[[` test mode on any word merged lines and let SCN4 miss an
  unbounded `systemctl start`; fixed, pinned by SCN11.
- RESOLVED (was MINOR): the SCN7 fixed-program exception could be abused by unquoted
  expansions; fixed, pinned by SCN12.
- MINOR (documented, not blocking): wrappers not followed by SCN7 (`bash -c`, `sh -c`,
  `su -c`, `runuser`, `flock`, `chroot`, `find -exec`); top-level `"${X}/bin/tailscale" up`
  remains a pre-existing gap of SCN7 and SET41. Both are recorded in walkthrough 251.

Gates: `cargo fmt --all -- --check` OK; `cargo clippy -p soos-invariants --all-targets -- -D
warnings` OK; `cargo test --locked -p soos-invariants -- --test-threads=3` 878 passed, 0 failed (re-run after the rebase).

## 5. Final Verdict

**VERDICT: APPROVED**
