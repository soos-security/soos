# Walkthrough 114 — Daemon Session Policy Hardening and Follow-ups

- **Date**: 2026-09-30
- **Issue**: GitHub #276 `[DMN-FU]` (follow-ups of the approved candid review of the P1 daemon/PAM
  batch #157–#160, #173–#176) — **Branch**: `fix/p2-daemon-session-followups`
- **Matrix criteria**: DSF1–DSF4 and DSF6–DSF10 (✅ Verified), DSF5 (hardware check, pending)
- **ADR**: 2026-09-30 "Session Policy Hardening Follow-ups" in `AI/DECISIONS.md` (amends
  "Local Session Binding for Facial `Auth`")

---

## 1. Context & Objectives

The review of the P1 batch left six non-exploitable, fail-closed hardening and documentation
items. Each one is addressed here:

| Item | Change |
|---|---|
| `sessions()` skipped unreadable directory entries | `collect_session_records` returns an error: any I/O error denies |
| Accepted-risk wording | ADR amendment and `Docs/PAM_MODULE.md` item 5 name remote access without a logind session reaching `user@<uid>.service` through the user bus |
| `parse_session_id_from_cgroup` read every hierarchy | Only `0::` and exact `name=systemd` lines, shared helper with the user-manager parser |
| polkit ≥ 126 hardware note | Recorded in the ADR and `Docs/PAM_MODULE.md`; characterized by a test; real-host check DSF5 |
| First `Auth` used the 80 ms default estimate | Start-up warm-up seeds the inference estimate |
| Evidence store per-UID cap only, unbounded counter map | Global daily cap and single-day counters |

## 2. Specification

### 2.1 Session policy (`crates/daemon/src/session_policy.rs`)

- `pub fn collect_session_records<I>(entries: I) -> Result<Vec<SessionRecord>, LogindError>`
  with `I: IntoIterator<Item = io::Result<(OsString, PathBuf)>>`. An `Err` entry is a
  `LogindError` (the policy maps it to `logind_unavailable`); invalid session-ID names are
  skipped, a vanished or non-regular file is skipped, more than `MAX_SCANNED_SESSIONS` entries
  is an error. `SystemLogind::sessions()` maps `read_dir` into this function.
- `fn systemd_hierarchy_path(line) -> Option<&str>`: the path of a `0::` line or of a line whose
  controller list is exactly `name=systemd`; used by both cgroup parsers.

### 2.2 Inference warm-up (`crates/daemon/src/inference.rs`, `pipeline.rs`, `main.rs`)

- `WARMUP_PASSES = 2`; `InferenceEstimator::seed(Duration)` replaces the estimate (clamped to
  1–1000 ms).
- `InferenceGate::warm_up(job) -> Result<Duration, InferenceJobError>`: acquires the slot, runs
  the job `WARMUP_PASSES` times on the blocking pool, seeds with the last pass. A panic returns
  `Panicked` and leaves the default estimate.
- `pipeline::warm_up_vision_stages(vision, w, h)`: detector on a blank `w x h` RGB frame (sides
  clamped to `1..=MAX_WARMUP_DIMENSION` = 1920), PAD and embedding on blank crops of the
  configured sizes. A blank frame has no face, so `process_frame` would stop after detection;
  the stages are called directly. Results and errors are dropped.
- `pipeline::warmed_inference_gate(vision, w, h)`: builds the default gate, warms it, logs the
  measured and seeded estimate (milliseconds only). `soos-daemon` calls it with the configured
  camera size before binding the socket and passes the gate with `with_inference_gate`.

### 2.3 Evidence store (`crates/evidence-store`)

- `DEFAULT_DAILY_CAP_TOTAL = 100`; `EvidenceStore::with_daily_cap_total`, `daily_cap_total`,
  `daily_total(date)`, `tracked_daily_counters()`.
- `EvidenceStoreError::GlobalDailyCapExceeded { cap, date }` (no UID in the message).
- Counters are a single-day structure (`date`, per-UID map, total). A store for another date
  resets them; a per-UID entry is only inserted once both caps passed, so the map never exceeds
  the global cap.
