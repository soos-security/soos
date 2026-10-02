# Walkthrough 169 — Daemon Minor Findings of the 2026-10-02 Review

- **Date**: 2026-10-02
- **Issue**: GitHub #315 ([DMN-FU-1002], review findings DMN-NEW-2..5 and two suggestions).
  GitHub-only review finding: the branch is not registered in `BRANCH_TO_ISSUE`; the commit
  carries `Closes #315`.
- **Branch**: `fix/daemon-review-minors`
- **Base commit**: `095eae9`
- **Matrix criteria**: DRM1–DRM8 (new section `daemon-review-minors` right after the DFU section)
- **ADR**: 2026-10-02 "Decaying Inference Estimate, Session Policy Locked to the Mock Camera,
  Shared Bounded Config Reader"

---

## 1. Context and Objectives

The 2026-10-02 full-project review grouped the minor daemon findings in GitHub #315:

- **DMN-NEW-2**: the inference latency estimate (`InferenceEstimator`, EMA clamped to
  `MAX_INFERENCE_ESTIMATE_MS` = 1000 ms) only changes when an inference runs. Once it exceeded
  the 950 ms budget of a 1000 ms PAM request (a slow warm-up pass, one pathological
  measurement), the admission gate refused every inference, nothing lowered the estimate, and
  face authentication stayed disabled until a restart.
- **DMN-NEW-3**: `[dispatcher] enforce_active_session = false` was accepted from the production
  configuration without a word, silently disabling the local-session policy.
- **DMN-NEW-4**: the daemon configuration loader used `std::fs::read_to_string` (unbounded,
  blocks on a FIFO, opens device nodes), checked the system path with `is_file()` before opening
  it (check-then-open race), and silently dropped an unknown `sensor_preference`.
- **DMN-NEW-5 (docs)**: `Docs/DAEMON.md` had two "step 4" items (one stranded in §5) and no
  warm-up step; `AI/ARCHITECTURE.md` said evidence is written only after `PasswordFailed`
  (`PadFailed` too), claimed a `root:soos` check of `/run/soos` that `socket.rs` did not do, and
  Invariant 3 claimed an `/etc/passwd` cross-check that does not exist.
- **Suggestions**: `SocketConfig::validate` did not require `socket_path` to live in
  `socket_dir`; the `sd_notify` module doc over-claimed that an abstract `NOTIFY_SOCKET` is
  reachable under `PrivateNetwork=yes`.

## 2. Architect Design

| Item | Design |
|---|---|
| DMN-NEW-2 | `InferenceEstimator::decay_toward_default(&self)`: atomic `fetch_update`, `new = default + (current - default) / 2` when `current > default`, untouched otherwise. `InferenceGate::decay_estimate(&self)` delegates. In the consensus loop (step 8e-1 of `dispatcher.rs`), when `deadline.can_start(cur_ns, estimate)` is false **and** `aggregator.frames_evaluated() == 0`, the dispatcher decays the estimate before breaking out. Verdict unchanged (`Unavailable` / `Timeout`). |
| DMN-NEW-3 | `DaemonConfig::validate` returns `DaemonError::Config` naming `enforce_active_session` when it is `false` and `pipeline.use_mock_camera` is `false`. |
| DMN-NEW-4 | New `pub fn soos_camera_v4l::daemon_config::read_daemon_config_text(&Path) -> Result<String, DaemonConfigError>` (a wrapper around the existing private `O_PATH` reader; nothing in camera-v4l was modified). `DaemonConfig::load_from_path` and `load_or_default_with_system_path` use it; only `DaemonConfigError::NotFound` at the system path yields the runtime defaults. New field `DaemonConfig::warnings: Vec<String>`; `main.rs` logs each entry with `warn!` after `init_logging`. |
| DMN-NEW-5 | `pub fn socket::validate_directory_group(&File, &Path, expected_gid: u32)` (fstat on the descriptor); `bind_socket` calls it on the locked directory descriptor before binding when `enforce_root_owner` and `socket_group` are set. Docs fixed (§4 renumbered with a warm-up step, architecture claims, `session.rs` header). |
| Suggestions | `SocketConfig::validate` requires the last component of `socket_path` to be a normal file name and `socket_path.parent() == socket_dir` (component-wise, so a trailing slash on `socket_dir` is accepted). `sd_notify.rs` module doc reworded. |

Invariants touched: fail closed per request (the gated request never runs an inference), no
`Allow` path added, socket ownership invariants strengthened, no new log of sensitive data (the
configuration warning names the key, never the value).

## 3. Plan Evaluation

