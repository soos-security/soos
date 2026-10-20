# Walkthrough 245 — Setup Scanners: Continuation Lines, Tailscale by Path and in Variables

- **Date**: 2026-10-20
- **Issue**: GitHub #459, part of epic #362 (GitHub-only, no backlog id), follow-up of the reviews of PR #440 (GitHub #437) and PR #456 (GitHub #455) — **Branch**: `test/setup-scanners`
- **Decision**: none new; the rules checked are those of ADR 2026-10-13 "One-Command Setup" (only `tailscale version` runs) and of GitHub #437 (daemon unit calls bounded by `UNIT_START_TIMEOUT_S`)
- **Matrix criteria**: SCN1–SCN5 (new, verified). SET40, SET41 and every other row unchanged.

## 1. Context & Objectives

Two static contracts of `scripts/setup.sh` had known blind spots, reported by the reviews of
PR #440 and PR #456. `setup.sh` itself was already correct; the goal is that a later edit cannot
slip through.

1. **SET41** (`test_set_s7_no_tailscale_command_inside_command_substitution`): its scanner takes
   `tailscale` as a command word only when the character before it is not a word character
   (`/` is one) and the character after it is a blank. `"$(/usr/bin/tailscale serve)"` and
   `$("tailscale" serve)` therefore passed. A Tailscale command held in a variable
   (`TS=tailscale`, `TS="$(command -v tailscale)"`, `CMD=(tailscale serve)`,
   `"${TS:-tailscale}" serve`) passed both SET3 (which needs the literal `tailscale ` outside
   quotes) and SET41.
2. **SET40** (`test_set_s6_daemon_unit_calls_bounded_by_unit_start_timeout`): checked line by
   line. A call split between `systemctl` and its verb or unit
   (`systemctl \` + newline + `start "${UNIT}"`) was neither counted nor checked.
3. Cosmetic, in `tests/invariants/src/setup_user_phase_contract.rs`: the section banner said
   "rows SRP1-SRP6" although the section carries SRP1-SRP10, and the comment above the
   `PRESENCE_READ_FLAG` rewrite said "when this checkout already has it" although #454 is merged.

## 2. Architect Design

- Additive only: SET40 and SET41 stay byte-identical; the new checks live in a new module,
  `tests/invariants/src/setup_scanner_contract.rs`, registered in `tests/invariants/src/lib.rs`.
- One shell lexer (`lex`) returns every simple command as raw words with its line and whether it
  runs inside a substitution. It follows single quotes, double quotes, `$'...'`, `${...}`,
  `$(...)`, backticks, `<(...)`/`>(...)`, here-strings and here-documents (unquoted bodies have
  their substitutions lexed, quoted bodies are text), `NAME=(...)` arrays as one word, comments,
  subshells, `[[ =~ ]]` groups and `case` labels. A backslash continuation is removed wherever it
  appears, so a split command is one command and a split word is one word. A script that does not
  end balanced panics, so an unreadable script fails instead of passing.
- A `tailscale` command word is the name bare, quoted or as the last path component, never part
  of a longer name (`tailscale_version`, `--tailscale-login`, `tailscale.com`) or a directory
  (`tailscale/`). It is allowed only when followed, past quotes and at least one blank, by
  `version`.
- Checks: SCN2, no such word other than `tailscale version` in a substitution command; SCN3, no
  such word in an assignment value (outside `${...}` patterns), a parameter fallback operand
  (`:-`, `-`, `:=`, `=`, `:+`, `+`), a `printf -v` or a `read`/`mapfile`/`readarray` command;
  SCN4, every daemon unit (`${UNIT}`, `$UNIT`, `soos-daemon`, `soos-daemon.service`) start,
  stop or restart by bare or absolute `systemctl` (`enable`/`disable --now` included) has
  `run_bounded "${UNIT_START_TIMEOUT_S}"` as the two words before `systemctl`, read-only verbs
  have `QUERY_TIMEOUT_S`, a verb in `$1` is only the `unit_query` helper and `unit_query` only
  receives read-only verbs, `UNIT_START_TIMEOUT_S` never bounds `systemctl --user`, and `UNIT`
  is assigned once.

## 3. Tester Contract (Red)

New file `tests/invariants/src/setup_scanner_contract.rs`:

- SCN1 `test_scn1_lexer_joins_continuations_and_follows_contexts` — words of continued commands,
  a word split by a continuation, quoted messages, nested substitutions, process substitution,
  here-documents (quoted and unquoted, line numbers after them), a `[[ =~ ]]` group, an array,
  and five unbalanced inputs that must panic.
- SCN2 `test_scn2_no_tailscale_in_substitution_by_path_or_quoted_name` — 10 rejected and 9
  accepted snippets, then `setup.sh` (the lexer must see its `tailscale version` substitution).
- SCN3 `test_scn3_no_tailscale_command_held_in_a_variable` — 15 rejected and 8 accepted snippets,
  then `setup.sh`.
- SCN4 `test_scn4_daemon_unit_bound_survives_continuation_lines` — 13 rejected and 7 accepted
  snippets plus the `UNIT` assignment counter, then `setup.sh`: no violation, `UNIT` assigned
  once to `soos-daemon.service`, and at least 6 bounded calls and at least as many as SET40's
  own line count (the lexer sees 6 today, lines 1853, 1890, 2105, 2109, 2169, 2181).
- SCN5 `test_scn5_traceability` — matrix rows, this walkthrough, SET40 and SET41 still present.

Red run: SCN5 failed until the matrix rows and this walkthrough existed; the first SCN3 run also
rejected the accepted snippet `v=${x#tailscale }`, which led to excluding `${...}` patterns from
assignment values (fallback operands keep their own check). The scanner self-tests are the proof
that each gap is closed: every rejected snippet above is a form SET40 or SET41 let through or a
neighbour of one.

## 4. Auditor Constraints

1. No existing assertion is changed: SET40, SET41 and their snippets are untouched; the only
   edit outside the new module is two comments in `setup_user_phase_contract.rs` and the module
   registration.
2. The lexer only reads a repository file; no process, no network, no write.
3. Every loop of the lexer advances at least one character per iteration or returns.
4. Unbalanced input fails loudly (asserted by SCN1).

## 5. Developer Implementation

- `tests/invariants/src/setup_scanner_contract.rs` (new), `tests/invariants/src/lib.rs`
  (registration), `tests/invariants/src/setup_user_phase_contract.rs` (two comments),
  `AI/VERIFICATION_MATRIX.md` (component `setup-scanners`, SCN1–SCN5), this walkthrough.
- No production change: `scripts/setup.sh` passes every new check as it is. No `Docs/` change:
  no user-facing behaviour changes.

## 6. Verification

- `cargo test --locked -p soos-invariants -- --test-threads=3`: all green, SCN1–SCN5 included.
- `cargo fmt --all -- --check`, `cargo clippy -p soos-invariants --all-targets -- -D warnings`.

## 7. Follow-ups

- None planned. A static check cannot follow every data flow; forms still outside SET3, SET41
  and SCN2–SCN3 include a Tailscale command reached through an environment variable `setup.sh`
  never assigns (`"${TS}" serve` with `TS` inherited), a name assembled from pieces
  (`a=tail; b=scale; "${a}${b}" serve`), and a value read from a file or a here-document body.
  SCN4 likewise only knows the daemon unit by `UNIT` and its literal names. The executed-call
  checks of the behavioral harness (`assert_no_forbidden_calls`) remain the backstop.
