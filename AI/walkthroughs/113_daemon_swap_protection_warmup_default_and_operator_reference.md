# Walkthrough 113 — Daemon Swap-Protection Reporting, Single Warm-Up Default and Operator Reference

- Branch: `fix/p2-daemon-mlock-warmup-docs`
- Issues: GitHub #201 (DMN-12), #205 (DMN-16), #206 (DMN-17), from `AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`
- Matrix: component `daemon-mlock-warmup-docs`, rows DMW1–DMW7
- ADRs (`AI/DECISIONS.md`, 2026-09-30): "Swap Protection Is `mlockall` Only", "One Effective Daemon `warmup_frames` Default", "Daemon Operator Reference"

## 1. Problems

1. **DMN-12** — `LockedBuffer` / `mlock_slice` were never used in production, yet
   `Docs/MEMORY_PROTECTION_AND_SWAP.md` presented them as "Layer 2" and showed
   `LockedBuffer::new(master_key_bytes)`. When `mlockall` failed, `main.rs` logged at `debug`
   that it was "continuing with granular buffer protection", which did not exist.
2. **DMN-16** — `from_toml_str` forced `warmup_frames = 0` whenever a `[pipeline]` table existed,
   while `load_or_default` without a file returned `DaemonConfig::default()` (camera library
   default 20). The same binary discarded 0 or 20 frames depending on whether a config file existed.
3. **DMN-17** — no document listed the `daemon.toml` keys; `Docs/IPC_PROTOCOL.md` lacked the
   `StatusResponse` / `PreviewResponse` schemas, `MAX_PREVIEW_MESSAGE_SIZE` and the persistent loop.
   (The `AI/ARCHITECTURE.md` and `AI/DECISIONS.md` statements the review quoted had already been
   corrected on `main` before this branch.)

## 2. Design

- **Swap protection** (`crates/daemon/src/mlock.rs`): `enable_swap_protection(&HealthState) -> bool`
  calls `mlockall` and `record_swap_protection(locked, &HealthState)`, which stores the outcome in
  the new `HealthState::memory_locked` flag and logs `info` on success, `warn` on refusal.
  `main.rs` creates `HealthState` first and calls it. `LockedBuffer` is kept (its hardening tests are
  contracts) and documented as "available, not wired": the keys, templates and embeddings live in
  `#![forbid(unsafe_code)]` crates. The flag does not change `is_healthy`.
- **Warm-up** (`crates/daemon/src/config.rs`): `DAEMON_DEFAULT_WARMUP_FRAMES = 0`,
  `DaemonConfig::runtime_default()` (= `default()` with that value), `DEFAULT_CONFIG_PATH`, and
  `load_or_default_with_system_path(path_opt, system_path)` for a hermetic test. `from_toml_str` now
  starts from `runtime_default()` and applies `warmup_frames` only when the key is present.
  `DaemonConfig::default()` keeps 20 because an existing test pins it (see §5).
- **Docs**: new `Docs/DAEMON.md` (every key with default and validation, connection model,
  authorization per request kind, startup order); `Docs/IPC_PROTOCOL.md` gains the request-kind
  table, the two response schemas, `MAX_PREVIEW_MESSAGE_SIZE` and a "Persistent connection loop"
  subsection; `Docs/MEMORY_PROTECTION_AND_SWAP.md` and `Docs/CAMERA_V4L_CRATE.md` corrected.
- **Invariant** `tests/invariants/src/daemon_docs_contract.rs` parses the private `*ConfigFile`
  structs of `config.rs` and requires each leaf key in `Docs/DAEMON.md`; a new struct that is not
  mapped to a TOML table fails the build. It also checks the IPC and memory-protection documents.

## 3. Audit

- No `unwrap`/`expect` added to production code; no PAM code touched; no `unsafe` added
  (`enable_swap_protection` reuses the documented `mlockall` wrapper).
- The warning names only `mlockall`, `CAP_IPC_LOCK` and `RLIMIT_MEMLOCK`; no key, frame or
  embedding content is logged (`logging_audit_test` still passes).
- The config change only alters a frame-discard count; no authorization default changes.

## 4. Red → green

- Before the implementation, `cargo test -p soos-daemon --test swap_protection_tests --test warmup_default_tests`
  did not compile: `enable_swap_protection`, `record_swap_protection`, `HealthState::memory_locked`,
  `DAEMON_DEFAULT_WARMUP_FRAMES`, `DaemonConfig::runtime_default` and
  `load_or_default_with_system_path` did not exist. On the old code `from_toml_str("")` gave 20 and
  `from_toml_str("[pipeline]\n")` gave 0.
- `cargo test -p soos-invariants daemon_docs_contract`: 4 failed (no `Docs/DAEMON.md`, no
  `MAX_PREVIEW_MESSAGE_SIZE` in the IPC spec, `LockedBuffer::new(master_key_bytes)` still in the
  memory-protection document).
- After: all the new tests pass, and so do the existing `config_tests`, `hardening_tests`,
  `health_tests` and `logging_audit_test`.

## 5. Open items

- **Status wire field**: `memory_locked` is not in `StatusResponse` yet. Adding it changes the
  Postcard layout and needs changes to existing tests that build `StatusResponse` literals
  (`crates/admin-cli/tests/status_tests.rs`, `crates/protocol/tests/property_tests.rs`); left for a
  user decision (DMW3).
- **`DaemonConfig::default()`** still reports 20 because
  `config_tests::test_pipeline_default_sensor_preference_is_prefer_ir` asserts it. The running daemon
  never uses that value; aligning it needs an approved test change.
- **Time-based warm-up** (discard frames younger than N ms after resume) needs hardware measurement.
- **`socket_mode`** from `daemon.toml` is applied without rejecting world bits (seen while writing
  `Docs/DAEMON.md`, out of scope here).