Condensed plan (orchestrator-scoped batch, decisions given in the task): estimate decay chosen
over a warm-up seed cap because the smallest client budget is the 10 ms PAM floor, so no useful
cap exists; the group check was added instead of weakening the documentation because systemd's
`RuntimeDirectory=soos` with the unit's `Group=soos` and every packaging/harness path already
create `/run/soos` as `root:soos` (verified with the systemd acceptance harness, §8). Scope
limits respected: no change to `crates/pam`, `crates/evidence-store`, `crates/biometric-store`,
`crates/enrollment-cli` or camera-v4l internals (one function added); the only `dispatcher.rs`
edit is in the estimate gate. Verdict: approved by the orchestrator's decisions.

## 4. Tester Contract

New file `crates/daemon/tests/daemon_review_minor_tests.rs` (21 tests), one new dispatcher test
in `crates/daemon/tests/inference_budget_tests.rs` (reuses its gated fixture) and three invariant
tests in `tests/invariants/src/daemon_docs_contract.rs`.

| Test | Criterion | Red evidence (before the fix, with compiling stubs) |
|---|---|---|
| `test_drm_estimator_decay_moves_a_saturated_estimate_toward_the_default`, `test_drm_gate_decay_estimate_delegates_to_the_estimator` | DRM1 | `first < previous` failed; `assertion failed: gate.estimate() < Duration::from_millis(MAX_INFERENCE_ESTIMATE_MS)` |
| `test_drm_estimator_decay_never_raises_a_low_estimate` | DRM1 | passed on the no-op stub (guard against a wrong implementation) |
| `inference_budget_tests::test_drm_estimate_seeded_at_max_recovers_for_1000ms_requests` | DRM2 | "a request finalized by the estimate gate with zero frames evaluated must decay the estimate" |
| `test_drm_enforce_active_session_false_is_refused_in_production_config`, `test_drm_validate_refuses_disabled_session_policy_without_mock_camera` | DRM3 | "the configuration must be refused"; "validate() must refuse it, got Ok(())" |
| `test_drm_enforce_active_session_false_is_accepted_with_mock_camera`, `test_drm_enforce_active_session_true_is_always_accepted` | DRM3 | passed (non-regression guards) |
| `test_drm_load_from_path_refuses_a_fifo_promptly` | DRM4 | "loading a FIFO must return promptly instead of blocking" (5 s timeout, `read_to_string` blocked in `open(2)`) |
| `test_drm_system_config_fifo_is_refused_promptly` | DRM4 | "a FIFO at the system path must be refused, not silently replaced by defaults: Ok(DaemonConfig { .. })" |
| `test_drm_load_from_path_refuses_a_file_above_one_mib` | DRM4 | "a file above 1 MiB must be refused, got Ok(..)" |
| `test_drm_system_config_directory_is_refused` | DRM4 | "a non-regular system configuration path must be refused (no check-then-open)" |
| `test_drm_config_loader_has_no_unbounded_read_or_is_file_check` | DRM4 | failed on `read_to_string(` |
| `test_drm_load_from_path_accepts_a_file_at_the_limit_boundary` | DRM4 | passed (non-regression) |
| `test_drm_unknown_sensor_preference_is_reported_as_a_load_warning` | DRM5 | "assertion `left == right` failed: [] left: 0 right: 1" |
| `test_drm_main_logs_config_warnings_after_logging_init` | DRM5 | "main.rs must log the configuration load warnings" |
| `test_drm_known_sensor_preference_has_no_warning` | DRM5 | passed (non-regression) |
| `test_drm_socket_path_must_be_a_direct_child_of_socket_dir`, `test_drm_socket_path_outside_socket_dir_is_refused_at_load` | DRM6 | both refused paths were accepted (`Ok(())`) |
| `test_drm_socket_path_inside_socket_dir_is_accepted` | DRM6 | passed (non-regression) |
| `test_drm_socket_directory_group_must_match_the_socket_group` | DRM7 | "a directory of another group must be refused, got Ok(())" |
| `test_drm_bind_socket_checks_the_directory_group_when_root_owned_is_enforced` | DRM7 | "bind_socket must check the socket directory group" |
| `daemon_docs_contract::test_daemon_doc_startup_steps_are_sequential_and_include_warmup` | DRM8 | "§4 must list configuration, swap protection, pipeline, warm-up and bind" |
| `daemon_docs_contract::test_architecture_doc_matches_daemon_code_claims` | DRM8 | "AI/ARCHITECTURE.md must not say evidence is written only after PasswordFailed" |
| `daemon_docs_contract::test_sd_notify_doc_does_not_overclaim_private_network_reachability` | DRM8 | "the sd_notify module doc over-claims PrivateNetwork reachability" |

**Migrated existing tests**: none. No existing assertion was changed; the existing
`inference_budget_tests` (`test_159_*`, `test_158_*`), `warmup_default_tests`, `config_tests`,
`config_validation_tests` and `socket_tests` pass unchanged.

