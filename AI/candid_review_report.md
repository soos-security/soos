# Candid Review Report

- **Date**: 2026-10-01
- **Target Branch**: `fix/p3fu3-batch` (GitHub #291; merges `fix/p3fu3-storage-gui`, `fix/p3fu3-camera-diagnostics`, the integration fix `5ebefd1` and the review-fix commits `6d9a3b9` and `d4ebc85`)
- **Base (merge-base)**: `56b0350` (`origin/main`)
- **Reviewed-Diff-Fingerprint**: `c43f5e617aac1e2abad743a800f21e682806b41b02ea84e040391f6f5d846a51`
- **Fingerprint provenance**: `./scripts/candid_subagent.sh --prepare` on a clean working tree, and recomputed with the
  pinned `review_diff` options against the committed tree `HEAD^{tree}` (the input of the pre-push / CI `--rev` gate):
  both give the value above.
- **Previous reviews**: `841e2db5…ccf26` (HEAD `5ebefd1`, CHANGES_REQUESTED: 1 MAJOR i686 build break, 4 MINOR) and
  `622ad059…e02b9` (HEAD `6d9a3b9`, CHANGES_REQUESTED: 1 MAJOR, the CI runner lacked 32-bit libc headers). The only
  commit since the second review is `d4ebc85`; `git diff 6d9a3b9 HEAD` touches exactly `.github/workflows/ci.yml`,
  `AI/walkthroughs/155_config_open_and_diagnostics_followups.md`, `Docs/DEVELOPMENT_WORKFLOW.md` and
  `tests/invariants/src/config_open_diagnostics_contract.rs` (+25/−2). Nothing else changed.
- **Audited Files** (46): `.github/workflows/ci.yml`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/walkthroughs/154_store_lock_ui_and_import_followups.md`,
  `AI/walkthroughs/155_config_open_and_diagnostics_followups.md`, `Cargo.toml`,
  `Docs/{BIOMETRIC_STORE_CRATE,CAMERA_V4L_CRATE,DAEMON,DEVELOPMENT_WORKFLOW,ENROLLMENT_CLI,EVIDENCE_STORE_CRATE,GUI_APPLICATION,IPC_PROTOCOL}.md`,
  `crates/admin-cli/src/test_pam.rs`, `crates/admin-cli/tests/{cli_deadline_json,test_pam_acceptance}_tests.rs`,
  `crates/biometric-store/src/{error,lib,store}.rs`, `crates/biometric-store/tests/{enroll_if_absent,temp_sweep}_tests.rs`,
  `crates/camera-v4l/src/{daemon_config,mock,v4l_guard,v4l_impl}.rs`,
  `crates/camera-v4l/tests/{daemon_config_open_path,v4l_panic_hook_reinstall}_tests.rs`,
  `crates/daemon/src/{main,pipeline}.rs`, `crates/daemon/tests/store_temp_sweep_tests.rs`,
  `crates/enrollment-cli/src/{lib,main,service}.rs`,
  `crates/enrollment-cli/tests/{full_service_config_notes,import_enroll_if_absent}_tests.rs`,
  `crates/evidence-store/src/{lib,store}.rs`, `crates/evidence-store/tests/temp_sweep_tests.rs`,
  `crates/gui/src/{app,lib,store_tasks}.rs`, `crates/gui/tests/store_task_tests.rs`, `crates/pam/src/ipc.rs`,
  `tests/invariants/src/{config_open_diagnostics_contract,lib}.rs`.

## 1. Executive Summary

This is the third review round. Every finding of the two previous rounds is resolved.

- Round 1 (fixed in `6d9a3b9`, verified in round 2): `old_enough` takes `libc::time_t` in both
  stores (the i686 workspace check and the armv7 PCX check pass); a GUI worker panic or a lost
  outcome becomes a failed outcome of the right kind and UID; the success message uses the
  username captured at submit; both misplaced or stale doc comments are corrected.
- Round 2 (fixed in `d4ebc85`): the CI step "32-bit type check (i686)" now runs
  `sudo apt-get update` and `sudo apt-get install -y --no-install-recommends gcc-multilib` before
  `rustup target add` / `cargo check`. That provides the 32-bit libc headers needed by the
  `v4l2-sys-mit` bindgen run (reviewer reproduction in round 2: stock `ubuntu:24.04` fails on
  `sys/time.h`; with `gcc-multilib` the header parses). The invariant
  `test_cdf_ci_type_checks_a_32_bit_target` also requires that install, and
  `Docs/DEVELOPMENT_WORKFLOW.md` §4.2 names the header packages (`gcc-multilib` / `lib32-glibc`)
  instead of claiming no toolchain is needed.

The security analysis of the earlier rounds still applies, because the production code is
byte-identical to round 2:
- the temp sweeps only remove the stores' own canonical temp names, and only regular single-link
  files owned by root/euid, at least 60 s old, unlinked relative to a directory fd, bounded per
  call, with the lock tried once;
- `enroll_if_absent` is atomic and never reports success on an error;
- the GUI runner neither loses nor duplicates a mutation;
- the `O_PATH` + `/proc` reopen fails closed without an fd leak;
- the panic-hook filter identity logic is race-free;
- PAM `monotonic_nanos` is identical on 64-bit;
- `float_roundtrip` is harmless.

No CRITICAL or MAJOR finding remains. Verdict: APPROVED.

Gates run by the reviewer:
- This round: `cargo test --locked --all-features -p soos-invariants` passed 321 tests in two
  consecutive full runs. One earlier run had a single failure of
  `distro_matrix::test_mock_daemon_malformed_modes_put_one_defect_on_the_wire`. It passed in three
  isolated reruns and two full reruns, and this diff touches neither that test nor anything it
  exercises. I treat it as pre-existing flakiness (observation only).
- Round 2, on the unchanged Rust code: fmt clean, workspace clippy `-D warnings` clean, tests of
  the touched crates green, local i686 workspace check and armv7 PCX check pass.
- Round 1: `cargo deny --locked check` ok (Cargo files unchanged since).

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: as in round 2. `cli_deadline_json_tests.rs` and `tests/invariants/src/lib.rs`
  are modified; all the others are new files or new inline modules (`daemon_config.rs`
  `mod tests`, `store_tasks.rs` `mod worker_failure_tests`).
- Delta of `d4ebc85`: one **added** assertion in the new test
  `config_open_diagnostics_contract::test_cdf_ci_type_checks_a_32_bit_target`, which strengthens it.
- Removed or changed assertions in the frozen patch: **none**. New escape hatches (`#[ignore]`,
  `cfg(any())`, `should_panic`, `tolerance`, `epsilon`): **none**.
