# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `fix/setup-read-paths`
- **Base (merge-base)**: `4e0c02a`
- **Reviewed-Diff-Fingerprint**: `5067a0853030615150e9ab8f5352a5a8755e00e8dd0defd0ec9179623081767c`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/244_setup_read_paths.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `scripts/setup.sh`, `tests/invariants/src/setup_user_phase_contract.rs`

## 1. Executive Summary

The branch (GitHub #455) pins the rescue command of the `setup.sh` summary to the only kill switch
`pam_soos.so` and soos-daemon read (`/etc/soos/disabled`), replaces the trailing-slash stripping of
`absolute_dir` with a lexical normalisation (empty and `.` components dropped, `..` refused with
exit 2) applied to every path option including the internal `--artifact-dir`, and resolves a
post-build `TARGET_DIR` holding a `..` physically (`cd -P -- … && pwd -P`) so the root half never
receives a value its own parser refuses. Ten new invariant tests (SRP1–SRP10) are add-only. The
review found no CRITICAL, MAJOR or MINOR defect; two cosmetic test-comment staleness items are
recorded as SUGGESTION.

Validation run on the reviewed tree: `bash -n scripts/setup.sh` (OK) and
`cargo test --locked -p soos-invariants -- --test-threads=3` (842 passed, 0 failed, SRP1–SRP10
included).

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/setup_user_phase_contract.rs` only.
- Removed or changed `assert`/`#[test]`/`proptest!`/`#[should_panic]` lines: **none**. The only
  removed lines of the whole patch are four lines of the old `absolute_dir` body and the old
  `Rescue:` line of `scripts/setup.sh` (production code).
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.
- The test file hunk is a single `@@ -1839,3 +1839,466 @@` addition after the last existing test
  (`test_set_u24_root_preview_docker_job_wired_into_ci`); every SET*, LKI*, LWF*, PFP*, DCC*
  assertion is untouched. New helpers carry an `srp_` prefix (no collision).

No contract migration is needed.

## 3. Deep Reasoning Audit

### Logic & Architecture

- `absolute_dir` exercised directly with `/`, `//`, `/etc/`, `/etc//`, `//etc/./`, `/./`,
  `/a/b/.`, `/a/..b/c`, `/a/...`, `/a/*/c`, `/a/ b`, `/a/../b`, `/..`: results `/`, `/`, `/etc`,
  `/etc`, `/etc`, `/`, `/a/b`, `/a/..b/c`, `/a/...`, `/a/*/c`, `/a/ b`, exit 2, exit 2. Only an
  exact `..` component is refused; glob characters are never used as patterns (the `case` subject
  and expansions are quoted); the loop consumes at least one character per iteration. The
  relative-path and newline validation runs first and is unchanged. `usage_error` inside the
  command substitution exits the parent through `set -euo pipefail`, the same mechanism as the
  pre-existing absolute-path refusal (SRP5, SRP9 confirm exit 2 with no change). PASS.
- Every path option goes through the one
  `--prefix|--sysconfdir|--localstatedir|--runstatedir|--pam-dir|--artifact-dir` branch of the
  parser, so `..` is refused for all six, the internal `--artifact-dir` included (SRP9 runs the
  root half directly). PASS.
- Post-build resolution: the trigger `"/${TARGET_DIR}/" == */../*` matches only an exact `..`
  component (same rule as `absolute_dir`). `TARGET_DIR` is always absolute
  (`resolve_target_dir` prefixes `WORKSPACE_ROOT`, itself from `cd … && pwd`), so `CDPATH` cannot
  redirect `cd` or print to stdout. The `printf 'x'` sentinel keeps a trailing newline in the name,
  which the newline check then refuses. A failed `cd`, a non-absolute, newline-bearing or
  `..`-bearing result, or a missing `release/` prints `[FAIL] build` and exits `EXIT_BUILD` (40)
  before escalation. `TARGET_DIR` and `ARTIFACT_DIR` are both updated, so the later
  `install_remote.sh` call and the `--system-phase` argument use the same physical directory. When
  started as root, `run_system_phase` runs after this block, so it also sees the resolved value.
  PASS.
- Previously working `CARGO_TARGET_DIR` values: unset, absolute without `..`, relative without
  `..`, with trailing or repeated slashes or `.` components are all passed to the root half
  unchanged and normalised there to an equivalent lexical path; `install.sh` already resolves
  `--artifact-dir` with `cd … && pwd -P` (`scripts/install.sh` line 631), so the installed binaries
  are identical. Values with `..` (relative `../shared-target`, absolute `/x/repo/../t`, and `..`
  behind a link) still install (SRP7, SRP8, the link case asserting the kernel's directory and the
  absence of the lexical one). No other script or workflow invokes `setup.sh` with a path option
  (grep of `scripts`, `tests/docker`, `.github`, `packaging`). PASS.
- Ownership/permission checks: `check_target_dir_owner` (`find -H` before the build) is unchanged
  and inspects the lexical `TARGET_DIR`, which the kernel resolves to the same directory the
  post-build `cd -P` lands on. Nothing is skipped and no check is weakened; handing root a
  symlink-free physical path removes, rather than adds, a later link traversal. `install.sh`'s
  own artifact checks are unchanged. PASS.
- Rescue line: `KILL_SWITCH_READ_FLAG="/etc/soos/disabled"` matches `crates/pam/src/config.rs`
  `DEFAULT_FLAG_DIR` + `disabled` and `crates/daemon/src/presence/mod.rs`
  `DEFAULT_KILL_SWITCH_DIR` + `GLOBAL_DISABLE_FLAG` (pinned by SRP1, including the single
  `flag_dir` assignment and the `PresenceSwitch::new(PathBuf::from(DEFAULT_KILL_SWITCH_DIR))` call
  in `main.rs`). `KILL_SWITCH` stays for the state report only. Setup never writes either file
  (SRP2). PASS.
- Behaviour change: `--prefix` and the other options now refuse `..` where they previously
  accepted it. This is the documented decision of the ADR (item 3) and the help text says so.
  Accepted, not a finding.

### PAM Concurrency & Deadlines

No change under `crates/pam` or in any PAM path; the only Rust change is test code. N/A, PASS.

### Panic Safety & Fail-Closed

No production Rust changed. In `setup.sh`, every new failure path exits non-zero before any system
change (exit 2 at parse time, exit 40 before escalation); no path turns an error into success.
`unwrap`/`expect`/`panic!` appear only in the new test helpers. PASS.

### Test Integrity & Anti-Weakening

- Section 2 listing: add-only. PASS.
- Can the tests fail against plausible wrong implementations? SRP3 (third spelling) fails if `.`
  or inner `//` are kept; SRP5 fails if `realpath`/`readlink -f` is used (a link to the read
  directory would become "applied") or if `..` is accepted; SRP8 (link case) fails against a
  lexical `..` resolution; SRP7/SRP8 fail against the first fix (root half exit 2); SRP9 fails if
  the `..` refusal is dropped from `--artifact-dir`; SRP1/SRP2 fail if the rescue line uses
  `${SYSCONFDIR}`/`${KILL_SWITCH}`. PASS.
- `srp_point_read_paths_at_fake_root` asserts the presence of the `DAEMON_READ_CONFIG` and
  `LOCK_WIDGET_READ_FLAG` lines but replaces `PRESENCE_READ_FLAG` without asserting it; the line
  exists on this base (`scripts/setup.sh` line 553), so the substitution is effective today. See
  SUGGESTION 2.

### Memory, Bounds & Secrets

No secret, frame or embedding involved. The normalisation loop is bounded by the argument length;
error messages quote only the user-supplied path (with `%q` in the post-build error). PASS.

### Supply Chain & Automation

`scripts/setup.sh` changed: no new external command (`cd`, `pwd` are builtins), no `eval`, no
`realpath`/`readlink`, no network access, no new dependency, no workflow change. PASS.

### English-Only Policy

All code, comments, docs, matrix rows and the walkthrough are in English. PASS.

### Documentation Consistency

`Docs/PACKAGING_AND_PROVISIONING.md` ("Read paths (GitHub #455)"), `AI/DECISIONS.md`,
`AI/VERIFICATION_MATRIX.md` (SRP1–SRP10) and walkthrough 244 agree with the code: normalisation
rule, `..` refusal for every option including `--artifact-dir`, `cd -P`/`pwd -P` post-build
resolution, exit 40 (`EXIT_BUILD=40`), unchanged ownership preflight, rescue path. #454 is
described as merged (`4e0c02a`) everywhere; no text describes it as open. The walkthrough claim
that #454 did not touch `setup_user_phase_contract.rs` is consistent with #454's own message
(PFP1–PFP4 in `setup_contract.rs`). PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `tests/invariants/src/setup_user_phase_contract.rs:1846` — the section banner
  still says "rows SRP1-SRP6" while the section now holds SRP1–SRP10. Optional: update the comment
  in a later change (comment only; no assertion involved).
- **[SUGGESTION]** `tests/invariants/src/setup_user_phase_contract.rs:1890` — the comment "when this
  checkout already has it" and the unasserted `PRESENCE_READ_FLAG` replacement are stale now that
  #454 is on the base; asserting the line is present (as for the two other constants) would make
  SRP3's coverage of the presence comparison explicit. Optional, add-only hardening.

No CRITICAL, MAJOR or MINOR finding.

## 5. Final Verdict

**VERDICT: APPROVED**
