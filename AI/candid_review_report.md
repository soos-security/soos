# Candid Review Report

- **Date**: 2026-10-23
- **Target Branch**: `feat/setup-preflight-deps`
- **Base (merge-base)**: `d5e3c09`
- **Reviewed-Diff-Fingerprint**: `443966868121a10ec6348439ee36d3240679fac85ae4d6bb7d7477c5e3fbe38e`
- **Audited Files**: `.github/workflows/ci.yml`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/architect_spec_one_command_setup.md`, `AI/auditor_constraints_setup_preflight_deps.md`,
  `AI/plan_evaluator_report.md`, `AI/walkthroughs/256_setup_preflight_deps.md`,
  `Docs/CI_CD_AND_SECURITY.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/REMOTE_COMPANION.md`,
  `README.md`, `crates/remote/tests/setup_render_tests.rs`, `scripts/check_build_deps.sh`,
  `scripts/install.sh`, `scripts/setup.sh`, `scripts/uninstall.sh`,
  `tests/docker/setup_packages_test.sh`, `tests/fixtures/setup/remote_android.toml`,
  `tests/invariants/src/lib.rs`, `tests/invariants/src/setup_contract.rs`,
  `tests/invariants/src/setup_deps_contract.rs`, `tests/invariants/src/setup_scanner_contract.rs`,
  `tests/invariants/src/setup_user_phase_contract.rs`

## 1. Executive Summary

PR A of the one-command setup (GitHub #362, owner decisions of 2026-10-23; spec §5, §6, §19,
§20): `companion` and `android` package groups in `check_build_deps.sh`, the scoped
`REQUIRED_TOOLS` table of `setup.sh` with a lexer-based coverage invariant (SDP3), a preflight of
every requested group, the `remote.toml.bak-<UTC>` backup of a user-written `remote.toml` with a
tolerant fail-safe parser, the trailing `[android]` table of the managed grammar, a
digest-pinned Docker package matrix job, and the #473 drop-in follow-ups.

Second review round. Round 1 (at `6209533`) raised one MAJOR finding: a TOML quoted key
(`"allow_funnel" = false`, `'allow_unlock' = false`) was silently skipped by the line parser, so
setup's default profile turned Funnel and remote unlock on against the user's written `false`.
It is fixed in `dcbcc47` (bare, double- and single-quoted keys; any unparsed top-level line that
names a feature key forces it off and lists it as not recognized; multi-line strings and arrays
tracked so a `[` line inside them is not a header) and pinned by SDP27. The two MINOR round-1
items are also fixed: the drop-in marker is checked on the first line only, on a regular
non-link file, in `install.sh` and `uninstall.sh` (SDP29), and the `optional` scope is enforced
by SDP28.

Rebind after `e14b604` (round 3). The developer closed the two MINOR fail-open residuals of
round 2 (escaped keys, and an escaped quote before `"""`; see §4). The reviewer proved each
SDP27 case red:

- `escapedkey` against the round-2 `setup.sh` of `5f7f315`;
- `escapedquotes` against the current `setup.sh` with only the `count_of` hunk removed.

Both are green on this tree. No test line was removed in this round.

Gates re-run on this tree: `cargo fmt --check`; workspace clippy (all targets, all features,
`-D warnings`); `cargo test -p soos-invariants -p soos-remote --test-threads=3` (invariants 923
passed, every suite green, 0 failed); shellcheck on every touched script (no error-severity
finding; `setup.sh`, `check_build_deps.sh` and the Docker test are fully clean, and the
`install.sh`/`uninstall.sh` warnings already exist on `origin/main`); and, in round 1,
`tests/docker/setup_packages_test.sh` (every name resolves on ubuntu:24.04, fedora:40, fedora:44
and arch, with the asserted Fedora `java` gap). The package lists did not change since. The
remaining findings are MINOR only.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Removed or changed pre-existing lines in `git diff origin/main...HEAD -- tests crates/*/tests`
(19 lines; everything else is an addition). Every line is in the owner-approved or acknowledged
set; no other pre-existing assertion changed. Rounds 2 and 3 (`a25b60e`..`defac7e`) only add
tests.

- `tests/invariants/src/setup_user_phase_contract.rs`
  `test_set_u12_template_replaced_user_file_untouched`: `let before = h.snapshot();`, the
  expected exit code `2` (now `0`) and `assert_eq!(before, h.snapshot());` are replaced by the
  owner-approved M1. M1 checks: exit 0, exactly one byte-equal 0600 `remote.toml.bak-<YYYYmmddTHHMMSSZ>`, managed file
  `[false,true,true,true,true,true,false]`, no `funnel --bg`. The `--yes` case becomes M2
  (identity from the file, backup, managed file, units enabled, no Tailscale change command). The
  former "kept" assertions are carried unchanged onto a new no-identity user file, plus an added
  "no backup" assertion.
- Same file, `Harness::new`: `for group in ["build", "models", "gui"]` gains `"companion"`, and
  inert `jq`/`dig` log-only stubs are added. This is harness wiring M3 (acknowledged): setup never
  runs either tool in PR A, so the stubs cannot make a test pass vacuously.
- Same file, `test_pss16_system_up_to_date_includes_the_push_sender`: setup-only fixture
  M17 (approved). It writes the `prefix.conf` drop-in that `install.sh` leaves for the fake
  non-`/usr` prefix. No assertion changes.
- `tests/invariants/src/setup_scanner_contract.rs`: `struct ShCmd` and its fields, `try_lex`,
  `lex`, `unquote`, `assignment_prefix`, `VarRef`, `variable_refs`, `names_fixed_program`,
  `Origin`, `script_assignments` and `command_word_index` become `pub(crate)`. This is a
  visibility change only (acknowledged, §20 F6), and every SCN test body is byte-identical.
- `tests/invariants/src/setup_contract.rs`: `cksum` is appended to `SAFE_TOOLS` (M3 wiring).
- `crates/remote/tests/setup_render_tests.rs`: the import line adds `parse_config_with_android`
  (for the new SDP25 test).

CI portability: the new harness labels user calls `uid=1000` (fixed role label). The real uid
only feeds the `id` stub and `SUDO_UID`, so CI runners with uid 1001 behave the same.

## 3. Deep Reasoning Audit

### Logic & Architecture
- **Preflight:**
  - `check_scope_tools base` and `family:<f>` run before any change (exit 2, `command -v` only).
  - Group tools are checked after the package step and before the build. A missing `companion` tool is non-fatal (R1-3), and in PR A no group with entries other than `companion` is ever requested.
  - `cargo` is checked after `install_rustup.sh` (exit 1).
- **SDP3 coverage, verified by mutation:** I injected `frobnicate --x`, `X="$(tr a b <<< c)"`,
  `… | sed -n p`, `run_bounded 5 udevadm settle` and `if ! awk …` into `setup.sh` one at a
  time. SDP3 failed for each and named the tool. The tree was restored byte for byte.
- **User file:** with an identity (options first, then the file) it is backed up and replaced; without one it is
  kept as before (M2, §20 F3). An oversized file is refused in preflight (§20 F4).
- **`[android]` binding:** an `[android]` binding while Funnel resolves off is refused in preflight. The managed grammar
  keeps a trailing strict `[android]` block and re-renders it last.
- **Push sender drop-in:** `push_sender_drop_in_current` reads only a regular non-link file. Its first-line rule now
  matches `install.sh` and `uninstall.sh`. With `/usr`, a marked stale file forces an install.sh
  run that removes it.

### PAM Concurrency & Deadlines
Not touched: no PAM, FFI or daemon code changes. Setup's bounded calls (`run_bounded`) are
unchanged.

### Panic Safety & Fail-Closed
- **Fail-safe parser.** I ran the extracted parser on adversarial inputs:
  - **Kept off, as required:** double- and single-quoted keys, tabs, CRLF, `false` then `true` duplicates (`false` wins), dotted keys, inline tables, a UTF-8 BOM, Unicode quotes, unterminated multi-line strings, `[` lines inside multi-line strings and arrays, `'''` literals, and comments naming a key. Every written `false` stayed `false`. Ambiguity only ever turns keys off.
- **Keys below a real table header are ignored** (spec R2-1, SDP14). This opens no path for a valid config: `FileConfig` and `FileAndroid` use `deny_unknown_fields`, and `[android]` is the only table, so soos-remote refuses any feature key under a header.
- **Round-3 adversarial run, all fail-safe:**
  - Quoted feature keys spelled with a short or long TOML unicode escape turn every feature key off.
  - An escaped quote followed by `""` keeps a multi-line basic string open.
  - An escaped backslash followed by `"""` closes the string, and the `[x]` line that follows is a real header, as in TOML.
  - A backslash inside a value (a `mailto:` subject, a Windows-style path, `"a=b"` with a backslash) changes nothing, because only text before the first `=` triggers the rule.
  - A comment line with a backslash is skipped before the rule applies.
  - Every round-2 input still keeps the written `false`.
- **No known residual input enables a feature the user wrote `false` for.**
- **Backup:**
  - **Creation:** `set -C` (O_EXCL) under `umask 077`; a name that exists or is a link (dangling included) is skipped; suffixes go up to `-9`, then the run fails with nothing replaced; the read is bounded by `head -c`; `cmp` runs before the replacement.
  - **Never deleted:** setup does not remove the backup.
  - **Rollback:** restores through a temporary file and rename under `set +e`.
  - **Privilege:** the companion phase runs as the session owner, so there is no cross-privilege race.

### Test Integrity & Anti-Weakening
See §2: only approved or acknowledged lines changed. SDP27 to SDP29 are additions, and their synthetic
self-checks reject and accept as stated. The SDP28 guard check is heuristic (a `command -v
<tool>` anywhere in the function or in all of its callers up to depth 3), which is enough for the
current call sites.

### Memory, Bounds & Secrets
- **Bounded reads:** the user file is read only through `head -c MAX_REMOTE_CONFIG_BYTES`, and nothing is sourced or evaluated.
- **No values printed:** plan and result lines print only key names, reduced to `[A-Za-z0-9_.-]` and capped at `MAX_REPORTED_KEYS`. SDP14 and SDP27 assert that no value is printed.
- **Login:** never printed, and never passed to the root half.

### Supply Chain & Automation
The `setup-packages` job:
- uses the pinned `actions/checkout` SHA with `persist-credentials: false`, under workflow-level `contents: read`;
- uses no secrets;
- runs digest-pinned images in unprivileged containers on a read-only mount;
- has a 60-minute ceiling and is required through `ci-success`.

`setup.sh` adds no `curl`, `wget`, repository or Tailscale command.

### English-Only Policy
All code, comments, docs, the walkthrough and commit messages are in English (checked on the
diff).

## 4. Detailed Findings & Action Items

1. **CLOSED (round 3), escaped quoted keys.** A quoted key that spells a feature key with a
   TOML escape sequence (valid TOML, the same key for soos-remote) now turns all seven feature
   keys off and names them. The rule is a backslash before the first `=` of a top-level line.
   Pinned by SDP27 `escapedkey`.
2. **CLOSED (round 3), an escaped quote before `"""` inside a multi-line basic string.**
   `count_of` drops the escaped backslash and escaped quote pairs before counting the delimiter,
   so the string stays open and its `[x]` line is not read as a header. Pinned by SDP27
   `escapedquotes`.
3. **CLOSED (round 3), `scripts/install.sh` `drop_in_first`:** now initialised at top level.
4. **INFO, SDP3 lexer:** it does not resolve `find -exec <cmd>` or `xargs <cmd>` targets. Neither
   is used in `setup.sh` today.

No CRITICAL or MAJOR finding remains.

## 5. Final Verdict
**VERDICT: APPROVED**
