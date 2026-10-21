# Walkthrough 251 — Setup Scanner Lexer: Quoted Names, Inherited Variables, Arithmetic and `case`

- **Date**: 2026-10-21
- **Issue**: follow-up of the independent review of PR #460 (GitHub #459, part of epic #362,
  GitHub-only, no backlog id) — **Branch**: `test/setup-scanner-lexer`
- **Decision**: none new; the rules checked are those of ADR 2026-10-13 "One-Command Setup" (only
  `tailscale version` runs), as in walkthrough 245
- **Matrix criteria**: SCN6–SCN12 (new, verified). SCN1–SCN5, SET40, SET41 and every other row
  unchanged.
- **Number**: 250 is left free for the README capture change prepared in parallel.

## 1. Context & Objectives

The review of PR #460 (walkthrough 245) left three minor follow-ups on the shell lexer of
`tests/invariants/src/setup_scanner_contract.rs`:

1. **(a)** a `tailscale` word spelled through quotes or escapes could escape SCN2/SCN3:
   `'tail''scale'`, `"tail""scale"`, `t\ailscale`, `$'\x74ailscale'` (the bare `"tailscale"`
   and `\tailscale` forms were already caught, because the scanner only looks at the character
   before the name).
2. **(b)** a value inherited from the environment rather than assigned in the script was not
   considered (`"${TS}" serve` with `TS` never assigned).
3. **(c)** `(( x << 2 ))` was read as a here-document (the lexer panicked with "unterminated
   here-document 2"), and a `case ... esac` inside `$( ... )` ended the substitution at the first
   pattern label. The second form did not panic on the probes run here: it was a **silent miss**,
   `x="$(case "$1" in a) tailscale serve ;; esac)"` returned no finding.

`scripts/setup.sh` is unchanged: it passes every new check as it is.

## 2. Architect Design

- **Fail closed, never panic**: the lexer methods return `Result<usize, String>`; the new
  `try_lex` returns `Err` for unbalanced, truncated or too deeply nested input (more than
  `MAX_NESTING` = 64 levels, so `$(` repeated 100 000 times is refused instead of overflowing the
  stack) and never panics. `lex`, used by every check, keeps its signature and turns the `Err`
  into a test failure naming the reason, so an unreadable script still fails (SCN1's unbalanced
  inputs still panic through `lex`, unchanged).
- **Arithmetic**: `((` at a command position and `$((` are first read as arithmetic up to the
  matching `))`, following quotes and nested `$( )`, `${ }`, backticks (their commands are still
  listed). When the text is not arithmetic (`$((a) | b)`, `((c) && d)`), the commands read during
  the attempt are dropped and the text is read again as subshells, as before; a start index found
  not to be arithmetic is remembered so it is never tried twice (no exponential re-reading).
- **`case ... esac`**: each list tracks open `case ... in` (word 3 is `in`) and whether the next
  words are a pattern list (after `in`, `;;`, `;&`, `;;&`); in that state `)` ends the pattern
  and a leading `(` is skipped; a command-position `esac` closes the `case`. A substitution that
  ends with an open `case` is an `Err`.
- **`[[ ... ]]`**: a test opens only when `[[` is in command-word position (the first word,
  or preceded only by `if`, `elif`, `while`, `until`, `!`, `then`, `do`, `else`, `{`, `time`);
  `echo [[` is an argument and changes nothing. Inside an open test, `&&`, `||`, `(` and
  newlines no longer split it into commands, so a test operand is never mistaken for a command
  word by SCN7 (needed by (b)). A `;` inside an open test, or a test still open when a
  substitution, backticks or the script end, is an `Err` (fail closed). A first version entered
  test mode on any `[[` word, so `echo [[` merged the following lines into one command and SCN4
  missed an unbounded `systemctl start` after it; the review of commit 980b2ca found it, SCN11
  pins the fix.
- **(a) Quote removal**: `tailscale_commands` checks the text as written (unchanged) **and**
  after quote removal (`quote_removed`: quotes and backslashes dropped, `$'...'` escapes `\xHH`,
  `\NNN`, `\uHHHH`, `\UHHHHHHHH`, `\n`, `\t` decoded). The union only adds findings.
- **(b) Inherited variables**: what can be enforced statically is that **a command word built
  from a variable** (past `run_bounded <secs>`, `as_root`, `as_build_user`, `command`, `exec`,
  `builtin`, `eval`, `nohup`, `env`, `timeout`, `sudo`, `doas`, `xargs`, `nice`, `setsid`,
  `stdbuf` and compound keywords) only expands variables the script
  assigns from values it writes itself: assigned somewhere in the script, transitively for every
  variable its values expand, never from its own value (`TS="${TS:-x}"`), never through
  `${!NAME}`, `read`, `mapfile`, `readarray` or `printf -v`. A word or value that names a fixed
  program by its last path component (`"${PREFIX}/bin/soos-admin"`, `"${BIN_DIR}/soos-remote"`)
  is not followed further: a variable may choose the directory, not which program runs. The
  exception holds only when every expansion of the command word and of the value is
  double-quoted: an unquoted expansion is word split (`TS='tailscale serve '` makes `$TS/x run`
  run `tailscale`), so `$TS/x`, `${TS}/bin/x`, `X="$TS/bin/tool"; $X` and, conservatively,
  `X=$TS/bin/tool; "${X}"` are followed and rejected (SCN12). A positional parameter is a command word only as the `"$@"` pass-through of a wrapper.
- **(b) What stays out of static reach**, documented rather than invented:
  - a bare command name (`tailscale`, `systemctl`) resolves through `PATH`, which is always
    inherited; no static check over the script text can know which binary that is;
  - the value of an assigned variable that comes from a command's output
    (`PREFIX="${value}"` with `value="$(absolute_dir "$1" "$2")"`) depends on arguments and the
    file system at run time; the check follows the variables named inside the substitution, not
    its output;
  - wrappers other than the ones listed are not followed: `bash -c "$X"`, `sh -c`, `su -c`,
    `runuser`, `flock`, `chroot`, `find -exec` and the like take their command word where the
    check does not look; only the listed wrappers' options are skipped (with the value of `-n`,
    `-I`, `-P`, `-d`, `-a`, `-L`, `-s`, `-E`, `-u`, `-g`, `-k`), so an unusual option spelling
    can hide the command word;
  - the shell's own inherited state (`BASH_ENV`, exported functions, `SHELLOPTS`) acts before or
    beside the script text.
  The executed-call checks of the behavioral harness (`assert_no_forbidden_calls`) remain the
  backstop for those.