- The only edits to pre-existing tests are still the two allowed setup-only changes
  (`accepted: true, rejected_reason: None`, and the new `mod` line).

## 3. Deep Reasoning Audit

### Logic & Architecture

- CI step order is headers installed → target added → check, with `ORT_SKIP_DOWNLOAD: "1"` kept.
  The step stays inside the `clippy` job, before the `test:` job, which the invariant's split
  relies on.
- `openssl-sys` is only a host build dependency of `ort-sys` (via `ureq`), so no 32-bit OpenSSL
  is needed. `gcc-multilib` also covers the `cc` build dependency of `wayland-backend`. → PASS.
- Docs §4.2 and walkthrough 155 now describe the real prerequisites. → PASS.
- Production code is unchanged since round 2 (all PASS there). → PASS.

### PAM Concurrency & Deadlines

- `crates/pam` is unchanged since round 1: behaviour-identical `monotonic_nanos`, no thread or
  async, panic-free. → PASS.

### Panic Safety & Fail-Closed

- No production code changed in this round. The round-2 analysis stands: the GUI worker runs
  under `catch_unwind`, no new `unwrap/expect/panic!` exists outside `#[cfg(test)]`, and no error
  is mapped to success. → PASS.

### Test Integrity & Anti-Weakening

- See §2: the only change is an added assertion. → PASS.

### Memory, Bounds & Secrets

- Unchanged. Sweeps and GUI outcomes log only counts, UIDs and static messages. → PASS.

### Supply Chain & Automation

- `ci.yml` step: `sudo apt-get` installs only from the runner's configured Ubuntu archives, with
  `--no-install-recommends` to keep the install minimal. It adds no action, no new `permissions:`
  and no `${{ github.event.* }}` interpolation in `run:`. `rustup target add` uses the toolchain
  pinned in `rust-toolchain.toml`.
- No `Cargo.*` or `deny.toml` change in this round. → PASS.

### English-Only Policy

- The workflow comment, docs, walkthrough and invariant message are in English. → PASS.

## 4. Detailed Findings & Action Items

- No CRITICAL, MAJOR or MINOR finding.
- **[SUGGESTION]** `crates/gui/src/store_tasks.rs`: add a test that drives `StoreTaskRunner::poll`
  through the lost-outcome branch. The author declined it as optional; a real temporary store
  would be enough.
- **[SUGGESTION]** `crates/evidence-store/src/store.rs` sweep: list partitions through the opened
  fd, or recheck dev/ino. This is harmless today, because unlinks are fd-relative and fully
  re-checked.
- **[SUGGESTION]** `tests/invariants/src/distro_matrix.rs:1275`
  `test_mock_daemon_malformed_modes_put_one_defect_on_the_wire` failed once in six runs (full and
  isolated) on the reviewer host. It is outside this diff and worth a separate flakiness ticket.

## 5. Final Verdict

**VERDICT: APPROVED**

The gate result of `./scripts/candid_subagent.sh` after writing this report is recorded in the
reviewer's hand-off message.