**Flakiness check**: the 22 `test_drm` daemon tests (including the dispatcher recovery test)
were run 10 times in a row: 10/10 green.

## 5. Auditor Constraints

1. No panic path added: every new production function returns `Result` or is infallible
   (`fetch_update` with saturating arithmetic, `clamp_estimate_micros`); no `unwrap`/`expect`.
2. Bounded I/O: the configuration read is bounded to 1 MiB before parsing; a FIFO/device node is
   never opened for reading (`O_PATH` pin + `fstat`), the read uses `O_NONBLOCK`.
3. No check-then-open: the system path is opened once; the error kind decides.
4. Fail closed: the estimate decay never changes the verdict of the request that triggers it;
   any read failure other than `NotFound` stops the daemon start; the group mismatch stops the
   bind.
5. No sensitive data logged: the configuration warning names the key, never the value; the
   group error reports numeric GIDs and the directory path only.
6. `unsafe`: none added (`crates/daemon` keeps its existing policy; `fstat` via `nix`).
7. Scope: camera-v4l change is an added public wrapper only; no other crate owned by another
   batch is touched; the `PasswordFailed` evidence code in `dispatcher.rs` is untouched.

## 6. Implementation

- `crates/daemon/src/inference.rs`: `InferenceEstimator::decay_toward_default`,
  `InferenceGate::decay_estimate`.
- `crates/daemon/src/dispatcher.rs`: decay call in the estimate admission gate (8e-1) when no
  capture was evaluated.
- `crates/daemon/src/config.rs`: `DaemonConfig::warnings`, `config_read_error`, bounded loader in
  `load_from_path` / `load_or_default_with_system_path`, `enforce_active_session` rule in
  `DaemonConfig::validate`, `socket_path` parent rule in `SocketConfig::validate`, unknown
  `sensor_preference` warning.
- `crates/daemon/src/main.rs`: logs `config.warnings` at `warn` after `init_logging`.
- `crates/daemon/src/socket.rs`: `validate_directory_group`, called by `bind_socket` (step 2b).
- `crates/daemon/src/sd_notify.rs`, `crates/daemon/src/session.rs`: module docs corrected.
- `crates/camera-v4l/src/daemon_config.rs`: `read_daemon_config_text` (added).
- Docs: `Docs/DAEMON.md` (§1 load order and reader, §1.2/§1.3/§1.4 rows, §4 renumbered with the
  warm-up step and `READY=1`, stray step removed from §5), `Docs/CAMERA_V4L_CRATE.md` (the daemon now uses the shared reader), `AI/ARCHITECTURE.md` (Invariant 3,
  evidence line, `/run/soos` paragraph), `AI/DECISIONS.md` (new ADR), `AI/VERIFICATION_MATRIX.md`
  (DRM1–DRM8).

## 7. Candid Review

Not run in this batch (the orchestrator runs the layer-2 candid sub-agent and `save.sh`).
Layer 1 (`./scripts/candid_review.sh`) was run locally; see §8.

## 8. Verification Results

```bash
export CARGO_BUILD_JOBS=5
cargo fmt --all -- --check
cargo clippy --locked -p soos-daemon -p soos-camera-v4l -p soos-invariants --all-targets --all-features -- -D warnings
cargo test --locked --all-features -p soos-daemon -p soos-camera-v4l -p soos-invariants   # SOOS_MODELS_DIR set
ORT_SKIP_DOWNLOAD=1 cargo check --locked --target i686-unknown-linux-gnu -p soos-daemon -p soos-camera-v4l -p soos-invariants --all-targets --all-features
./tests/docker/systemd_unit_acceptance_test.sh --models host
./scripts/candid_review.sh
```

- fmt: clean; clippy: no warning.
- Tests: 967 passed, 0 failed, 5 ignored (real-model tests ran with `SOOS_MODELS_DIR`).
- i686 type check: clean.
- systemd unit acceptance (real systemd PID 1, `RuntimeDirectory=soos`, `Group=soos`): all parts
  passed with the new directory-group check (READY=1, socket `0660 root:soos`, clean stop).
- Candid layer 1: passed.

## 9. Known Limitations / Follow-ups

- A peer sending `Auth` requests with tiny client budgets can pull the estimate down to the
  default; a genuinely slow host may then start an inference that overruns its own request
  deadline. That request times out (fail closed) and its measurement restores the estimate.
- `--mock-camera` does not make a file with `enforce_active_session = false` acceptable (the
  flag is applied after validation); harnesses must set `use_mock_camera = true` in the file.
- A directory, FIFO or device node at `/etc/soos/daemon.toml` now stops the daemon start instead
  of falling back to the defaults (fail closed, documented in `Docs/DAEMON.md` §1).