## 3. Tester Contract (Red)

New tests in `tests/invariants/src/setup_scanner_contract.rs` (pure additions; red run against
the unchanged lexer, with compile-only stubs for `try_lex` and `inherited_command_words`):

- SCN6 `test_scn6_tailscale_word_found_through_quotes_and_escapes` — 8 rejected and 4 accepted
  SCN2 snippets, 7 rejected and 2 accepted SCN3 snippets. Red: `"tail""scale" serve` in a
  substitution was accepted.
- SCN7 `test_scn7_command_word_variables_are_assigned_by_the_script` — rejected and accepted
  snippets, then `setup.sh` (no finding; `ADMIN_BIN`, `ENROLL_BIN`, `ESCALATE` seen). Red: the
  stub found nothing for `"${TS}" serve`.
- SCN8 `test_scn8_lexer_reads_arithmetic_and_case_in_substitution` — arithmetic words, subshell
  fallback, `case` inside `$( )` (labels, `;&`, `;;&`, nested), and SCN2 verdicts on such
  bodies. Red: "unterminated here-document 2".
- SCN9 `test_scn9_lexer_never_panics_and_fails_closed` — `Ok` without a panic for the (c)
  inputs, `Err` without a panic for 14 unbalanced inputs and four 100 000-level nestings, a clear
  `lex` message, and 4000 deterministic generated inputs. Red: `(( x << 2 ))` panicked.
- SCN10 `test_scn10_traceability` — matrix rows, this walkthrough, SCN1–SCN5 kept. Red: no row.
- SCN11 `test_scn11_test_mode_only_in_command_position` (after the review of 980b2ca) — SCN4
  still rejects an unbounded call after `echo [[` at top level, in `$( )`, in backticks, and
  after a multi-line real test; a real test with `!`, `&&`, `( || )` stays one command; open
  tests in `$( )`, backticks, at the end, or cut by `;` are an `Err`. Red: `echo [[` followed by
  `systemctl start soos-daemon` gave no SCN4 finding.
- SCN12 `test_scn12_command_word_quoting_and_more_wrappers` (after the review of 980b2ca) —
  unquoted expansions defeat the fixed-program exception; `eval`, `xargs`, `nice`, `setsid`,
  `stdbuf` are followed. Red: `$TS/x run` gave no finding.

While designing SCN7 against `setup.sh`, two of its own first rows were revised before the
implementation was accepted: `X="$(command -v tool)"` (a substitution output is not statically
known; replaced by a substitution expanding an unassigned variable) and `X="${HOME}/bin/tool"`
(names a fixed program; replaced by `X="${HOME}"`). No existing test or assertion was touched.

## 4. Auditor Constraints

1. No existing assertion or test changed: SCN1–SCN5, SET40, SET41 pass unchanged; the lexer
   implementation changed under them (allowed, every existing test still passes).
2. Every lexer loop advances at least one character or returns; recursion is bounded by
   `MAX_NESTING`; every index is checked against the input length before it is read.
3. The scanners only read a repository file; no process, no network, no write.
4. An unparseable script is an `Err`, and every check uses `lex`, which fails the test on it.

## 5. Developer Implementation

- `tests/invariants/src/setup_scanner_contract.rs`: `Result`-returning lexer, `try_lex`,
  arithmetic, `case`, `[[ ]]`, quote removal, SCN7 scanner, tests SCN6–SCN12.
- `AI/VERIFICATION_MATRIX.md` (component `setup-scanners`, SCN6–SCN12), this walkthrough.
- No production change, no `Docs/` change: no user-facing behaviour changes.

## 6. Verification

- `cargo test --locked -p soos-invariants -- --test-threads=3`: all green, SCN1–SCN12 included.
- `cargo fmt --all -- --check`, `cargo clippy -p soos-invariants --all-targets -- -D warnings`.

## 7. Follow-ups

- None planned. A name assembled from literal pieces (`a=tail; b=scale; "${a}${b}" serve`)
  still passes SCN7 (every piece is written by the script) and SCN2/SCN3 (no piece is a
  `tailscale` word); the behavioral harness remains the backstop for it.
- Known pre-existing gap, unchanged here: a literal program name behind a variable directory at
  top level (`"${X}/bin/tailscale" up`) passes SCN7 (it names a fixed program) and SET41 (which
  skips a name after `/`); SCN2 only catches it inside a substitution.
