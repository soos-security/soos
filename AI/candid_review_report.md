# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `fix/setup-read-paths` (PR #456, sub-issue #455, rebased onto `origin/main`)
- **Base (merge-base)**: `4e0c02a`
- **Reviewed-Diff-Fingerprint**: `fd3f8ee044b9d4771e4799f244bc173abc0b0a93681d7ef62a32b8af4e5b3c91`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/244_setup_read_paths.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `scripts/setup.sh`, `tests/invariants/src/setup_user_phase_contract.rs`

## 1. Executive Summary

This review covers the branch after its rebase onto `origin/main` `4e0c02a`, which added #454 (`PRESENCE_READ_FLAG`). `git range-diff` reports the fix commit as identical (`6503f5b = d9baeeb`). The changed lines of the diff against the old base `8f03eb7` and against the new base `4e0c02a` are byte-identical once the report is left out. The rebase therefore changed nothing in the branch's own work.

The fix is correct for what it targets:
- `KILL_SWITCH_READ_FLAG="/etc/soos/disabled"` matches `pam_soos.so` and soos-daemon, and the single `Rescue:` line names it.
- `absolute_dir` normalises `//etc/./`, `/etc/` and `/etc//` to `/etc`. Since `PRESENCE_FLAG`, `LOCK_WIDGET_FLAG`, `DAEMON_TOML` and `KILL_SWITCH` all derive from the normalised `SYSCONFDIR`, #454's presence comparison now composes with it.

The rebase exposed one defect that the `..` refusal brings with it. The internal `--artifact-dir` goes through the same `absolute_dir`. Its value is derived from `CARGO_TARGET_DIR`, and a relative `CARGO_TARGET_DIR` is a documented contract, resolved against the checkout. So a value such as `CARGO_TARGET_DIR=../shared-target` now fails the root half with exit 2 after the full release build has run (MAJOR, below).

