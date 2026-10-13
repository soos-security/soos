# Candid Review Report

- **Date**: 2026-10-13
- **Target Branch**: `feat/one-command-install`
- **Base (merge-base)**: `6f32ef8`
- **Reviewed-Diff-Fingerprint**: `7298454af48fd2d80f05a3f527cf6a01a21f5b8e8e23202eed8af5b27a86fe8d`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_one_command_install.md`, `AI/auditor_constraints_one_command_install.md`, `AI/tester_contract_one_command_install.md`, `AI/walkthroughs/200_one_command_install.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `README.md`, `scripts/setup.sh`, `tests/invariants/src/lib.rs`, `tests/invariants/src/setup_contract.rs`

## 1. Executive Summary

Re-review of PR #385 (`scripts/setup.sh`, GitHub #372, PR 1 of #362) after a CI failure. The
branch at `c44d2e7` was APPROVED under fingerprint `3eff69a5...`. The new delta
(`git diff --cached HEAD`) touches only three files: `tests/invariants/src/setup_contract.rs`
(harness), `AI/tester_contract_one_command_install.md` and `AI/walkthroughs/200_one_command_install.md`
(documentation of the fix). `scripts/setup.sh` is byte-identical to the approved commit
(`git diff HEAD -- scripts/` is empty).

Three tests (`test_set_second_run_is_idempotent`, `test_set_presence_flag_toggles`,
`test_set_damaged_install_and_reinstall`) failed on the GitHub runner (uid 1001) because the
`id`/`getent` stubs hard-coded uid 1000 for the fake build user `alice`, while every file of the
harness (including the fake `repo/target`) is owned by the real test uid. The second run's
ownership preflight (`find -H "${TARGET_DIR}" ! -uid "${BUILD_UID}"`, `scripts/setup.sh:630`)
then correctly refused. The fix gives `alice` the real uid of the test process (`STUB_UID`).
This is a harness correction, not a weakening: it changes no assertion and no production code,
and the CI failure itself shows the production preflight works. No CRITICAL or MAJOR finding.
Verdict: APPROVED.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/lib.rs` (adds `mod setup_contract;`) and
  `tests/invariants/src/setup_contract.rs` (new in this PR; every test in it is new).
- Removed or changed assertions (`^-` lines with assert, `#[test]`, proptest, should_panic):
  **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, should_panic, tolerance, epsilon): **none**.
- Inline test modules added or removed: none.

Delta since the approved commit (harness only, in `setup_contract.rs`):

| Change | Effect | Justification |
|---|---|---|
| `real_uid()` helper (runs the real `id -u`, asserts numeric and non-root) | Source of `STUB_UID` | Adds checks; does not relax any |
| `STUB_UID` baked into every stub (also into `id` and `getent`, which had no baked prologue before) and exported in the harness env | Stubs work under `env -i` and through the `sudo` stub (`STUB_*` names are kept) | Consistency with the existing baked paths |
| `id -u`, `id -u alice`, `getent passwd alice/<uid>` return `STUB_UID` | Fake account matches the real file owner | Root cause of the CI failure |
| `sudo` stub: `SUDO_UID="${STUB_UID}"` | `SUDO_UID` agrees with `id -u alice` | Same |
| `soos-enroll` fake writes `${STUB_UID}.cbor.enc` | Matches `setup.sh:722` (`${TARGET_UID}.cbor.enc`), so the enrolled-template detection keeps the same meaning | Same |

`uid=1000` in `STUB_PROLOGUE` log lines (asserted at lines 1132, 1152-1154, 1281, 1349-1350,
1618) is a role label set by `__uid=1000` / `STUB_AS_ROOT`; it is unchanged, so those
assertions keep their exact meaning (normal user vs root). No test asserts the literal numeric
uid 1000 anywhere else.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Scenario: is the delta masking a real defect in `setup.sh`? The preflight compares the target
  directory owner with the resolved build uid. In production, the build user owns its checkout,
  so `id -u <user>` equals the owner; the old harness violated that premise only on hosts whose
  uid is not 1000. The fix restores the premise. PASS.
- Scenario: is any other uid-dependent path left inconsistent? `getent passwd 1000` was replaced
  by `passwd ${STUB_UID}`; the enrolled template name follows `STUB_UID`; `SUDO_UID` follows it.
  Grep of `1000` in the file shows only the role-label prologue and its assertions. PASS.
- Scenario: first-run vs second-run. The first run passed on CI because `repo/target` did not
  exist yet; the second run now exercises the preflight's positive path with a matching owner.
  PASS.

### PAM Concurrency & Deadlines
- Not affected (no `crates/pam` change). PASS.

### Panic Safety & Fail-Closed
- No production Rust or PAM code changes. The production ownership preflight is unchanged and
  still fails closed (refuses on a foreign-owned file). `expect`/`assert` in `real_uid()` are in
  test code only. PASS.

### Test Integrity & Anti-Weakening
- Reproduction: the test binary built from the working tree, run as uid 1000: 28 passed,
  0 failed. Run as uid 1001 via `unshare --user --map-user=1001 --map-group=1001 <binary>
  setup_contract --test-threads=4`: 28 passed, 0 failed (`id -u` inside the namespace prints
  1001). The pre-fix failure under uid 1001 (25 passed, 3 failed) is reported by the author in
  the tester contract; it is consistent with the CI log and with the code path at
  `setup.sh:630`.
- Scenario: could a wrong implementation now pass? If `setup.sh` stopped checking ownership, the
  tests would pass as before (they did not assert the refusal earlier either). The coverage of
  the preflight's negative path is unchanged (it cannot be exercised without `chown` as root),
  so nothing was lost. Noted as a pre-existing limitation, not introduced here.
- `real_uid()` refuses to run as root: under `sudo cargo test` the suite now fails loudly
  instead of producing misleading results. CI runs as the unprivileged `runner` user (no
  `container:` job runs the invariants suite). PASS.

### Memory, Bounds & Secrets
- No secrets, frames or embeddings involved; the uid is not sensitive. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `.github/`, `.githooks/` or `scripts/` change in the delta. `cargo fmt --check`
  clean; `cargo clippy -p soos-invariants --all-targets -- -D warnings` clean. PASS.

### English-Only Policy
- New comments and documentation are in English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `tests/invariants/src/setup_contract.rs` — the ownership preflight
  (`scripts/setup.sh:630`) has no negative-path test. A future test could create a target owned
  by a different uid inside a user namespace with a mapped range, or stub `find`. Pre-existing,
  optional.

No CRITICAL, MAJOR or MINOR findings. The rest of the branch diff is identical to the version
approved under fingerprint `3eff69a5...`; its earlier review stands.

## 5. Final Verdict

**VERDICT: APPROVED**
