# `soos-enrollment-cli` — Root Enrollment and Diagnostics Tool

`soos-enrollment-cli` (`soos-enroll`) is the administrative command-line interface for the `soos` local biometric PAM subsystem. It provides privileged user enrollment, template deletion (best-effort overwrite, encryption-backed erasure), diagnostic verification, and template inventory enumeration.

---

## 1. Architectural Role & Security Invariants

In accordance with `AI/ARCHITECTURE.md` (§8 Monorepo Structure, §9 Privacy, Persistence, and Anti-Intrusion):
- **Root-Only Modification**: State-modifying operations (`enroll`, `delete`) mandate root execution (effective UID 0), preventing unauthorized tampering with biometric credentials. Arguments are parsed before the privilege check, so `--help`, `--version` and usage errors work for any user; every subcommand still requires root (GitHub #237).
- **Single-Face Security Invariant**: The tool strictly enforces that exactly one human face is visible in any frame evaluated for enrollment or verification. Multi-face or zero-face candidate frames are rejected.
- **Encrypted Persistence**: All biometric templates are serialized to canonical CBOR and encrypted using authenticated AES-256-GCM via `soos-biometric-store` under `/var/lib/soos/biometrics/<uid>.cbor.enc` with POSIX permissions `0600` owned by `root:root`.
- **Template Erasure (honest scope)**: `delete` and re-`enroll` overwrite the discarded template file in place with kernel CSPRNG bytes (3 passes, each followed by `fsync()`) before it is unlinked or replaced, then sync the directory. This is **best effort only**: it does not reach the physical blocks on copy-on-write filesystems (btrfs, ZFS), with data journaling, snapshots or backups, or on SSD/eMMC media with wear levelling. The actual guarantee is that every template is AES-256-GCM ciphertext under `/var/lib/soos/master.key`; residual copies are unreadable without that key, and destroying the key (together with full-disk encryption for the key file itself) is the only complete erasure. See ADR 2026-09-30 "Biometric Template Erasure Model" in `AI/DECISIONS.md` and `Docs/BIOMETRIC_STORE_CRATE.md` §3.4.
- **Zeroization**: Sensitive biometric vector buffers (`Vec<f32>`) implement `Zeroize` via `zeroize::Zeroizing` and are automatically scrubbed from memory when dropped.

---

## 2. Command Reference

### `soos-enroll enroll`
Enrolls a user with multi-frame quality selection and interactive confirmation:

```bash
# Interactive enrollment of the user who invoked sudo (SUDO_UID)
sudo soos-enroll enroll

# Interactive enrollment of a specified UID
sudo soos-enroll enroll --uid 1000 --frames 5

# Enrolling root is only possible explicitly
sudo soos-enroll enroll --uid 0

# Enrollment by username with automated non-interactive confirmation
sudo soos-enroll enroll --username alice --yes
```

**Options**:
- `-i, --uid <UID>`: Target Linux User ID. Without `--uid` and `--username`, the target is the invoking user: the real UID when it is not root, otherwise `SUDO_UID` (then `PKEXEC_UID`) when it names a non-root user. Root is never an implicit target: with no such variable the command fails before any capture and asks for `--username`/`--uid`; `--uid 0` (or `--username root`) enrolls root explicitly. A malformed `SUDO_UID`/`PKEXEC_UID` fails closed. The resolved target UID is printed before capture (GitHub #184 / STO-11).
- `-u, --username <NAME>`: Target username (resolved via system user database).
- `--frames <N>`: Number of candidate frames to capture and evaluate (default: 5, bounded 1–30).
  Each candidate after the first is a distinct capture: the CLI waits up to `ENROLL_FRESH_FRAME_TIMEOUT_MS` = 500 ms for a frame whose `sequence` is greater than the previous candidate's and fails with a starved-camera error otherwise (nothing stored). A frame rejected by presentation attack detection is an invalid candidate, shown as `PAD-rejected frames` in the confirmation summary, and does not abort the enrollment; if no candidate is valid and at least one was PAD-rejected, the PAD error is reported (GitHub #228 / STO-05). A face rejected by the pre-PAD quality gate (too small or too blurred, GitHub #218) is also an invalid candidate, never an abort; when no candidate is usable the enrollment ends with the low-quality error and nothing is stored (GitHub #285).
- `-y, --yes`: Automatically confirm enrollment without interactive confirmation prompt. Without it, the confirmation summary warns when the user is already enrolled and saving replaces the existing template (`EnrollmentSummary::already_enrolled`, GitHub #233).
- `--model-id <ID>`: Override of the embedding model identifier stored in template metadata. Default: the loaded embedding extractor `MODEL_ID_EMBEDDING` = `arcface_w600k_mbf` (GitHub #182 / STO-09; the former `mobilefacenet` default is retired per ADR 2026-09-20).
- `--model-version <VER>`: Override of the model version stored in template metadata. Default: `EMBEDDING_MODEL_VERSION` = `2.0.0`, the attested `models/manifest.toml` version (pinned by a test).
- Any override that differs from the loaded model prints a warning before capture and sets `model_overridden` in the confirmation summary: `soos-daemon` refuses templates whose `model_id` differs from its loaded extractor (`Verdict::Unavailable` / `ReasonClass::ModelUnavailable`, PAM falls back to the next module), so such a template can never authenticate. Migration aid: templates recorded with the retired default, exactly `mobilefacenet` / `1.0.0`, contain ArcFace vectors and are still accepted as a legacy alias with a warning recommending re-enrollment (ADR 2026-09-30 "Embedding Model Binding and Legacy Model Alias"); any other version or id is refused. Re-enroll affected users (`soos-enroll list` shows the recorded model) before the alias is removed.

### `soos-enroll verify`
Performs a one-shot diagnostic verification against an enrolled biometric template:

```bash
sudo soos-enroll verify --uid 1000
```

**Report Details**:
- Match verdict: `Allow` (similarity >= threshold) or `Deny`.
- Match score: Cosine similarity value `[-1.0, 1.0]`.
- Face count and detection confidence.
- Presentation Attack Detection (PAD) status (GitHub #216, #236): the report carries the PAD outcome class (`pad_result`), the liveness score returned by the PAD model (`pad_score`) and the effective threshold applied to the frame modality (`pad_threshold`, the IR threshold for monochrome frames). The `PAD Anti-Spoof` line reads, for example:
  - `PASSED (score=0.930, threshold=0.85)` — live presentation;
  - `SPOOF(score=0.120, threshold=0.85)` — presentation attack: `Deny` verdict with the PAD score, never a generic error;
  - `IR_GATE_REJECTED(<reason>)` — the fail-closed IR liveness gate rejected a monochrome crop before the PAD model ran (no score);
  - `FACE_TOO_SMALL(width=<w>px, min=<m>px)`, `FACE_BLURRED(sharpness=<s>, min=<m>)` — the pre-PAD quality gate (GitHub #218) rejected the face: `Deny` report, PAD did not run (GitHub #285);
  - `NO_FACE`, `MULTIPLE_FACES`, `LOW_CONFIDENCE(<c>)` — PAD did not run.
  Every non-`Allow` verdict, including a spoof, makes `soos-enroll verify` exit with status 1 after printing the report. The PAD score is a calibration aid only: it is not a secret, and no frame or embedding is printed.
- Latency breakdown: camera frame capture, neural vision pipeline, cosine matching, total roundtrip.

### `soos-enroll delete`
Deletes an enrolled biometric template (best-effort in-place overwrite before unlink; see "Template Erasure" above for what this can and cannot guarantee):

```bash
sudo soos-enroll delete --uid 1000
sudo soos-enroll delete --username alice --yes
```

### `soos-enroll list`
Lists all enrolled biometric templates with identity and model metadata:

```bash
# Human-readable table output
sudo soos-enroll list

# Machine-readable JSON output
sudo soos-enroll list --format json
```

`list` reads each template through `BiometricStore::get_metadata`: the file is authenticated and decrypted, but the embedding vector is never materialised, and a template file larger than `MAX_TEMPLATE_FILE_BYTES` (64 KiB) is refused as corrupt before it is read (GitHub #235 / STO-19).

The JSON array is produced by `serde_json` (`format_enrolled_json`), so usernames and model identifiers containing quotes or backslashes are escaped (GitHub #232).

### `soos-enroll import`
Imports an existing embedding (JSON array of 512 finite floats, or a CBOR `BiometricTemplate`) into the encrypted store. This is the command the GUI runs through `pkexec` (GitHub #156, review findings CAM-08 / STO-12):

```bash
# From standard input (the GUI path; the plaintext embedding never touches the filesystem)
sudo soos-enroll import --uid 1000 --file - < embedding.json

# From a file
sudo soos-enroll import --uid 1000 --file /home/alice/embedding.json
```

Input contract (`crates/enrollment-cli/src/service.rs`):
- **Bounded**: every input is capped at `MAX_IMPORT_INPUT_BYTES` (64 KiB) while it is read (`Read::take`); an endless or oversized stdin, or a larger file, is refused with `EnrollmentCliError::InvalidImport`.
- **Validated**: the embedding must have exactly 512 values, all finite (an overflowing JSON number such as `1e39` becomes infinity and is refused). Values are never echoed in errors.
- **Zeroized**: the raw input and the parsed embedding live in `Zeroizing` buffers.
- **File inputs** (`read_import_file`): opened with `O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK` and checked on the open descriptor: regular file only (symlinks, directories and FIFOs are refused), at most 64 KiB, and, under `pkexec`, owned by the invoking user (`PKEXEC_UID`, parsed by `parse_pkexec_uid`), so a Polkit caller cannot make root import another user's file.
- Nothing is stored when any check fails.
- **No silent overwrite** (GitHub #237): a file import onto a user who already has a template fails with `EnrollmentCliError::AlreadyEnrolled` before the file is read; pass `-y, --yes` to replace it. The same rule applies to the standard-input channel (`--file -`): the check runs before stdin is read, so a refused import never consumes its input. The GUI confirms the enrollment itself and therefore passes `--yes`. A replacement is reported (`EnrollmentOutcome::replaced_existing`), and the CLI prints a warning whenever a template was replaced.

Verification: `crates/enrollment-cli/tests/import_stdin_tests.rs`, `import_tests.rs` (matrix rows ISE5–ISE7), `cli_hygiene_tests.rs` (rows CDJ5–CDJ7) and `import_stdin_overwrite_tests.rs` (rows CDJ7, CDJ9).

### `soos-enroll debug-vision`
Captures one camera frame, runs the face detector and writes a standalone HTML report drawing the bounding boxes, confidence scores and 5-point landmarks on an HTML5 canvas (GitHub #149 / STO-02 hardening):

```bash
# Geometry-only report in the root-only default directory (/var/lib/soos/debug/soos-debug-<unix_secs>-<pid>.html)
sudo soos-enroll debug-vision

# Explicit output path (absolute, no '..', under an allowed FHS prefix; the file must not exist)
sudo soos-enroll debug-vision --output /var/lib/soos/debug/lighting-check.html

# Opt in to embedding the raw camera frame (biometric data) in the report
sudo soos-enroll debug-vision --embed-frame

# Mock camera and synthetic detector, no hardware required
sudo soos-enroll --mock debug-vision
```

Filesystem and privacy contract (`crates/enrollment-cli/src/service.rs`):
- **Atomic root-only creation**: the report is opened with `O_CREAT | O_EXCL | O_NOFOLLOW` and mode `DEBUG_REPORT_FILE_MODE` (`0600`) in a single `open(2)` call; the mode is never fixed up with `chmod` after the write.
- **Never overwrite, never follow**: a pre-existing file or symbolic link at the output path, or a symbolic link in place of the parent directory, aborts the command with `EnrollmentCliError::DebugReportRefused` and leaves the existing target byte-identical. This closes the root truncate-through-symlink primitive of the former `std::fs::write` into the current working directory.
- **Safe default location**: without `--output`, reports go to `DEFAULT_DEBUG_REPORT_DIR` (`/var/lib/soos/debug`), created with `DEBUG_REPORT_DIR_MODE` (`0700`) when missing; an existing directory is accepted as is and never `chmod`-ed. An explicit `--output` passes `validate_fhs_path` (absolute, no `..`, allowed FHS prefix) and is validated before the camera is touched.
- **Frame embedding is opt-in**: by default the report contains only detection geometry and resolution. `--embed-frame` embeds the raw RGB24 frame as base64; the report then carries a visible privacy warning and the CLI prints a reminder to delete it after use. The raw frame is biometric data (`AI/ARCHITECTURE.md` lists storing raw frames on disk as an anti-pattern), so embedding is a deliberate, per-invocation administrator decision.
- **Detector errors are propagated**: a failing detector aborts the command (`EnrollmentCliError::Inference`) instead of silently producing a report with zero detections, and no file is created.

Verification: `crates/enrollment-cli/tests/debug_vision_tests.rs` (matrix rows EN12–EN14).

---

### Guided multi-angle enrollment (`GuidedEnrollmentSession`, used by `soos-gui`)

`soos_enrollment_cli::guided_enrollment` collects samples over four steps and fuses them (GitHub #183 / STO-10). A sample is recorded only when:

| Check | Rule | Feedback on failure |
|---|---|---|
| Liveness / centering | PAD live and face centered | `SpoofDetected` / `PromptCenterFace` |
| Pose finiteness | yaw, pitch, roll finite | `PromptHoldStill` |
| Roll (all steps) | `abs(roll) <= 10` degrees | `PromptHoldStill` |
| Frontal | `abs(yaw) <= 8`, `abs(pitch) <= 10` | `PromptCenterFace` / `PromptHoldStill` |
| Turn left | yaw in `[-25, -10]`, `abs(pitch) <= 15` | `PromptTurnLeft` (not enough) / `PoseOutOfRange` (too far) |
| Turn right | yaw in `[10, 25]`, `abs(pitch) <= 15` | `PromptTurnRight` / `PoseOutOfRange` |
| Tilt up | pitch in `[-25, -8]`, `abs(yaw) <= 15` | `PromptTiltUp` / `PoseOutOfRange` |
| Embedding validity | non-empty, at most `MAX_GUIDED_EMBEDDING_DIM` (2048) values, same dimension as the first sample, all values finite, L2 norm > 1e-6 | `InvalidEmbedding` |
| Identity consistency | cosine >= `MIN_SAMPLE_CONSISTENCY_COSINE` (0.5) with every accepted frontal sample and, for off-axis steps, with the frontal mean direction | `IdentityMismatch` |

Each step records at most `MAX_SAMPLES_PER_STEP` (20) samples (`GuidedEnrollmentSession::new` clamps the target to `1..=20`). The composite template is the mean direction of the L2-normalized samples, `t = m / ||m||` with `m = sum_i s_i / ||s_i||`; `compute_composite_embedding` re-validates every sample (dimension, finiteness, consistency with the frontal anchor) and fails closed instead of returning a non-finite or contaminated vector.

**Session-level liveness (GitHub #217 / PAD-12).** Liveness is tracked over the session with a
`LivenessPolicy`, not per frame:

- a sample is recorded only after `min_consecutive_live_frames` consecutive live frames; a live frame
  that is not yet sampled returns `PromptHoldStill`;
- a spoof frame discards every sample of the current step and resets the streak (`SpoofDetected`);
- the `max_spoof_events`-th spoof frame aborts the session: every sample is discarded,
  `SessionAborted` is returned from then on and `compute_composite_embedding` fails;
- `interrupt_liveness_streak()` breaks the streak without counting a spoof event (frames with no
  PAD verdict or rejected by the quality gate).

`LivenessPolicy::strict()` (3 frames, 3 spoof events; both values clamped to `1..=32`) is the policy
of the GUI (`soos_gui::worker::new_guided_enrollment_session`). `GuidedEnrollmentSession::new`
keeps a single-frame gate (`LivenessPolicy::single_frame()`), still with the step reset and the
spoof abort, for compatibility with its existing contract.

---

### `soos-enroll migrate`
Re-encrypts every legacy (payload format v1, unbound) biometric template and evidence snapshot with the AES-GCM associated-data bound v2 envelope (GitHub #287, owner decision 2026-10-01; ADR 2026-10-01 "Operator-Run Migration of Legacy v1 Storage Envelopes"). Legacy files stay readable without it: there is no cut-off, the command only removes the remaining unbound files.

```bash
# Report what would be migrated, write nothing
sudo soos-enroll migrate --dry-run

# Migrate, human-readable summary
sudo soos-enroll migrate

# Machine-readable summary (serde_json), custom evidence location from [pipeline.evidence]
sudo soos-enroll migrate --format json --evidence-dir /var/lib/soos/evidence --evidence-key-file /var/lib/soos/evidence.key
```

- **Root only**, like every other subcommand (arguments are parsed first, so `--help` works for any user).
- **Templates**: `BiometricStore::migrate_legacy_templates` over `--biometrics-dir` / `--key-file`. Unlike `list` and `delete`, `migrate` (with or without `--dry-run`) **never creates the master key or the biometrics directory**: the key is loaded with `MasterKey::load_existing`, and a missing key or directory reports the template store as skipped (nothing can exist to migrate) while evidence is still processed. **Evidence**: `EvidenceStore::migrate_legacy_snapshots` over `--evidence-dir` (default `/var/lib/soos/evidence`) with the key `--evidence-key-file` (default `/var/lib/soos/evidence.key`). A missing evidence key means evidence was never enabled: evidence is reported as skipped and **no key is created**. An existing key is validated like the daemon validates it.
- **Atomic and symlink-safe**: every rewrite goes through the store's own write path (temporary file created `0600`, `fsync`, `rename`, directory `fsync`); no cryptography is duplicated in the CLI. Symlinks are refused or skipped, never followed.
- **Idempotent**: v2 files are never rewritten; a second run reports `0` migrated.
- **Per-file failures** (tampered or foreign file, symlink, I/O error) are counted and listed with their UID or path and reason; they never abort the other files and leave the file untouched. A failure of a whole store (unlistable or symlinked directory) is reported the same way and the other store is still migrated. The command exits with status `1` when anything failed, `0` otherwise.
- **Summary**: per store `migrated` (with `--dry-run`: would migrate), `already_current`, `failed`, `failures[]` (`item`, `error`), plus `skipped` when the store was not examined. Neither output contains embedding values, frame bytes or key material.
- **Concurrency** (GitHub #289): each template rewrite runs under the biometric store lock shared with `enroll`, `import` and `delete` (an exclusive `flock` on the biometrics directory, held only around the file mutation, never across a camera capture), so a concurrent `delete` is never undone and a concurrent re-enrollment is never overwritten. A template that still changed between the read and the rewrite is reported as a per-file failure ("changed concurrently") and left as found; a lock still held after 5 s is reported as a per-file failure too. The template store is opened with `BiometricStore::open_existing`, so a biometrics directory removed after the existence check is never recreated. The daemon may keep running (it only reads templates, and new evidence is always written as v2).

Verification: `crates/enrollment-cli/tests/migrate_tests.rs`, `crates/biometric-store/tests/bulk_migration_tests.rs`, `crates/evidence-store/tests/legacy_migration_tests.rs` (matrix rows SMI1–SMI6, SMI10–SMI11).

## 3. Global Options

- `--biometrics-dir <PATH>`: Override biometric template storage directory (default: `/var/lib/soos/biometrics`).
- `--key-file <PATH>`: Override cryptographic master key path (default: `/var/lib/soos/master.key`).
- `--models-dir <PATH>`: Override ONNX neural models directory (default: `/var/lib/soos/models`).
- `--camera-device <PATH>`: Override V4L2 camera device node. Without it (or with `auto`/`default`), the device is resolved exactly like `soos-daemon` through the shared `soos_camera_v4l::resolve_camera_device`: `[pipeline] camera_device` from `/etc/soos/daemon.toml` if explicit, otherwise the capture node matching `[pipeline] sensor_preference` (default IR first), reported through its stable `/dev/v4l/by-id/` alias when udev created one, otherwise as the `/dev/videoN` node itself; with no capture node at all the `/dev/v4l/by-id/default-camera` sentinel is returned and opening it fails as "device not found" (GitHub #152, #197). The selection is pinned by the hermetic fixture table in `crates/enrollment-cli/tests/device_resolution_hermetic_tests.rs`.

---

## 4. Lazy Initialization & Model Attestation

- **Lazy Service Construction**: The CLI separates store-only initialization (`build_store_only`) from full biometric pipeline initialization (`build_full_service`). Non-biometric operations (`list`, `delete`) initialize solely the cryptographic `BiometricStore`, avoiding camera device allocation and neural model loading. This allows headless or unprovisioned machines to inspect and clean up templates without camera or model files.
- **Model Registry Attestation**: Biometric capture operations (`enroll`, `verify`) attest against official neural models defined in `models/manifest.toml`:
  - Face Detection & 5-Point Landmarks: `scrfd_500m_kps` (`scrfd_500m_kps.onnx`)
  - Presentation Attack Detection: `minifasnet_v2_pad` (`minifasnet_v2_80x80.onnx`), constructed by `service::build_pad_detector` with `OrtPadDetector::new`, i.e. the crate default live class index `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX = 1` shared with `soos-daemon` and `soos-gui` (never overridden; matrix PLC2, GitHub #146)
  - Feature Embedding (512D): `arcface_w600k_mbf` (`arcface_w600k_mbf.onnx`)
- **Deterministic Camera Addressing**: Satisfies Criterion C4 by resolving camera device paths via `/dev/v4l/by-id/`, eliminating enumeration races across kernel restarts.