`bash -n` passes, and `cargo test -p soos-invariants` passes 838/838, SRP1–SRP6 included. No pre-existing assertion changed.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:
- Test files touched: only `tests/invariants/src/setup_user_phase_contract.rs`.
- Removed or changed assertions (`^-` lines with assert/#[test]/proptest/should_panic): **none**. The test file has zero `-` lines; the hunk is a pure append at line 1839.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance or epsilon): **none**.
- Inline `mod tests` changes: **none**.
- Added: the helpers `srp_str_const`, `srp_lexical`, `srp_point_read_paths_at_fake_root` and `srp_line`, and the tests `test_srp1`–`test_srp6`. After the rebase, the `PRESENCE_READ_FLAG` replacement in `srp_point_read_paths_at_fake_root` matches a real line of `setup.sh` (`setup.sh:553`). SRP3's "never `not applied`" assertion over the whole dry-run plan therefore also covers the presence note (`setup.sh:2411`), for `<parent>//etc/./` among other inputs.

No migration justification is needed: no existing test changed.

## 3. Deep Reasoning Audit

### Logic & Architecture
- `absolute_dir` was extracted and run on `//etc/./`, `/etc/`, `/etc//` (all give `/etc`), `/`, `//` and `/./` (all give `/`), `/a/./b//c/` (gives `/a/b/c`), `/etc/...` and `/etc/..x` (kept), `/et c/`, `/etc/*` (kept literally, no globbing), `/etc/..` and `/../etc` (exit 2) and `etc` (exit 2). All results are as specified. `usage_error` writes to stderr, and under `set -euo pipefail` (`setup.sh:49`) the failing `value="$(absolute_dir ...)"` assignment exits the parent with 2. **PASS**
- Composition with #454: `PRESENCE_FLAG="${SYSCONFDIR}/soos/presence.disable"` (`setup.sh:546`) is compared with `PRESENCE_READ_FLAG` (`:1033`). `LOCK_WIDGET_FLAG` (`:1087`) and `DAEMON_TOML` (`:1362`) follow the same pattern. With `--sysconfdir //etc/./` the normalised value is `/etc`, so every comparison is equal. SRP3 exercises this with the read paths redirected. **PASS**
- The remaining `<sysconfdir>/soos/disabled` mention, in `presence_state` (`:1075-1076`), is reached only from `presence_report` when `presence_flag_elsewhere` is false, that is when `SYSCONFDIR` is exactly `/etc` after normalisation. `KILL_SWITCH` is then `/etc/soos/disabled` = `KILL_SWITCH_READ_FLAG`, so the report names the read path. The rescue line no longer uses `KILL_SWITCH`. **PASS**: the mention is now covered.
- The `..` refusal for every path option, looking for callers: no script, workflow, Docker or distro harness under `.github/`, `packaging/`, `tests/docker/`, `tests/distro/` or `scripts/` calls `setup.sh` with a path option containing `..`. However, `setup.sh` itself calls the root half with `--artifact-dir "${ARTIFACT_DIR}"` (`:2867`). `ARTIFACT_DIR="${TARGET_DIR}/release"` (`:2323`), and `resolve_target_dir` returns `${WORKSPACE_ROOT}/${CARGO_TARGET_DIR}` for a relative value, unnormalised. A relative `CARGO_TARGET_DIR` is a documented contract (`Docs/PACKAGING_AND_PROVISIONING.md` line 150; `scripts/install.sh:38`). With `CARGO_TARGET_DIR=../shared-target` the user phase builds into `<checkout>/../shared-target/release`. The build takes several minutes, and then `as_root bash setup.sh --system-phase ... --artifact-dir <checkout>/../shared-target/release` is refused by `absolute_dir` with exit 2: "--artifact-dir must not contain a '..' component". Nothing is installed, and the error names an internal option the user never typed. An absolute `CARGO_TARGET_DIR` that contains `..` fails the same way. Before this branch the run succeeded. **FINDING (MAJOR)**
- The docs are stale after the rebase. The ADR item (`AI/DECISIONS.md:158`) still says "PR #454, open ... once it lands". The walkthrough (`AI/walkthroughs/244_setup_read_paths.md:12, 31, 42, 89`) says "PR #454 (still open)", "after its merge" and "After #454 merges, a rebase ...". **FINDING (MINOR)**
- `--sysconfdir /` gives `SYSCONFDIR=/`, so the derived paths are `//soos/...`. This is unchanged from before the branch and is never equal to a read path, so the outcome is correct, with cosmetic output only. Out of scope.

### PAM Concurrency & Deadlines
- No change to `crates/pam`, the daemon or the protocol. **PASS** (not applicable)

### Panic Safety & Fail-Closed
- The shell change adds no path to `PAM_SUCCESS` or `Allow`. Setup still never writes `/etc/soos/disabled` (SRP2 asserts that the file is absent). A refused `..` exits before any change (SRP5 asserts read-only calls and an unchanged snapshot). The MAJOR above fails safe, with nothing installed, but it is a functional regression. **PASS**

### Test Integrity & Anti-Weakening
- SRP1 pins the three sources (`setup.sh`, `crates/pam/src/config.rs`, `crates/daemon/src/presence/mod.rs` + `main.rs`) and would fail if any of them drifted. SRP3/SRP4 would fail against the pre-branch trailing-slash-only strip (`//etc/./` was reported "not applied"). SRP5 would fail if `realpath` were used (the link would equal `/etc`) or if `..` were resolved. The `unwrap`/`expect` calls in test helpers are test-only. No test covers `--artifact-dir` with a `..` derived from `CARGO_TARGET_DIR`, which is why the MAJOR passed CI. **PASS** (with the gap noted)

### Memory, Bounds & Secrets
- Parameter expansion only. No file reads, no secrets, no new output beyond paths. **PASS**

### Supply Chain & Automation
- No change to `Cargo.*`, `deny.toml`, `.github/` or the hooks. `scripts/setup.sh` adds no new external command (no `realpath`, `readlink` or `cd`). **PASS**

### English-Only Policy
- All added code, comments, docs and test text are in English. **PASS**

## 4. Detailed Findings & Action Items

- **[MAJOR]** `scripts/setup.sh:297-316` with `:2323` and `:2867`. The `..` refusal applies to the internal `--artifact-dir`, whose value setup itself derives from `CARGO_TARGET_DIR` without normalisation. A documented relative `CARGO_TARGET_DIR` (for example `../shared-target`), or an absolute one that contains `..`, makes the root half exit 2 after the full release build, and nothing is installed. Required correction, in production code only: setup must never pass `--system-phase` an `--artifact-dir` that its own parser refuses. Either resolve `TARGET_DIR` and `ARTIFACT_DIR` to a `..`-free path in the user phase before escalating (the directory exists after the build, so a physical `cd ... && pwd -P` is safe; it is not a read-path comparison), or exempt `--artifact-dir` from the `..` refusal while keeping it for the five user-facing path options. Add a new contract test (relative `CARGO_TARGET_DIR` containing `..` reaches the root half), and keep the docs' list of normalised options consistent with the choice.
- **[MINOR]** `AI/DECISIONS.md:158` and `AI/walkthroughs/244_setup_read_paths.md:12,31,42,89`. They still describe #454 as open or not yet merged. Reword them to state that #454 is on main and that the presence comparison composes with the normalisation (SRP3 covers it).

## 5. Final Verdict

**VERDICT: CHANGES_REQUESTED**
