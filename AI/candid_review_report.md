# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `fix/setup-read-paths`
- **Base (merge-base)**: `8f03eb7`
- **Reviewed-Diff-Fingerprint**: `c81d4ec2699fb51e34c9dd3c66e4559b003d1cc9747a88c11b1f6e4cd5b1be8b`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/244_setup_read_paths.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `scripts/setup.sh`, `tests/invariants/src/setup_user_phase_contract.rs`

## 1. Executive Summary

The diff fixes both defects of GitHub #455. `setup.sh` pins `readonly KILL_SWITCH_READ_FLAG="/etc/soos/disabled"` and the single `Rescue:` summary line names it instead of `${KILL_SWITCH}` (`<sysconfdir>/soos/disabled`). `absolute_dir` keeps the existing validation (absolute, no newline), then normalises every path option once with pure parameter expansion: empty and `.` components are dropped, `..` exits 2 through `usage_error`, and `/` is returned when nothing is left. The constants were checked against the source: `crates/pam/src/config.rs:22` `DEFAULT_FLAG_DIR = "/etc/soos"`, `:120` `flag_dir.join("disabled")`, `:167` the only `flag_dir` assignment; `crates/daemon/src/presence/mod.rs:155/158`; `main.rs:222` `PresenceSwitch::new(PathBuf::from(DEFAULT_KILL_SWITCH_DIR))`. There is no PAM, daemon or protocol change. The six SRP tests are new functions only. `bash -n` passes, and `cargo test -p soos-invariants` passes 834/834. No CRITICAL or MAJOR finding.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:
- Test files touched: `tests/invariants/src/setup_user_phase_contract.rs` only. The diff adds lines at the end of the file and removes none.
- Removed lines (`^-`) in the whole patch: only the old `absolute_dir` body (lines `local opt…`, the trailing-slash loop, `printf '%s' "${value}"`) and the old `Rescue:` line of `scripts/setup.sh`. No assertion, `#[test]` or `proptest!` is removed.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.
- No pre-existing test needed migration, and the full invariant suite (SET*, LKI*, LWF*, PFP*, DCC*) is green unchanged.

## 3. Deep Reasoning Audit

### Logic & Architecture
- I ran the extracted `absolute_dir` on these inputs: `/etc/`→`/etc`, `/etc//`→`/etc`, `//etc/./`→`/etc`, `/`→`/`, `//`→`/`, `/a/.../b` unchanged (`...` is not `..`), `/a/b/.`→`/a/b`. Each loop iteration consumes at least one character, so the loop ends. Glob characters in the value are inert, because `${rest%%/*}` and `case "${part}"` match literal patterns. PASS.
- `..` refusal: `usage_error` runs inside `$(absolute_dir …)` and exits the subshell with 2. Under `set -e` the assignment `value="$(…)"` then fails and the script exits 2 during option parsing, before detection, planning or any change. SRP5 checks this with the read-only-call log and a snapshot. PASS.
- Idempotence: the normalised value goes to the root half (`--system-phase`) and to `install.sh`, and normalising it again changes nothing. `--artifact-dir` is covered too. PASS.
- Rescue path: the pinned constant equals both readers. `KILL_SWITCH` stays only for the state report, which the issue keeps unchanged. PASS. One residual wording issue is noted in Finding 1.
- Independence from PR #454 (open): #454 touches `tests/invariants/src/setup_contract.rs`, not `setup_user_phase_contract.rs`. The test's `PRESENCE_READ_FLAG` replacement is a no-op when the constant is absent. The docs, ADR and walkthrough describe #454 as open ("once it lands") and never claim its content is on main. PASS.

### PAM Concurrency & Deadlines
No change in `crates/pam`. N/A, PASS.

### Panic Safety & Fail-Closed
The diff touches no Rust production code. The test-only `unwrap`/`expect` calls are acceptable. Failing a test can never grant access. PASS.

### Test Integrity & Anti-Weakening
- SRP1 pins the shell constant, the PAM and daemon constants, the single `flag_dir` assignment and the absence of `${KILL_SWITCH}`/`${SYSCONFDIR}` in the rescue line. It fails on the old code. PASS.
- SRP2 fails on the old code, which printed `<root>/etc/soos/disabled`. PASS.
- SRP3, third spelling, and SRP5 (`..` accepted before) fail on the old code. SRP5's symlink case kills a `realpath`/`readlink -f` implementation. PASS.
- SRP4 and the first two SRP3 spellings were already green before the change. The walkthrough says so openly, and they pin the trailing-slash behaviour. SUGGESTION only.

### Memory, Bounds & Secrets
The change writes no new file, logs no secret, and setup still never creates the kill switch (SRP2 asserts it). PASS.

### Supply Chain & Automation
Only `scripts/setup.sh` changes: no `eval`, no new external tool, no network. No CI, Cargo or deny change. PASS.

### English-Only Policy
All added text is in English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `scripts/setup.sh:1021-1022`: under a non-`/etc` `--sysconfdir`, the state report still prints "everything off (`<sysconfdir>/soos/disabled` present: face PAM and presence auto-unlock stopped)". Neither reader looks at that file, so the claim is false in that case. The issue and ADR item (1) keep this report unchanged on purpose, so it is out of scope here. Follow-up: compare against `KILL_SWITCH_READ_FLAG`, or qualify the wording when `KILL_SWITCH` differs from it.
- **[SUGGESTION]** `tests/invariants/src/setup_user_phase_contract.rs` (SRP6, `para.find(" | ")`): the slice runs past the end of the "Read paths" paragraph, up to the next table cell separator. A needle could therefore be satisfied by a later paragraph. Cut the slice at the next `**` paragraph heading instead.
- **[SUGGESTION]** SRP4 was not red before the change, because trailing slashes were already stripped. A real-run spelling with an inner `//` or `/./` would make it a regression detector for the new logic too.

## 5. Final Verdict

**VERDICT: APPROVED**
