# Walkthrough 124 — Fresh Enrollment Candidates, PAD-Tolerant Selection and Metadata-Only Listing

- **Date**: 2026-09-30
- **Issues**: GitHub #228 (STO-05, multi-frame enrollment re-reads the cached frame and aborts on
  the first PAD rejection), GitHub #235 (STO-19, `list` decrypts full templates; template reads
  are unbounded)
- **Branch**: `fix/p2-enroll-frames-list-metadata`
- **Matrix criteria**: EFL1–EFL7 (new)
- **ADRs**: 2026-09-30 "Fresh Enrollment Candidates and PAD Rejections as Invalid Candidates",
  2026-09-30 "Bounded Template Reads and Metadata-Only Listing"

---

## 1. Context

`EnrollmentService::enroll` took `camera.latest_frame()` once per candidate with no sequence
check. When inference is faster than the frame interval (mocks, a low-FPS camera, a stalled
capture thread) several candidates were the same frame, so `--frames N` did not evaluate N
captures. Any `VisionError::PadFailed` fell into the catch-all `Err(e) => return Err(e.into())`
and aborted the whole enrollment.

`EnrollmentService::list` (and the GUI profile refresh) called `BiometricStore::get` per UID,
which reads with an unbounded `read_to_end`, decrypts, and allocates the embedding only to copy
five metadata fields.

## 2. Specification

| Item | Crate | Definition |
|---|---|---|
| `ENROLL_FRESH_FRAME_TIMEOUT_MS` | `soos-enrollment-cli` | `500`; maximum wait for a frame newer than the previous candidate |
| `EnrollmentService::acquire_frame_after(Option<u64>)` | `soos-enrollment-cli` (private) | `None`: first cached frame (2 s budget, unchanged); `Some(prev)`: first frame with `sequence > prev`, polling every 10 ms, else `CameraError::Starved` |
| `EnrollmentSummary::pad_rejections` | `soos-enrollment-cli` | number of candidates rejected by PAD (`PadFailed`, `IrLivenessGateFailed`) |
| `MAX_TEMPLATE_FILE_BYTES` | `soos-biometric-store` | `64 * 1024` |
| `BiometricStore::get_metadata` | `soos-biometric-store` | `Result<Option<TemplateMetadata>, BiometricStoreError>` |
| `TemplateMetadata` | `soos-biometric-store` | `uid`, `model_id`, `model_version`, `enrollment_timestamp`, `embedding_dim`; `from_cbor` validates like `BiometricTemplate::from_cbor` |

Security invariants kept: only a frame that passed the full pipeline (PAD included) can be
selected, so a spoof is never enrolled; an all-PAD-rejected run returns the PAD error and stores
nothing; a stale camera fails closed. No frame, embedding or key material is logged. Both crates
keep `#![forbid(unsafe_code)]`; no `unwrap`/`expect` in production code.

## 3. Red evidence (tests written first)

- `crates/enrollment-cli/tests/enroll_fresh_frames_tests.rs` (a `SlowSwapCamera` that publishes a
  new sequence every 50 ms; a `RecordingDetector` that records the sequence encoded in the
  pixels). With the constant and summary field stubbed and the old loop: all 4 tests failed —
  `test_enroll_candidates_carry_distinct_frame_sequences` (4 candidates, 1 distinct sequence),
  `test_enroll_counts_pad_rejection_as_invalid_candidate` (enrollment aborted with `PadFailed`),
  `test_enroll_all_pad_rejections_fails_closed_without_storing` (aborted after 1 frame instead of
  evaluating 3), `test_enroll_frozen_camera_fails_with_bounded_wait` (the frozen frame was
  enrolled 3 times).
- `crates/biometric-store/tests/bounded_read_tests.rs`: did not compile (`get_metadata`,
  `TemplateMetadata`, `MAX_TEMPLATE_FILE_BYTES` missing).
- `crates/enrollment-cli/tests/list_bounded_tests.rs`:
  `test_list_refuses_oversized_template_file` failed (a 10 MiB file was read and reported as a
  `Crypto` MAC failure, not refused as `CorruptFile`).
- `tests/invariants/src/enroll_list_metadata_contract.rs`: both tests failed (`list` and the GUI
  called `get(uid)`).

## 4. Change

- `crates/enrollment-cli/src/service.rs`: `acquire_frame_after`; the enroll loop tracks
  `last_sequence`, counts PAD rejections as empty-detection candidates, and returns the last PAD
  error when no candidate is valid and at least one was PAD-rejected; `list` uses
  `get_metadata`.
- `crates/enrollment-cli/src/main.rs`: the confirmation summary prints `PAD-rejected frames`.
- `crates/biometric-store/src/store.rs`: `MAX_TEMPLATE_FILE_BYTES`; shared bounded
  `read_decrypted` (fstat check, `take(MAX + 1)`, re-check); `get_metadata`; `enroll` refuses an
  oversized ciphertext; UID check factored into `check_uid`.
- `crates/biometric-store/src/template.rs`: `TemplateMetadata` and a counting
  `EmbeddingShape` deserializer (no `Vec<f32>` allocation); unit tests in `metadata_tests`.
- `crates/gui/src/app.rs`: `load_local_profiles` uses `get_metadata`.
- The evidence store already bounds `load_snapshot` with `MAX_EVIDENCE_FILE_BYTES` (GitHub
  #181); EFL7 cites the existing test.

## 5. Verification

- `cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features
  -- -D warnings`, `cargo test --locked --workspace --all-targets --all-features
  --no-fail-fast`, `./scripts/candid_review.sh`.
- Behavior change visible to existing callers: enrollment over the mock camera now takes about
  one frame period per candidate (30 FPS mock: ~33 ms each). No existing test was modified.

## 6. Follow-ups

- Optional on-disk format change: carry the metadata in an AES-GCM authenticated cleartext header
  (AAD) so listing needs no decryption at all (needs a migration of existing templates).
- Real-camera confirmation that `V4lCameraManager` sequences advance within 500 ms at the
  configured idle/throttled FPS during enrollment (hardware).