- Daemon: `PipelineConfig::evidence_daily_cap_total` (TOML `[pipeline.evidence]
  daily_cap_total`), applied in `initialize_pipeline`. It is not an `EvidenceConfig` field so
  that existing struct literals in pre-existing tests keep compiling unchanged.

## 3. Tests (written first)

- `crates/daemon/tests/session_followups_tests.rs` — DSF1, DSF2, DSF4.
- `crates/daemon/tests/inference_warmup_tests.rs` — DSF6, DSF7.
- `crates/evidence-store/tests/global_daily_cap_tests.rs` — DSF8, DSF9.
- `crates/daemon/tests/evidence_global_cap_config_tests.rs` — DSF10.

Red evidence: every new file failed to compile against `origin/main` (`unresolved import
collect_session_records`, `WARMUP_PASSES`, `warm_up_vision_stages`, `DEFAULT_DAILY_CAP_TOTAL`;
`no method named seed` / `warm_up` / `with_daily_cap_total` / `tracked_daily_counters`; `no
variant named GlobalDailyCapExceeded`; `no field evidence_daily_cap_total`). With only
`collect_session_records` implemented, `test_cgroup_session_parser_ignores_non_systemd_controller_lines`
failed with `left: Some("3") right: None`. The polkit test (DSF4) is a characterization test: it
passes before and after, documenting the password fallback. No pre-existing test was modified.

## 4. Audit

- No `unwrap`/`expect` in production code; the blank-buffer length uses `try_from` with
  saturating multiplication on sides already clamped to 1920.
- Warm-up inputs are zero buffers; no frame, embedding or score is logged (only durations in ms).
- Warm-up runs before the socket is bound, holds the single inference slot and cannot turn into
  an `Allow`: it never touches the policy, the store or a request.
- Every new error path in the session policy denies; no lookup error reaches `Ok(())`.
- The evidence counters stay bounded (≤ global cap entries) and the cap is checked before any
  filesystem side effect.

## 5. Gate

See the branch report: `cargo fmt --all -- --check`, `cargo clippy --locked --workspace
--all-targets --all-features -- -D warnings`, `cargo test --locked --workspace --all-targets
--all-features --no-fail-fast`, `./scripts/candid_review.sh`.

## 6. Residual Risks & Follow-ups

- **DSF5 / LSB8 (hardware)**: confirm on a host with polkit ≥ 126 that polkit prompts fall back to
  the password, and the GDM / locker behaviour already listed in walkthrough 97.
- **Remote access without a logind session** can still reach the target's user manager; accepted
  and documented. `SO_PEERPIDFD` and a logind D-Bus query remain the long-term options.
- **Warm-up on real models**: the seeded estimate has only been measured with mocks; the first
  real measurement is logged at start-up (`Vision inference warm-up complete`).
- A clock stepped back across midnight resets the evidence day budget (root-only control).

## 7. Note: Default Connection Timeout Raised to 2500 ms (User Decision 2026-09-30)

The inference warm-up above seeds the admission estimate, but the daemon request budget was
still capped by the 1000 ms default `connection_timeout`, so the GDM line `timeout_ms=2500`
never received more than about 950 ms of daemon time. `DispatcherConfig::default()` now uses
`DEFAULT_CONNECTION_TIMEOUT_MS` = 2500 (`crates/daemon/src/config.rs`); `Docs/DAEMON.md`,
`Docs/IPC_PROTOCOL.md` and the ADR "Daemon Default Connection Timeout Raised to 2500 ms for
the GDM Deadline" record it. Console/sudo stacks keep the 1000 ms PAM module default, whose
client deadline still caps the daemon. New contract: `connection_timeout_default_tests`
(default value, every load path, GDM line coverage, PAM default below it). The pre-existing
assertion of `config_tests::test_config_defaults_when_file_absent` (1000 ms) is left for a
user-approved test change.
