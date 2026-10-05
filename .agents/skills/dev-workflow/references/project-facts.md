# soos — Shared Project Facts for Agent Skills

Single reference consumed by every skill under `.agents/skills/`. It records facts that
drifted in the past (see walkthroughs cited in brackets) so that agents stop re-deriving
them from stale prose.

**Precedence rule**: when prose (`AGENTS.md`, `AI/ARCHITECTURE.md`, a walkthrough) and code
disagree, the **code constant is the current truth** and the prose is drift. Do not silently
"fix" the code to match stale prose — report the drift in your deliverable and propose an
ADR entry in `AI/DECISIONS.md`. Re-check every value below with the listed `grep` before relying on it.

---

## 1. Workspace Map

| Crate dir | Package | Kind | `unsafe` policy |
|---|---|---|---|
| `crates/protocol` | `soos-protocol` | lib | `#![forbid(unsafe_code)]` |
| `crates/policy` | `soos-policy` | lib, zero I/O, clockless | `#![forbid(unsafe_code)]` |
| `crates/vision` | `soos-vision` | lib | `#![forbid(unsafe_code)]` |
| `crates/inference-ort` | `soos-inference-ort` | lib (ORT CPU) | `#![forbid(unsafe_code)]` |
| `crates/biometric-store` | `soos-biometric-store` | lib | `#![forbid(unsafe_code)]` |
| `crates/evidence-store` | `soos-evidence-store` | lib, zero network | `#![forbid(unsafe_code)]` |
| `crates/enrollment-cli` | `soos-enrollment-cli` (`soos-enroll`) | lib + bin | `#![forbid(unsafe_code)]` |
| `crates/admin-cli` | `soos-admin-cli` (`soos-admin`) | lib + bin | `#![forbid(unsafe_code)]` |
| `crates/gui` | `soos-gui` | lib + bin (eframe/glow) | `#![forbid(unsafe_code)]` |
| `crates/remote` | `soos-remote` | lib + bin (current-thread Tokio, user-level, Unix socket only, zbus; leaf crate, GitHub #339) | `#![forbid(unsafe_code)]` |
| `crates/pam` | `soos-pam` → `libpam_soos.so` | cdylib + rlib | adapter: `unsafe` allowed, `// SAFETY:` mandatory |
| `crates/camera-v4l` | `soos-camera-v4l` | lib, `mock-camera` feature | adapter: `unsafe` allowed, `// SAFETY:` mandatory |
| `crates/daemon` | `soos-daemon` | lib + bin (Tokio) | `main.rs` forbids; `lib.rs` only denies undocumented unsafe (`mlock.rs`) |
| `tests/invariants` | `soos-invariants` | static repo checks | — |
| `tests/fixtures` | `soos-test-fixtures` | dev-only fixture lib (`[lib] path = "mod.rs"`), never a normal dependency [129] | — |

- The authoritative forbid list is `test_business_crates_forbid_unsafe_code` in
  `tests/invariants/src/lib.rs` (10 crates, `remote` included since GitHub #339). `AGENTS.md` lists only 3 — that is a minimum, not the full set.
- Workspace-wide lints live in root `Cargo.toml` (`[workspace.lints]`); every crate declares
  `[lints] workspace = true` and `publish.workspace = true` (required by `deny.toml` private-crate exemption).
- `cargo -p` takes the **package** name (`-p soos-pam`, never `-p pam`) [11].

## 2. Security & Latency Constants (verify: `grep -rn "pub const" crates/*/src`)

| Constant | Location | Value (2026-09-29) |
|---|---|---|
| `MAX_MESSAGE_SIZE` | `crates/protocol/src/types.rs` | 4096 bytes |
| `MAX_PREVIEW_MESSAGE_SIZE` | `crates/protocol/src/types.rs` | 2 MiB (preview frames only) |
| `MAX_SERVICE_LEN` | `crates/protocol/src/types.rs` | 64 bytes |
| `DEFAULT_TIMEOUT_MS` / `MIN_` / `MAX_` | `crates/pam/src/config.rs` | 1000 / 10 / 5000 ms (`timeout_ms=` is clamped) |
| `EVENT_TIMEOUT_MS` | `crates/pam/src/ipc.rs` | 20 ms (password-failed event) |
| `MAX_RESPONSE_FUTURE_SKEW_NS` | `crates/protocol/src/types.rs` (re-exported by `crates/pam/src/ipc.rs`) | 10 ms (tolerated lead of `Response::issued_monotonic_ns`; the PAM client enforces the response expiry, test daemons must stamp from CLOCK_MONOTONIC [148]) |
| `RESPONSE_VALIDITY_NS` | `crates/daemon/src/dispatcher.rs` | 2 s (`expires = issued + 2 s`) |
| `DECISION_BUDGET_MS` | `crates/daemon/src/pipeline.rs` | 900 ms |
| `MAX_FRAME_AGE_NS` | `crates/daemon/src/pipeline.rs` | 150 ms |
| `FRAME_POLL_INTERVAL_MS` | `crates/daemon/src/pipeline.rs` | 10 ms (consensus loop poll) |
| `MAX_CONCURRENT_INFERENCES` | `crates/daemon/src/inference.rs` | 1 (blocking-pool inference slots) |
| `RESPONSE_WRITE_MARGIN_MS` | `crates/daemon/src/inference.rs` | 50 ms (reserved before client and outer deadlines) |
| `DEFAULT_INFERENCE_ESTIMATE_MS` / `MAX_INFERENCE_ESTIMATE_MS` | `crates/daemon/src/inference.rs` | 80 / 1000 ms (initial / upper bound of the EMA admission estimate) |
| `WARMUP_PASSES` | `crates/daemon/src/inference.rs` | 2 (start-up warm-up passes; the estimate is seeded with the last one) |
| `DEFAULT_DAILY_CAP_TOTAL` | `crates/evidence-store/src/config.rs` | 100 evidence snapshots per day across all UIDs (`[pipeline.evidence] daily_cap_total`) |
| `DEFAULT_PAD_CONSENSUS_REQUIRED` / `_WINDOW` / `MAX_PAD_CONSENSUS_WINDOW` | `crates/policy/src/pad_consensus.rs` | 3 / 5 / 32 captures (Allow needs 3 consecutive; any spoof vetoes the request) |
| `RateLimitConfig::DEFAULT_MAX_ATTEMPTS` / `DEFAULT_WINDOW_DURATION_NS` | `crates/policy/src/rate_limit.rs` | 40 attempts / 60 s per UID, shared by every face request (`sudo` included) and presence scans (was 5 before GitHub #323 [175]) |
| `PRESENCE_RESERVED_ATTEMPTS` | `crates/daemon/src/presence/mod.rs` | 5 (presence never consumes the last 5 attempts of a window) |
| `[presence]` `scan_interval_ms` / `lock_grace_ms` | `crates/daemon/src/presence/config.rs` | 2000 / 3000 ms (each `1000..=60000`); presence auto-unlock is enabled by default [175] |
| `GDM_PAM_LINE` | `crates/admin-cli/src/gdm.rs` | `auth  [success=done default=ignore]  pam_soos.so timeout_ms=2500`, inside a managed block after every pre-credential gate [98]; omitted (and an existing block removed) when the delegated stack already reaches a primary `pam_soos.so` rule, GitHub #331 |
| `MAX_PAM_STACK_READS` | `crates/admin-cli/src/pam_stack.rs` | 32 stack files opened per `gdm enable` / `gdm status` analysis (on top of `MAX_PAM_INCLUDE_DEPTH` = 4; the 33rd open refuses), GitHub #333 |
| `PRESENCE_WAKE_SETTLE_MS` | `crates/daemon/src/presence/mod.rs` | 1000 ms (presence-only settle; bound keyed on the scan wake and the camera stream start, GitHub #329/#331) |
| `SYSTEMCTL_SHOW_TIMEOUT_MS` | `crates/admin-cli/src/status.rs` | 1000 ms (`soos-admin status` kills and reaps a slow `systemctl show`, fields `unknown`) |
| admin-cli `DEFAULT_TIMEOUT_MS` | `crates/admin-cli/src/args.rs` | 250 ms |
| Match / PAD thresholds | `crates/vision/src/pipeline.rs`, policy | 0.50 / 0.85 (match default 0.50 since the SFace switch, GitHub #278 [162]; floor `MIN_MATCH_THRESHOLD` 0.40) |
| `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` | `crates/inference-ort/src/pad.rs` | 1 |
| `soos-remote` bounds | `crates/remote/src/lib.rs` | `MAX_CONNECTIONS` 16, `MAX_SSE_STREAMS` 4, `MAX_REQUEST_HEAD_BYTES` 8192, `MAX_HEADERS` 32, `MAX_PATH_LEN` 256, `REQUEST_HEAD_TIMEOUT_MS` 5000, `RESPONSE_WRITE_TIMEOUT_MS` 2000, `SSE_KEEPALIVE_MS` 15 000, `MAX_SSE_STREAM_MS` 1 800 000, `MIN_LOCK_INTERVAL_MS` 2000, `SNAPSHOT_DEADLINE_MS` 1500, `LOCK_FLOW_DEADLINE_MS` 2000, `DEFAULT_POLL_INTERVAL_MS` 1000 (250..=10 000), `MAX_ALLOWED_LOGINS` 8, `MAX_ALLOWED_HOSTS` 4, `EXIT_CONFIG` 78 / `EXIT_RUNTIME` 1 (GitHub #339; independent of the daemon presence constants of the same names) |

The fixed "200 to 250 ms" PAM deadline wording was removed from the normative documents (ADR 2026-09-30
"PAM Deadline Derived From Clamped `timeout_ms`", enforced by `tests/invariants/src/pam_deadline_contract.rs`).
The enforced invariant is **"every blocking PAM operation has an explicit deadline derived from the
clamped `timeout_ms`; nothing is ever unbounded"**, not the literal 250ms figure. The MiniFASNet live
class index is 1 everywhere since ADR 2026-09-29 (code, `AI/ARCHITECTURE.md`, matrix ASG1/PLC1–PLC3);
production never overrides it — `OrtPadDetector::new` only, enforced by the invariant
`test_no_pad_live_class_index_override_outside_tests` [79].

## 3. Runtime Paths & Modes

| Path | Mode | Owner |
|---|---|---|
| `/run/soos/` | 0750 | `root:soos` |
| `/run/soos/daemon.sock` | 0660 | `root:soos` (never 0666, never abstract namespace) |
| `/var/lib/soos/` | 0755 | `root:root` |
| `/var/lib/soos/biometrics/`, `/var/lib/soos/evidence/` | 0700 | `root:root` |
| `/var/lib/soos/models/` | 0755 (files 0644) | `root:root` |
| `/var/lib/soos/master.key` | 0600 | `root:root` |
| `/etc/soos/disabled`, `/etc/soos/gdm.disable`, `/etc/soos/<service>.disable` | flag files | PAM returns `PAM_IGNORE` immediately; the service is `service=` or else the `PAM_SERVICE` item [94] |
| `/etc/soos/presence.disable` (and the global `/etc/soos/disabled`) | flag files | presence auto-unlock stops within one tick (1 s), no restart; `gdm.disable` does **not** stop it [175] |

## 4. Model Contract (manifest `models/manifest.toml` v2.0.0)

| ID | File | Input | Normalization | Notes |
|---|---|---|---|---|
| `scrfd_500m_kps` | `scrfd_500m_kps.onnx` | 640×640 BGR, letterbox | `(x-127.5)/128` | 9 outputs (3 strides × score/bbox/kps); **scores are already sigmoided** [65] |
| `minifasnet_v2_pad` | `minifasnet_v2_80x80.onnx` | 80×80 BGR, 2.7× expanded bbox | raw `x` in `[0, 255]` (no `/255`, upstream `to_tensor`; ADR 2026-10-01 "PAD Input Range Matches Upstream (0-255)" [161]) | live class index **1** [72, 79]; `[PrintPhoto, Live, ScreenReplay]`, never overridden in production |
| `sface_2021dec` | `sface_2021dec.onnx` | 112×112 aligned, **NCHW** `data` `[1,3,112,112]`, fed **RGB** [162] | raw `x` in `[0, 255]` (the graph applies `(x-127.5)/128`; OpenCV `FaceRecognizerSF` recipe) | OpenCV Zoo **SFace 2021dec**, Apache-2.0, 38.7 MB, training data undocumented (owner-accepted); output `fc1` `[1,128]`, L2-normalized by the extractor; `SHIPPED_EMBEDDING_MODEL` (`crates/inference-ort/src/embedding.rs`) is the single source of the id and dimension; templates bind by id **and** dimension, no alias (`mobilefacenet` is Foreign); its ORT session logs at `Error` (`ERROR_ONLY_LOG_MODELS`, 174 initializer-as-input warnings); replaced the ArcFace ResNet34 `arcface_w600k_mbf` (NHWC, BGR `/127.5`, 512D, NOASSERTION), now only in `models/retired_models.toml` (evaluation only) [102, 145, 160, 162] |

Any change to channel order, layout, class index or normalization MUST be validated against the real
`.onnx` metadata (input/output shapes) — mocks alone hid four shipped bugs [65, 66, 68, 71].

`input_shape` is the logical `[N, C, H, W]` shape; `input_layout` (default `"NCHW"`; `"NHWC"` only for the
retired ArcFace entry) is the physical layout. `ModelRegistry::get_or_load_session` validates every
session's I/O shapes against the manifest (symbolic dims are wildcards) and fails closed with
`InferenceError::ModelShapeMismatch` [102]; an entry without `input_layout` (older installed manifest)
leaves the layout unspecified (either order accepted, dims still enforced, one-time warning). Never rename manifest ids or file names to "fix" a
description: `soos-daemon` and `soos-enroll` reference them. Real-model evidence targets:
`crates/inference-ort/tests/pad_real_model_tests.rs` [87] and `embedding_real_model_tests.rs` [102]
(skip cleanly without `/var/lib/soos/models`). MJPEG frames are bounded (`MAX_MJPEG_COMPRESSED_BYTES`
16 MiB, `MAX_MJPEG_DIMENSION` 4096) and the SOF header must equal the negotiated frame size [102].

## 5. Quality Commands (identical locally and in CI)

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test   --locked --workspace --all-targets --all-features
cargo deny --locked check                      # cargo-deny >= 0.20
./scripts/candid_review.sh                     # Layer 1 deterministic invariants
./scripts/candid_subagent.sh --prepare         # Layer 2: diff + fingerprint for the reviewer
./scripts/candid_subagent.sh                   # Layer 2 gate (fresh, fingerprint-bound report)
./run_tests.sh                                 # Dockerized PAM matrix T1–T15 (ubuntu)
```

Toolchain: `rust-toolchain.toml` pins Rust `1.98.1` (components `clippy`, `rustfmt`; user decision
2026-09-30). The sandbox Dockerfiles install the same release (`--default-toolchain 1.98.1`); bump
both together, never back to a floating `stable` channel.
The images install rustup only through `scripts/install_rustup.sh` (pinned rustup release, committed
`rustup-init` SHA-256 digests, never `curl | sh`) [139]; a rustup bump changes the version and both digests.

Omitting `--all-features` locally was the root cause of several CI-only failures [65–75].
`cargo test --all-targets` does not run doctests; do not rely on doctests as acceptance evidence.

Release profile: `[profile.release]` in the root `Cargo.toml` MUST keep `panic = "unwind"`
(ADR 2026-09-29, review PAM-01 [78]). `cargo test` runs under `[profile.test]` and can never detect
an aborting release profile; only Docker case T10 (release-built `.so` + opt-in `fault-injection`
feature of `soos-pam`) proves `catch_unwind` on the shipped artifact. Never enable that feature in
packaging, install or CI build commands.

## 6. Traceability Conventions

- Walkthroughs: `AI/walkthroughs/NN_snake_case.md`, next number = highest existing + 1
  (`ls AI/walkthroughs | sort -n | tail -1`). Numbers 11 and 54 are duplicated historically — never reuse a number.
- Verification matrix status marker for new rows: `✅ Verified` (older rows use `☑ Validated`; do not rewrite them).
  Every backticked citation of a claimed row (`module::test_*`, `test_prefix_*`, `module::*`, repo
  paths, bare `*.rs`/`*.sh` names) must resolve — enforced by
  `soos-invariants::matrix_citations::test_matrix_claimed_rows_cite_only_existing_evidence` [104].
  Put historical names ("renamed from", "never existed") inside an italic `*( ... )*` annotation;
  a row without evidence is `⬜ Pending (<reason>)`, a replaced row is `⏹ Superseded (<rows>)`.
- Branch prefixes allowed by `AGENTS.md`: `feat/`, `fix/`, `test/`, `chore/` (not `refactor/` or `docs/`).
  `python3 scripts/sync_issue.py --check` rejects any other prefix in `BRANCH_TO_ISSUE` except the frozen,
  merged `LEGACY_BRANCHES` `refactor/remove-ort-landmark-detector`, `refactor/vision-pipeline-3-model`,
  `refactor/model-ids-nextgen`, `refactor/mock-backends-nextgen` and `docs/nextgen-model-documentation` [128].
- Root-level `*.py` and `temp_*.md` files are ignored by `.gitignore`: durable tooling lives under `scripts/` [128].
- Every topic branch that implements a backlog issue must be registered in `BRANCH_TO_ISSUE` in
  `scripts/sync_issue.py` (tooling-only `chore/` branches and GitHub-only review-finding branches
  without a backlog issue are not). `python3 scripts/sync_issue.py --check` must pass (enforced by
  `tests/invariants/src/sync_issue_contract.rs`); the soos-gui backlog work is `#51` (formerly a
  duplicate `#22`) and has no GitHub issue [105].
- Use repository-relative paths in all docs and skills (never `/home/<user>/...`).
