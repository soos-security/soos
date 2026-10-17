# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `feat/setup-user-phase`
- **Base (merge-base)**: `01ce6fa` (rebased from `9e82795`; `origin/main` gained #411, #417 and #413)
- **Reviewed-Diff-Fingerprint**: `7c946bc13225f52c5b2760673d8e14e991c74df283871e6ad11b868bdebad3f0`
- **Audited Files**: `.github/workflows/ci.yml`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_setup_user_phase.md`, `AI/auditor_constraints_setup_user_phase.md`, `AI/tester_contract_setup_user_phase.md`, `AI/walkthroughs/213_setup_user_phase.md`, `Docs/CI_CD_AND_SECURITY.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/REMOTE_COMPANION.md`, `README.md`, `crates/remote/tests/setup_render_tests.rs`, `scripts/install_remote.sh`, `scripts/setup.sh`, `tests/docker/setup_root_preview_test.sh`, `tests/fixtures/setup/remote.toml`, `tests/invariants/src/lib.rs`, `tests/invariants/src/setup_contract.rs`, `tests/invariants/src/setup_user_phase_contract.rs`

## 0. Re-Review After the Rebase Onto `origin/main` `01ce6fa`

Independent re-review, without author context, of the branch after its rebase from `9e82795` onto
`01ce6fa`. `origin/main` gained three commits in between: #411 (`6eb0746`, daemon `LockState` refusal
test on the auth-reserved connection), #417 (`085ba63`, zbus 5 -> 6 migration in `Cargo.toml`/`Cargo.lock`,
`crates/daemon/src/presence/logind.rs`, `crates/remote/src/{logind,offline,session}.rs`, their tests and
three invariant files) and #413 (`01ce6fa`, lazy second shm buffer of `soos-lock-widget`).

Rebase integrity, checked mechanically:
- `git range-diff 9e82795..origin/feat/setup-user-phase 01ce6fa..HEAD`: commit 1 (`7171450`, the
  owner-approved SET3 contract migration, formerly `e57c0e1`) is `=`; commits 2-4 are `!` only because
  of context lines in `AI/DECISIONS.md` (the ADR line now follows the #410/#412 ADR lines), in
  `AI/candid_review_report.md` and `AI/plan_evaluator_report.md` (pre-images changed on main), and in
  `.github/workflows/ci.yml` hunk headers.
- The `+`/`-` lines of `git diff 9e82795 origin/feat/setup-user-phase` and `git diff 01ce6fa HEAD`
  (both excluding the two report files) are byte-identical, and the branch's
  `AI/plan_evaluator_report.md` is byte-identical before and after the rebase. The branch's own changes
  are therefore unchanged by the rebase.

Semantic interaction with what main gained:
- `git diff 9e82795 01ce6fa` touches nothing under `scripts/`, `.github/`, `packaging/` or
  `tests/docker/`; `setup.sh` queries logind only through `loginctl` (shell), so the zbus 6 migration
  does not reach it.
- `crates/remote/tests/setup_render_tests.rs` uses only `soos_remote::config::{parse_config, ConfigError}`;
  #417 changed `logind.rs`, `offline.rs`, `session.rs`, not `config.rs`. The test builds and passes
  against the zbus 6 `soos-remote`.
- #417 edited `remote_companion_contract.rs`, `presence_unlock_contract.rs` and
  `portable_crates_contract.rs`; the branch edits `lib.rs` (one module registration), `setup_contract.rs`
  and the new `setup_user_phase_contract.rs`. No overlap; the whole invariant suite passes together.
- #413 and #411 touch `crates/lock-widget` and `crates/daemon` tests only: no interaction.

Executed evidence (this review): `LIBONNXRUNTIME_NO_PKG_CONFIG=1 CARGO_TARGET_DIR=~/.cache/soos-targets/setup-user-phase
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=4 cargo test --locked
-p soos-invariants -p soos-remote -- --test-threads=4`: 1186 passed, 0 failed (soos-invariants 731);
`cargo clippy --locked -p soos-invariants -p soos-remote --all-targets -- -D warnings`: clean (zbus 6.0.0).
The Docker harness `tests/docker/setup_root_preview_test.sh` was not re-run here (no input of it changed
in the rebase); it runs as the required CI job `setup-root-preview`.

Test assertions: the step-3 greps on the new frozen patch find no removed assertion, no new
`#[ignore]`/`cfg(any())`/`should_panic`/tolerance, no inline test module change. The only modified
pre-existing test remains SET3 in `setup_contract.rs`, the owner-approved 2026-10-15 migration recorded
in its own commit `7171450` and in ADR 2026-10-15 "Setup User Phase". No other pre-existing assertion changed.

Sections 1-4 below are the previous APPROVED review (fingerprint `ae799ec6…`, base `9e82795`), whose
findings still apply to the unchanged branch content; line references in `scripts/setup.sh` are unchanged.
The SUGGESTION on stale commit references now also applies to `e57c0e1` -> `7171450`.

## 1. Executive Summary

The frozen patch (merge-base `9e82795` to the working tree, 4631 lines) adds the phone-companion user
phase to `scripts/setup.sh` (GitHub #400, PR 2 of #362): companion options, a strict `remote.toml`
classifier (absent / untouched template / setup-managed / user), default-on feature resolution with the
prerequisite cascade, a journaled companion rollback with exit 5, the root-half camera-view daemon side
(`/etc/soos/daemon.toml` `[preview]` created only when absent, by hard link, restart + readiness gate,
removal on failure with exit 1 or 70), and the `install_remote.sh` `CARGO_TARGET_DIR` fix. The newest,
uncommitted part adds a privileged-systemd Docker harness `tests/docker/setup_root_preview_test.sh`
that drives the unmodified `setup.sh` through the real `sudo` into the real `/etc` creation path, a
required CI job `setup-root-preview` (in `CI Success` needs), the invariant
`test_set_u24_root_preview_docker_job_wired_into_ci`, and matrix/docs updates.

Executed evidence (this review, not the author's): `bash tests/docker/setup_root_preview_test.sh`
passed all eight cases (A1 A2 B C1 C2 D E1 E2) and the host sysctl guard; `cargo test -p soos-invariants`
731 passed / 0 failed; `cargo test -p soos-remote --test setup_render_tests` 3 passed; ShellCheck
(stable, all severities) on `setup.sh`, `install_remote.sh` and the new harness: clean.

No CRITICAL or MAJOR finding. Four MINOR/SUGGESTION items are listed in section 4.

## 2. Test Changes (mechanical listing from step 3)

Test files touched:
- `crates/remote/tests/setup_render_tests.rs` (new): the setup render fixture and every cascade
  variant parsed by the real `soos-remote` `parse_config`; one variant the cascade never writes is
  refused. Additive.
- `tests/docker/setup_root_preview_test.sh` (new, untracked): Docker harness, additive.
- `tests/fixtures/setup/remote.toml` (new): fixture, byte-equal to the render (asserted in SET38).
- `tests/invariants/src/lib.rs`: registers `setup_user_phase_contract`. Additive.
- `tests/invariants/src/setup_user_phase_contract.rs` (new): SET20-SET39 contracts, including the new
  `test_set_u24_root_preview_docker_job_wired_into_ci`. Additive.
- `tests/invariants/src/setup_contract.rs`: **modified** `test_set_s3_no_pam_tailscale_remote_or_network_lock_action`.

Removed/changed assertions (`^-` assert grep): none matched the grep, but the SET3 diff removes three
banned needles (`install_remote.sh`, `[preview]`, `remote.toml`) and relaxes the `tailscale ` rule to
"`tailscale version` or inside a quoted string". Justification: this is the scope change of the issue
itself (the user phase must run `install_remote.sh` and write `remote.toml` / `[preview]`), recorded as
an owner-approved contract migration in its own commit `e57c0e1` (verified byte-identical to the
referenced pre-rebase commit `40b0fb1`, and not modified since), documented in the test doc comment, in
`AI/tester_contract_setup_user_phase.md` and ADR 2026-10-15. The executed-call check
`assert_no_forbidden_calls` (only `tailscale version` may run) is unchanged and still applied by the new
behavioral contracts. Accepted as a justified migration, not a weakening. See finding 2 for a residual
gap of the quote heuristic.

New escape hatches (`#[ignore]`, `cfg(any())`, `should_panic`, tolerance/epsilon): none.
Inline test modules changed: none.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Root half receives only `--camera-view on`; `--system-phase` rejects any other companion option
  (parser block after option parsing). Tried: `--system-phase --funnel on` → usage error. PASS.
- `daemon_preview_state` ordering: `elsewhere` (sysconfdir not `/etc`) before any read; `-L` before
  `-e` so a dangling link is `link`, not `absent`. Harness E1 (dangling link) and E2 (link to file)
  confirm: link kept, target never created or edited, no restart. PASS.
- Creation race: `ln tmp daemon.toml` fails if the file appeared after the check → `return 1` →
  `fail_and_rollback`; the administrator file is never overwritten. PASS.
- Failure path: file removed and its journal entry dropped before the second restart; second failure
  → `PREVIEW_DAEMON_DOWN` → exit 70 without further rollback; otherwise exit 1 with journal rollback
  (removes `/etc/soos` only when this run created it, `rmdir` never removes a non-empty dir). Harness
  C1/C2/D assert exactly this, including that the daemon was actually restarted with and then without
  the file (start log). PASS.
- Companion runs only after root-half exit 0 or 3; any other exit prints `[SKIP] companion` and keeps
  the root exit code (harness `companion_not_run` on C1/C2/D). Exit 3 is preserved over 5. PASS.
- `install_remote.sh`: relative `CARGO_TARGET_DIR` resolved against the checkout where cargo runs
  (`cd "${REPO_ROOT}"`). PASS.
- Restart bound vs unit start timeout: see finding 1. MINOR.

### PAM Concurrency & Deadlines
- No change under `crates/pam`. All new `systemctl`/`soos-remote` calls go through `run_bounded`;
  `daemon_ready` uses `wait_daemon_ready.sh` with its own timeout. PASS (not applicable to PAM).

### Panic Safety & Fail-Closed
- No Rust production code changed. In shell, every error of the preview path leads to exit 1 or 70,
  never to a `[DONE]`; `ensure_soos_conf_dir` refuses a symlinked or non-directory `/etc/soos`. The
  companion failure path rolls back its own journal and exits 5. Tried: readiness gate fails while
  restart succeeds (C1) → exit 1, file gone, daemon healthy again. PASS.

### Test Integrity & Anti-Weakening
- New harness can fail on plausible wrong implementations: content compared to an independently
  spelled expected file; mode/owner/link-count asserted; inode+mtime identity for "never edited";
  invocation ID to prove restart/no restart; start log proves the daemon really ran with the file.
  A setup.sh that edited an existing file, followed a symlink, skipped the restart or left the file
  after a failed gate would fail B, E1/E2, A1 or C1 respectively. PASS.
- `setup.sh` under test is byte-compared to the workspace copy (`cmp -s`), no test hook added to
  `setup.sh` (`DAEMON_READ_CONFIG` is a literal constant, asserted by SET34 invariant). PASS.
- `test_set_u24` is a static wiring check (job present, required in `CI Success`, not advisory in
  `pr_checks_gate.sh`, read-only mount, no `--sysconfdir`). Appropriate for wiring. PASS.
- SET3 migration: justified (section 2). Residual heuristic gap: finding 2.

### Memory, Bounds & Secrets
- `remote.toml` / `daemon.toml` scans are bounded (`head -c` 16 KiB / 64 KiB). Login validated by a
  strict charset before any TOML write (no injection). The plan prints "login given", not the login.
  `remote.toml` 0600 in a 0700 dir via temp file + rename; backup 0600; symlinked paths refused.
  `daemon.toml` 0644 root:root (non-secret content: uid list and booleans). PASS.
- Harness host isolation: `--privileged --cgroupns=private`, tmpfs `/run`, masked host-state units
  asserted, host `kernel.*/vm.*/fs.*` sysctls compared before/after (passed locally). The workspace is
  mounted read-only. PASS.

### Supply Chain & Automation
- New CI job: `actions/checkout` pinned by SHA (same pin as the other jobs), `persist-credentials: false`,
  no job-level `permissions:` (workflow default `contents: read`), no `${{ github.event.* }}` in `run:`,
  `timeout-minutes: 20`, `needs: lint`, added to `CI Success` needs whose step fails on any non-success
  result. Docker image is the existing digest-pinned `tests/docker/Dockerfile.systemd`; the container
  installs `python3-minimal` and `sudo` from Ubuntu archives (same trust as the existing systemd job).
  The new `.sh` is picked up by the lint job's `git ls-files '*.sh'` ShellCheck once committed. No
  `Cargo.*` / `deny.toml` change. PASS.

### English-Only Policy
- Code, comments, docs, harness output and test messages reviewed: English only. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `scripts/setup.sh:1652` (and `:1656`) — `systemctl restart soos-daemon` is bounded by
  `QUERY_TIMEOUT_S=10` while the shipped unit is `Type=notify` with `TimeoutStartSec=60`
  (`packaging/soos-daemon.service:30`). On a slow host where the real daemon (model verification and
  ONNX session load) needs more than 10 s to send `READY=1`, the client is killed, the step is treated
  as failed, the new `daemon.toml` is removed and the run exits 1 although the configuration was valid.
  Fail-closed, so not a security issue, and the same pattern pre-exists at `:1728`; consider
  `systemctl restart --no-block` followed by the `daemon_ready` gate, or a dedicated start timeout.
- **[MINOR]** `tests/invariants/src/setup_contract.rs:165` and
  `tests/invariants/src/setup_user_phase_contract.rs:68` (`quoted_at`) — the quote-parity heuristic
  treats `x="$(tailscale serve --bg ...)"` as "inside a quoted message", so the static rule would not
  catch an executed `tailscale serve`/`funnel` (or `usermod`, `enable-linger`) inside a quoted command
  substitution. The behavioral `assert_no_forbidden_calls` still catches it on exercised paths only.
  Suggest additionally banning `$(tailscale`, `` `tailscale `` and `$(usermod` in the static checks.
- **[SUGGESTION]** `scripts/setup.sh:1643-1650` — a signal between `mktemp` and `rm -f "${tmp}"`
  leaves `/etc/soos/.daemon.toml.XXXXXX`, and a signal between `ln` and `JOURNAL+=(daemon_toml_created)`
  leaves an unjournaled `daemon.toml`. The windows are a few instructions wide; journaling cannot simply
  move before `ln` (a lost race would then delete an administrator file), so the fix would be a local
  trap or a flag consulted by `on_signal`.
- **[SUGGESTION]** `AI/tester_contract_setup_user_phase.md:6`, `AI/architect_spec_setup_user_phase.md:6`
  — refer to the SET3 migration as commit `40b0fb1`; on this branch it is `e57c0e1` (rebased,
  content verified identical). Reference the subject line or both SHAs.

## 5. Final Verdict

No CRITICAL or MAJOR finding after the rebase; the two MINOR and two SUGGESTION items above carry forward.

**VERDICT: APPROVED**
