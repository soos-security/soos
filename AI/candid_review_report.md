# Candid Review Report

- **Date**: 2026-09-30
- **Target Branch**: `fix/p0-review-batch`
- **Base (merge-base)**: `2623805`
- **Reviewed-Diff-Fingerprint**: `637203568cabb14137834ffaff0c44e5f1f3bc979440ec4a4f9b70383abd7509`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `.github/workflows/ci.yml`, `AI/ARCHITECTURE.md`, `AI/BACKLOG.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/78_pam_release_panic_unwind.md`, `AI/walkthroughs/79_pad_live_class_index_single_source.md`, `AI/walkthroughs/80_pad_multiframe_consensus.md`, `AI/walkthroughs/81_preview_frame_authorization.md`, `AI/walkthroughs/82_debug_vision_report_safety.md`, `AI/walkthroughs/83_package_master_key_isolation.md`, `AI/walkthroughs/84_fedora_authselect_profile_activation.md`, `Cargo.lock`, `Cargo.toml`, `Docs/CI_CD_AND_SECURITY.md`, `Docs/DEVELOPMENT_WORKFLOW.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md`, `Docs/ENROLLMENT_CLI.md`, `Docs/INFERENCE_ORT_CRATE.md`, `Docs/IPC_PROTOCOL.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/PAM_DOCKER_TEST_MATRIX.md`, `Docs/PAM_MODULE.md`, `Docs/POLICY_CRATE.md`, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`, `crates/daemon/Cargo.toml`, `crates/daemon/src/config.rs`, `crates/daemon/src/dispatcher.rs`, `crates/daemon/src/lib.rs`, `crates/daemon/src/main.rs`, `crates/daemon/src/pipeline.rs`, `crates/daemon/src/preview.rs`, `crates/daemon/tests/dispatcher_tests.rs`, `crates/daemon/tests/pad_wiring_tests.rs`, `crates/daemon/tests/pipeline_integration_tests.rs`, `crates/daemon/tests/preview_authorization_tests.rs`, `crates/enrollment-cli/Cargo.toml`, `crates/enrollment-cli/src/args.rs`, `crates/enrollment-cli/src/error.rs`, `crates/enrollment-cli/src/html_report.rs`, `crates/enrollment-cli/src/lib.rs`, `crates/enrollment-cli/src/main.rs`, `crates/enrollment-cli/src/service.rs`, `crates/enrollment-cli/tests/debug_vision_tests.rs`, `crates/enrollment-cli/tests/pad_wiring_tests.rs`, `crates/gui/Cargo.toml`, `crates/gui/src/ipc_camera.rs`, `crates/gui/src/lib.rs`, `crates/gui/src/main.rs`, `crates/gui/tests/ipc_camera_tests.rs`, `crates/inference-ort/src/lib.rs`, `crates/inference-ort/src/mock.rs`, `crates/inference-ort/src/pad.rs`, `crates/inference-ort/src/registry.rs`, `crates/pam/Cargo.toml`, `crates/pam/src/config.rs`, `crates/pam/src/fault_injection.rs`, `crates/pam/src/lib.rs`, `crates/pam/tests/fault_injection_tests.rs`, `crates/policy/src/decision.rs`, `crates/policy/src/error.rs`, `crates/policy/src/lib.rs`, `crates/policy/src/pad_consensus.rs`, `crates/policy/tests/decision_tests.rs`, `crates/policy/tests/pad_consensus_tests.rs`, `crates/protocol/src/types.rs`, `crates/protocol/tests/preview_tests.rs`, `models/README.md`, `packaging/arch/PKGBUILD`, `packaging/arch/soos.install`, `packaging/debian/postinst`, `packaging/debian/rules`, `packaging/pam/fedora/soos/README`, `packaging/pam/fedora/soos/REQUIREMENTS`, `packaging/pam/fedora/soos/dconf-db`, `packaging/pam/fedora/soos/dconf-locks`, `packaging/pam/fedora/soos/fingerprint-auth`, `packaging/pam/fedora/soos/nsswitch.conf`, `packaging/pam/fedora/soos/password-auth`, `packaging/pam/fedora/soos/postlogin`, `packaging/pam/fedora/soos/smartcard-auth`, `packaging/pam/fedora/soos/system-auth`, `packaging/rpm/soos.spec`, `run_tests.sh`, `scripts/build_arch.sh`, `scripts/build_deb.sh`, `scripts/check_no_key_material.sh`, `scripts/install.sh`, `scripts/provision_master_key.sh`, `scripts/uninstall.sh`, `tests/distro/fedora_rhel_test.sh`, `tests/docker/authselect_profile_test.sh`, `tests/docker/test_packages.sh`, `tests/docker/test_suite.sh`, `tests/fixtures/mod.rs`, `tests/invariants/src/lib.rs`

## 1. Executive Summary

Integration review of seven individually approved P0 fixes (#148 PAM `panic = "unwind"` + T10,
#146 PAD live class index single source, #147 multi-frame PAD consensus with spoof veto, #143
PreviewFrame authorization + rate limit, #149 debug-vision report safety, #144 no master key in
packages, #145 Fedora authselect profile + rollback + CI job) merged into `fix/p0-review-batch`.
The review focused on the hand-resolved merge conflicts and on semantic interactions between fixes.

Conflict resolution was verified mechanically: for every one of the seven fix commits, every line
it added (relative to its base `fbb99c4`) is present in the integrated tree, and no line any
branch removed was revived beyond what another branch legitimately re-added. The only "missing"
lines are the two single-purpose `use soos_policy::...` imports in `dispatcher.rs`, correctly
merged into one `use soos_policy::{ConsensusDecision, FrameEvaluation, PadAggregator, RateLimiter};`,
and the 2026-09-20 ADR line that gained its "(Superseded ...)" suffix. ADRs appear in
chronological/merge order (Release Panic, Live Class Index, PAD Consensus, Preview Exception);
all six new matrix components (PRU, PLC, PMC, PFA, PMK, FAP) are complete; `ci-success` needs the
new `authselect-profile` job; T1–T10 wording is consistent across ci.yml, run_tests.sh,
Docs/CI_CD_AND_SECURITY.md and Docs/PAM_DOCKER_TEST_MATRIX.md.

No CRITICAL or MAJOR defect found. Two MINOR items and two SUGGESTIONS are listed in §4.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Test files touched: `crates/daemon/tests/{dispatcher_tests,pad_wiring_tests,pipeline_integration_tests,preview_authorization_tests}.rs`,
`crates/enrollment-cli/tests/{debug_vision_tests,pad_wiring_tests}.rs`, `crates/gui/tests/ipc_camera_tests.rs`,
`crates/pam/tests/fault_injection_tests.rs`, `crates/policy/tests/{decision_tests,pad_consensus_tests}.rs`,
`crates/protocol/tests/preview_tests.rs`, `run_tests.sh`, `tests/distro/fedora_rhel_test.sh`,
`tests/docker/{authselect_profile_test,test_packages,test_suite}.sh`, `tests/fixtures/mod.rs`,
`tests/invariants/src/lib.rs`. No inline `mod tests` added/removed.

Removed/changed assertions:
- `tests/invariants/src/lib.rs` (`test_install_script_creates_required_directories`):
  `assert!(master_key.is_file())` → `assert!(!master_key.exists())`, and the 0600/32-byte checks
  moved to `test_provision_master_key_helper_generates_0600_key_once`. Justified contract
  migration: AI/BACKLOG.md #144.1 acceptance ("staged tree contains no `*.key` file") and matrix
  PMK1/PMK2. The 0600 / 32-byte properties are still asserted, now on the real key generator.
- `crates/daemon/tests/dispatcher_tests.rs`: two preview tests now build the dispatcher with
  `with_preview_config(preview_allow(current_uid))`. Justified by #143 (PFA1: preview denied by
  default); assertions unchanged, only the setup opts the unprivileged test peer in.
- `tests/distro/fedora_rhel_test.sh` Step 4: the `authselect check || true` tolerance and
  template-only greps were replaced by real activation, `authselect check` (fatal), generated-file
  ordering and nsswitch checks plus rollback — strictly stronger (FAP5 / DV2).
- `tests/docker/test_suite.sh`: `trap cleanup_daemon` → `trap cleanup_all` (adds T10 temp cleanup).
- `run_tests.sh`: header comments only, plus the new `authselect` mode.

Escape hatches: none (the single `tolerance` hit is matrix prose documenting its *removal*).

## 3. Deep Reasoning Audit

### Logic & Architecture
- Conflict integrity: scripted per-branch line-survival check over all files of all seven
  commits → PASS (see §1). ADR order and "Superseded" annotation → PASS. Matrix: PRU1–4, PLC1–3,
  PMC1–5, PFA1–6, PMK1–6, FAP1–5 all present → PASS; one missing `---` separator (MINOR, §4).
- ci.yml job graph: `authselect-profile` `needs: lint`, same `if` as siblings, checkout pinned
  by SHA with `persist-credentials: false`; `ci-success` `needs` includes it and fails on any
  non-success result (skipped included) → PASS.
- Dispatcher interplay (#147 × #143): PreviewFrame is now dispatched at Step 6c, after kernel
  peer verification (Step 6) and before the auth-only deadline/pipeline steps, so it never enters
  the consensus loop nor touches the auth rate limiter; it has its own `preview_limiter` built
  from `PreviewConfig` and rebuilt in `with_preview_config`; `main.rs` wires
  `config.preview` → PASS. Tried: foreign `uid_hint` (rejected at Step 6 and again by
  `authorize_preview`), default config unprivileged peer (Disabled), `max_requests_per_sec = 0`
  (RateLimiter returns Err for `max_attempts == 0`) → all refuse with zero pixel bytes.
- Consensus loop: thresholds come from the same `ThresholdConfig` that sets
  `vision.pad_threshold` (config.rs:307–313), so the aggregator and `OrtPadDetector` agree.
  Spoof → sticky veto + immediate break; stale last capture → `Unavailable/StaleFrame`; one
  `record_attempt` per request after the loop, Allow downgraded on rejection; poll sleep clamped
  to remaining budget → PASS.
- #146 × #147: `build_pad_detector` (daemon + enroll) uses `OrtPadDetector::new` (index 1) and
  the consensus path consumes `pad_result.is_live/score` → PASS.
- #144 × #145 in install.sh/uninstall.sh/soos.spec: key helper installed and invoked on live
  install only; authselect.previous recorded only when `DESTDIR` empty and current profile is not
  `custom/soos*`; uninstall restores before deleting the profile and keeps it if restore fails;
  spec `%files` ghosts both `master.key` and `authselect.previous` → PASS.
- Latency of consensus vs packaged `timeout_ms=250` → MINOR finding (§4).

### PAM Concurrency & Deadlines
- `crates/pam` diff adds only a cfg-gated synchronous hook inside the existing `catch_unwind`
  region, before any socket I/O; no threads/async → PASS. Release profile `panic = "unwind"`
  pinned by invariant; fault-injection feature never referenced by packaging/CI build lines → PASS.

### Panic Safety & Fail-Closed
- Tried: injected panic/overflow via C ABI, PamHooks, `authenticate_with_config` → PAM_IGNORE
  (tests). Preview refusals return standard `Response` (ProtocolError), never Allow. Consensus:
  no path from Pending/SpoofVetoed/rate-limited to Allow. Inference/internal errors inside the loop
  return `Unavailable` (unchanged). `debug_vision` now propagates detector errors → PASS.

### Test Integrity & Anti-Weakening
- §2 listing reviewed; the only weakened-looking assertion is a justified #144 contract migration
  with the invariant re-asserted on the helper. New tests would fail against plausible wrong
  implementations (e.g. single-frame Allow, non-sticky veto, preview served by default, index 2)
  as recorded by the per-branch red evidence → PASS.

### Memory, Bounds & Secrets
- Preview responses and every encoded daemon response wrapped in `Zeroizing`; `PreviewResponse`
  zeroizes on drop; GUI reply bounded by `MAX_PREVIEW_MESSAGE_SIZE`, refusal decoded only when
  `<= MAX_MESSAGE_SIZE` and nonce-bound; `allowed_uids` bounded (64). debug-vision report:
  `O_CREAT|O_EXCL|O_NOFOLLOW`, 0600, parent symlink refused, frame embedded only on opt-in, RGB
  buffer and base64 in `Zeroizing`. Master key: umask 077 temp file + hard-link publish, symlink
  refused, never printed → PASS.

### Supply Chain & Automation
- `Cargo.lock`/crate manifests: only intra-workspace/workspace deps (`ort` dev-dep for enroll
  tests, `getrandom`/`zeroize` already in workspace). New CI job pinned by SHA, inherits
  `permissions: contents: read`, no `${{ github.event.* }}` in `run:` → PASS.

### English-Only Policy
- Scanned added lines for non-ASCII Latin diacritics; none. All docs/comments English → PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `AI/walkthroughs/80_pad_multiframe_consensus.md:74` / `packaging/pam/fedora/soos/system-auth:9` —
  the #147 latency analysis ("170–450 ms, inside DECISION_BUDGET_MS = 900 (sudo, PAM timeout_ms=1000)")
  assumes a 1000 ms PAM deadline, but every packaged stack (Debian pam-configs, Arch, and the Fedora
  authselect profile newly shipped by #145) passes `timeout_ms=250`. With k = 3 captures, the upper
  half of that range exceeds the packaged deadline, so face unlock for sudo/login may fall back to the
  password more often than before (fail-closed, not a security issue). Correct the walkthrough
  statement and track a hardware measurement of consensus latency at `timeout_ms=250` (or an ADR
  revisiting the packaged timeout).
- **[MINOR]** `AI/VERIFICATION_MATRIX.md:457` — merge resolution left no `---` separator between
  the `pad-live-class-index-single-source` and `pad-multiframe-consensus` components (all other
  components are separated). Cosmetic; add the separator.
- **[SUGGESTION]** `crates/daemon/src/dispatcher.rs:684-708` — inference/internal-error early
  returns inside the consensus loop skip `record_attempt`, so such requests are not counted by the
  per-UID rate limiter (pre-existing behavior, unchanged by this diff). Consider recording the
  attempt on every exit path of Step 8.
- **[SUGGESTION]** `crates/daemon/src/dispatcher.rs:917-927` — the preview camera wake loop is
  bounded by `connection_timeout` (max 1 s) rather than a request deadline; acceptable for the
  diagnostic stream, but a single shared constant with the auth wake path would avoid drift.

## 5. Final Verdict

**VERDICT: APPROVED**
