# Walkthrough 256 — One-Command Setup PR A: Preflight Dependencies and the `remote.toml` Backup

- **Date**: 2026-10-23
- **Issue**: GitHub #362, owner decisions of 2026-10-23 (no backlog id) — **Branch**: `feat/setup-preflight-deps`
- **Matrix criteria**: SDP1–SDP25 (SDP26 is the owner's from-scratch check)
- **ADR**: "[2026-10-23] One Command Without Manual Steps ..., part 1" (amends ADR 2026-10-15
  "Setup User Phase" items (5) and (9) AS-U3)

## 1. Context & Objectives

The owner's from-scratch Ubuntu 24.04 run hit packages discovered missing in the middle of the
run and a setup that refused a pre-existing `remote.toml`. Owner decisions OD-T3 (back the file up
to `remote.toml.bak-<UTC>` and continue) and OD-T4 (detect every dependency in preflight) are PR A
of the spec's split; the Tailscale exposure (PR B) and the Android opt-in (PR C) follow. The
review follow-ups of PR #473 (push sender `--prefix` drop-in) are folded in.

## 2. Plan evaluation (round 3)

§19 of the spec was checked against `main` after #473 and against the repositories of the pinned
images (`AI/plan_evaluator_report.md`, round 3; spec §20). Main finding (F1): no JDK package
resolves on both `fedora:40` and the current `fedora:44` (only JDK 25 there, which Gradle 8.14.5
does not run on): the Fedora `android` group has no JDK and PR C decides with the owner. Other
fixes: the user file without identity stays "kept" (M2), an oversized user file is refused in
preflight instead of failing after the plan, the SCN lexer items are exposed with `pub(crate)`
for SDP3, PR A's wording for a dropped `[android]` table points to §2j. Verdict APPROVED.

## 3. Design

- **`scripts/check_build_deps.sh`**: groups `companion` (`jq`, DNS `dig`, `curl`,
  `ca-certificates`) and `android` (JDK where it resolves, `unzip`, udev rules for `adb`).
- **`REQUIRED_TOOLS`** (`setup.sh`, `declare -rA`, `[tool]=scope`): `base` and `family:<f>`
  checked with `command -v` before any change (exit 2); `companion`/`android` checked after the
  package step and before the build (`companion` non-fatal, R1-3); `toolchain` (`cargo`) checked
  after `install_rustup.sh` (exit 1); `escalate`, `root-start` keep their existing checks;
  `optional` call sites are guarded by `command -v`. `detect_packages` takes the groups to query
  (deduplicated across calls); the `companion` group is queried only when the companion runs.
- **User `remote.toml`** (`parse_user_remote_config`, `backup_user_config`): identity from the
  options, else the file (exactly one valid login, a valid node name); seven booleans read at top
  level with a tolerant, fail-safe rule; names (never values) of default-on keys, unrecognized
  keys and dropped keys/tables in the plan and the `[DONE]` line; the backup is created with
  `set -C` under `umask 077`, mode 0600, never through a link, byte-compared, never removed;
  journal entry `config_user_replaced` restores it on a companion failure.
- **`[android]`**: optional final table of the managed grammar (exact render), parsed into
  `FILE_ANDROID_FPS` and rendered last; a strict trailing user table (trailing comments allowed)
  is carried over; a binding with Funnel off is refused in preflight.
- **#473 follow-ups**: marker-only removal of `prefix.conf` in `install.sh` (`--prefix /usr`) and
  `uninstall.sh`; `push_sender_drop_in_current` in `system_up_to_date`; `--prefix` was already
  normalised by `absolute_dir` (GitHub #455), now pinned by SDP21.
- **Docker package matrix** `tests/docker/setup_packages_test.sh` (required CI job
  `setup-packages`): every name of every group in `ubuntu:24.04`, `fedora:40`, `fedora:44`,
  `archlinux` (digest-pinned), then every group tool after installing `companion` and
  `android`; the Fedora `java` gap is asserted, so closing it needs an explicit change.

## 4. Tests

- **Red** (commit `test(setup): add the preflight dependency contract (red)`): SDP1–SDP24 in
  `tests/invariants/src/setup_deps_contract.rs` (own harness cloned from the user-phase one, a
  fixed-stamp `date` stub for the collision cases, a no-op `pacman`/`install_rustup.sh` knob) and
  SDP25 in `crates/remote/tests/setup_render_tests.rs`; all 24 failed, SDP25 failed on the
  missing fixture.
- **Harness wiring** (category S, no assertion changed): M3 (the user-phase harness marks the
  `companion` group installed and has inert `jq`/`dig` stubs; `cksum` joins the setup_contract
  tools), and `pub(crate)` on the SCN lexer items.
- **Scanner fix** before green: SDP3 skips `case` pattern lists and trailing subshell redirections
  (which the SCN lexer reports as commands) and exempts the bounded `tailscale version`, because
  SCN3 forbids naming `tailscale` in any variable, the table included (synthetic cases added).
- **Green**: SDP1–SDP25. Owner-approved migrations M1/M2 of `test_set_u12_...` and the setup-only
  migration M17 of `test_pss16_...` are applied by the lead (they encode the old refusal and a
  fake-prefix install without the drop-in).

## 5. Verification

`cargo fmt --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D
warnings`, `cargo test --locked -p soos-invariants` and `-p soos-remote`, ShellCheck
(`koalaman/shellcheck:stable`, error severity as in CI), and `tests/docker/setup_packages_test.sh`
on the four images (every name resolves; every group tool found; Fedora `java` gap asserted).

## 6. Candid review round 1 (CHANGES_REQUESTED) and fixes

The lead applied M1/M2/M17 as `6209533`. Fixes on top, red first (`a25b60e`), then `dcbcc47`:

- **B1 (blocking)**: `"allow_funnel" = false` and `'allow_unlock' = false` are valid TOML that
  `soos-remote` accepts, but the user-file parser skipped quoted keys, so setup turned them on.
  Keys are now bare or quoted (unquoted before the match). Fail-safe: any top-level line that is
  not parsed but names a feature key turns that key off and names it, including the lines of a
  multi-line string (`"""`/`'''`) or array, which are tracked so that their `[` lines are no
  longer taken for table headers (that was the way to hide a later key). Keys below a real table
  header stay ignored, as spec R2-1 and SDP14 require (`soos-remote` refuses them there anyway).
  SDP27.
- **N1**: `install.sh` (`--prefix /usr`) and `uninstall.sh` read the marker on the first line of a
  regular file that is not a link, like `push_sender_drop_in_current`. SDP29.
- **N2**: SDP28 checks statically that every call of an `optional` tool is guarded by
  `command -v` (in its function, a predicate function it calls, or all of its callers).
- **N3**: commands run through `find -exec` or `xargs` are not seen by the SDP3 lexer check
  (`setup.sh` uses neither today); documented lexer limit.
- **N4**: `USER_UNRECOGNIZED` lists each key once, in `COMPANION_KEYS` order.

## 7. Residuals

- Fedora JDK for `--android`: owner decision in PR C (pinned JDK archive, third-party repository
  or Gradle upgrade).
- The `companion` tools are installed but unused until PR B.
- SDP26: the owner's from-scratch run on Ubuntu 24.04.
