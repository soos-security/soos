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
| `crates/pam` | `soos-pam` → `libpam_soos.so` | cdylib + rlib | adapter: `unsafe` allowed, `// SAFETY:` mandatory |
| `crates/camera-v4l` | `soos-camera-v4l` | lib, `mock-camera` feature | adapter: `unsafe` allowed, `// SAFETY:` mandatory |
| `crates/daemon` | `soos-daemon` | lib + bin (Tokio) | `main.rs` forbids; `lib.rs` only denies undocumented unsafe (`mlock.rs`) |
| `tests/invariants` | `soos-invariants` | static repo checks | — |

- The authoritative forbid list is `test_business_crates_forbid_unsafe_code` in
  `tests/invariants/src/lib.rs` (9 crates). `AGENTS.md` lists only 3 — that is a minimum, not the full set.
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
| `DECISION_BUDGET_MS` | `crates/daemon/src/pipeline.rs` | 900 ms |
| `MAX_FRAME_AGE_NS` | `crates/daemon/src/pipeline.rs` | 150 ms |
| `FRAME_POLL_INTERVAL_MS` | `crates/daemon/src/pipeline.rs` | 10 ms (consensus loop poll) |
| `MAX_CONCURRENT_INFERENCES` | `crates/daemon/src/inference.rs` | 1 (blocking-pool inference slots) |
| `RESPONSE_WRITE_MARGIN_MS` | `crates/daemon/src/inference.rs` | 50 ms (reserved before client and outer deadlines) |
| `DEFAULT_INFERENCE_ESTIMATE_MS` / `MAX_INFERENCE_ESTIMATE_MS` | `crates/daemon/src/inference.rs` | 80 / 1000 ms (initial / upper bound of the EMA admission estimate) |
| `DEFAULT_PAD_CONSENSUS_REQUIRED` / `_WINDOW` / `MAX_PAD_CONSENSUS_WINDOW` | `crates/policy/src/pad_consensus.rs` | 3 / 5 / 32 captures (Allow needs 3 consecutive; any spoof vetoes the request) |
| GDM PAM line | `crates/admin-cli/src/gdm.rs` | `auth sufficient pam_soos.so timeout_ms=2500` |
| admin-cli `DEFAULT_TIMEOUT_MS` | `crates/admin-cli/src/args.rs` | 250 ms |
| Match / PAD thresholds | `crates/vision/src/pipeline.rs`, policy | 0.70 / 0.85 |
| `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` | `crates/inference-ort/src/pad.rs` | 1 |

Known prose drift: `AGENTS.md`, `AI/ARCHITECTURE.md` and `AI/DECISIONS.md` still say "200–250ms".
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

## 4. Model Contract (manifest `models/manifest.toml` v2.0.0)

| ID | File | Input | Normalization | Notes |
|---|---|---|---|---|
| `scrfd_500m_kps` | `scrfd_500m_kps.onnx` | 640×640 BGR, letterbox | `(x-127.5)/128` | 9 outputs (3 strides × score/bbox/kps); **scores are already sigmoided** [65] |
| `minifasnet_v2_pad` | `minifasnet_v2_80x80.onnx` | 80×80 BGR, 2.7× expanded bbox | `x/255` | live class index **1** [72, 79]; `[PrintPhoto, Live, ScreenReplay]`, never overridden in production |
| `arcface_w600k_mbf` | `arcface_w600k_mbf.onnx` | 112×112 aligned, **NHWC** `input_1` `[N,112,112,3]`, fed **BGR** [68, 71] | `(x-127.5)/127.5` | **ArcFace ResNet34** (tf2onnx, 34.1 M params, 136.6 MB), **not** MobileFaceNet — the id is historical [102]; output `embedding` `[N,512]`, L2-normalized by the extractor; trained channel order / normalization unverified |

Any change to channel order, layout, class index or normalization MUST be validated against the real
`.onnx` metadata (input/output shapes) — mocks alone hid four shipped bugs [65, 66, 68, 71].

`input_shape` is the logical `[N, C, H, W]` shape; `input_layout` (default `"NCHW"`, `"NHWC"` for the
embedding model) is the physical layout. `ModelRegistry::get_or_load_session` validates every
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
./run_tests.sh                                 # Dockerized PAM matrix T1–T12 (ubuntu)
```

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
- Branch prefixes allowed by `AGENTS.md`: `feat/`, `fix/`, `test/`, `chore/` (not `refactor/` or `docs/`).
- Every topic branch that implements a backlog issue must be registered in `BRANCH_TO_ISSUE` in
  `scripts/sync_issue.py` (tooling-only `chore/` branches without a backlog issue are not).
- Use repository-relative paths in all docs and skills (never `/home/<user>/...`).
