# Walkthrough 101 — GUI Import over stdin and Self-Describing Evidence Snapshots

- **Date**: 2026-09-30
- **Issues**: Review findings CAM-08 / STO-12 (GitHub #156) and STO-08 (GitHub #181) —
  **Branch**: `fix/import-stdin-evidence-format`
- **Matrix criteria**: ISE1–ISE7 (`gui-import-stdin`), ESF1–ESF5
  (`evidence-self-describing-frames`), all ✅ Verified
- **Docs**: `Docs/GUI_APPLICATION.md` §1 and §1b, `Docs/ENROLLMENT_CLI.md` (`soos-enroll import`),
  `Docs/EVIDENCE_STORE_CRATE.md` §1, §3.2–§3.3, §4.2

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`) confirmed two MAJOR
findings in the storage paths:

- **#156**: the GUI staged the plaintext 512-D fused embedding in a file under the system
  temporary directory for the whole Polkit prompt. Walkthrough 90 already made that file `0600`,
  `O_EXCL`, randomly named and removed by a drop guard, but the template still reached the
  filesystem, `soos-enroll import` read any file with an unbounded `std::fs::read` and no owner
  check, and the GUI silently fell back to `temp_dir()/soos-gui-master.key` and
  `temp_dir()/soos-gui-biometrics` whenever the system store was unreadable, so users could
  believe they had enrolled for PAM while their template sat in `/tmp`.
- **#181**: `EvidenceRecord` held raw V4L2 bytes without width, height or pixel format, in files
  named `.webp.enc` although nothing encodes WebP; the evidence could not be rendered, and the
  CBOR plaintext and record were plain `Vec<u8>` dropped without zeroization.

Objectives: embedding over a pipe with bounded, validated input on the root side; an explicit
developer mode instead of the silent fallback; a versioned, authenticated, self-describing
evidence record with honest naming and backward compatibility.

## 2. Architect Design

### 2.1 GUI and `soos-enroll import` (#156)

- `privileged::import_helper_args(uid)` → `soos-enroll import --uid <uid> --file -`;
  `privileged::import_template_with(Command, uid, &[f32])` spawns the helper (production:
  `pkexec`), writes the JSON from a `Zeroizing` buffer to its stdin, closes the pipe and always
  reaps the child. `TempFileGuard` and `write_private_file` are removed.
- New module `store_mode`: `GuiStore::{System(Arc<BiometricStore>), Developer { store, dir },
  Polkit}` with `local()`, `uses_polkit()`, `banner()`, and `resolve_gui_store(key_file,
  biometrics_dir, dev_store)`. `GuiArgs` gains `--dev-store <DIR>` (absolute). `SoosApp::new`
  takes the `GuiStore` instead of `(Arc<BiometricStore>, is_system_store)`.
- Enrollment CLI: `MAX_IMPORT_INPUT_BYTES` (64 KiB), `IMPORT_STDIN_PATH` (`-`),
  `EnrollmentService::import_from_reader`, `read_import_file(path, expected_owner)`,
  `parse_pkexec_uid`, `EnrollmentCliError::InvalidImport`.
- **Why `--file -` and not a new `--stdin` flag**: `ImportArgs` is built with an exhaustive
  struct literal by the existing contract tests (`import_tests.rs`); adding a field would have
  required editing them. `-` is the conventional stdin marker and keeps `ImportArgs` unchanged.

### 2.2 Evidence store (#181)

- New module `frame`: `EvidencePixelFormat::{Gray8, Rgb24, Yuyv, Nv12, Mjpeg}` (serialized as the
  V4L2 fourcc), `FrameMetadata { width, height, pixel_format, captured_at_mono_ns, sequence }`
  with `validate(data_len)`, `EvidenceFrame<'a> { metadata, data }`, `FrameBytes` (zeroize on
  drop, redacted `Debug`, CBOR byte string, legacy integer arrays still accepted), constants
  `EVIDENCE_RECORD_VERSION` (2), `LEGACY_EVIDENCE_RECORD_VERSION` (1), `MAX_EVIDENCE_DIMENSION`
  (8192), `MAX_EVIDENCE_IMAGE_BYTES` (32 MiB), `MAX_EVIDENCE_FILE_BYTES` (65 MiB).
- `EvidenceRecord` gains `format_version` (serde default 1) and `frame: Option<FrameMetadata>`
  (serde default `None`); `image_data` becomes `FrameBytes`. `from_cbor` validates; `to_cbor`
  returns `Zeroizing`; `to_rgb24()` reconstructs RGB (BT.601 for YUV).
- `EvidenceStore::store_frame_snapshot(uid, reason, &EvidenceFrame, ..)` writes
  `<uuid>.frame.enc`; `store_snapshot(&[u8])` stays for opaque payloads with the historical
  `.webp.enc` name (`OPAQUE_SNAPSHOT_EXTENSION`), because `permissions_tests.rs` asserts that
  suffix for this API (tests are immutable contracts). The daemon now uses
  `store_frame_snapshot` with the camera `Frame` metadata (`evidence_pixel_format` mapping in
  `dispatcher.rs`).
- Authentication: the version and metadata are fields of the CBOR record, which is the AES-GCM
  plaintext, so they are covered by the GCM tag; the `SOOSEVD1` envelope is unchanged and
  existing keys keep working.

## 3. Tester Contract (Red Phase)

| Test file | Tests | Red evidence |
|---|---|---|
| `crates/gui/tests/import_privacy_tests.rs` (new) | 9: helper argv, stdin pipe with no `temp_dir()` file during/after the call, helper failure, helper ignoring stdin, static no-`temp_dir` guard, System / Polkit / Developer selection, `--dev-store` parsing | `unresolved imports soos_gui::privileged::import_helper_args, import_template_with`; `unresolved import soos_gui::store_mode`; `no field dev_store on type GuiArgs` |
| `crates/enrollment-cli/tests/import_stdin_tests.rs` (new) | 8: CLI `--file -`, stdin import, oversized and endless stdin, malformed / wrong-dimension / non-finite payloads, oversized file, `PKEXEC_UID` owner check, symlink and directory refusal, `parse_pkexec_uid` | `unresolved imports ... parse_pkexec_uid, read_import_file, IMPORT_STDIN_PATH, MAX_IMPORT_INPUT_BYTES`; `no method named import_from_reader` |
| `crates/evidence-store/tests/frame_format_tests.rs` (new) | 9: self-describing roundtrip, RGB reconstruction (GREY/YUYV/NV12), MJPEG verbatim, invalid frames refused without side effects, legacy v1 decode, unknown version / contradictory metadata, oversized file, redacted `Debug`, 320x240 roundtrip beyond the decoder scratch buffer | `unresolved imports soos_evidence_store::EvidenceFrame, EvidencePixelFormat, FrameMetadata, ...`; `no method named store_frame_snapshot`; `no field frame on type EvidenceRecord` |
| `crates/daemon/tests/pipeline_integration_tests.rs` (1 test added) | `test_181_password_failed_snapshot_records_frame_metadata` | `no field frame on type EvidenceRecord`, `no method named to_rgb24` |

No existing test was modified. The first green run of the daemon test exposed a real bug:
`deserialize_bytes` only works for byte strings shorter than ciborium's 4 KiB scratch buffer
("invalid type: bytes, expected bytes"); `FrameBytes` now uses `deserialize_byte_buf`, and the
320x240 evidence-store test pins it.

## 4. Auditor Constraints

1. No `unwrap`/`expect`/indexing/unchecked arithmetic in production code (workspace lints with
   `-D warnings`); the YUV fixed-point helper carries a reasoned `allow` (8-bit inputs).
2. Every input is bounded while it is read: `Read::take` on stdin and import files (64 KiB);
   evidence files refused above 65 MiB via `fstat` and read through `take`; legacy integer
   arrays bounded element by element.
3. No embedding or frame values in logs, errors or `Debug` (`FrameBytes`, `EvidenceFrame`,
   `PrivilegedAction` redact; import errors never echo values).
4. Zeroization: GUI JSON buffer and fused embedding, import raw bytes and parsed embedding,
   evidence record payload, CBOR plaintext and reconstructed RGB.
5. File input checks run on the open descriptor (`O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK`,
   `fstat`): no TOCTOU window, FIFOs cannot block root.
6. Fixed argument vectors, no shell; the pkexec child is always reaped.
7. Invalid evidence frames never consume the per-UID daily cap nor create files.

## 5. Implementation Summary

- `crates/gui/src/privileged.rs`, `store_mode.rs` (new), `args.rs`, `main.rs`, `app.rs`, `lib.rs`,
  `Cargo.toml` (`tempfile` dev-dependency).
- `crates/enrollment-cli/src/service.rs`, `args.rs` (help text only), `error.rs`.
- `crates/evidence-store/src/frame.rs` (new), `snapshot.rs`, `store.rs`, `error.rs`, `lib.rs`.
- `crates/daemon/src/dispatcher.rs` (evidence call and pixel-format mapping only).

The GUI now also reports a failed local `enroll` instead of ignoring it (`let _ =`), and the
developer and system modes no longer write a second copy into a scratch store.

## 6. Migration & Compatibility

- Existing evidence files (`.webp.enc`, no version) decode unchanged as version 1 with
  `frame == None`; new daemon snapshots are `.frame.enc` version 2. No rewrite is required and
  retention rotation handles both.
- GUI users who relied on the implicit `/tmp` store must pass `--dev-store <DIR>`; templates left
  in `temp_dir()/soos-gui-biometrics` by older versions are no longer read and can be deleted.
- `soos-enroll import --file <path>` keeps working for files owned by the caller; under `pkexec`
  the file must belong to `PKEXEC_UID`.

## 7. Validation

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features --no-fail-fast
./scripts/candid_review.sh
```

Results are recorded in §8.

## 8. Gate Results

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo test --locked --workspace --all-targets --all-features --no-fail-fast` | 135 test binaries, 901 passed, 0 failed |
| `./scripts/candid_review.sh` | PASSED (7 audits) |

The serialized embedding (about 6 KiB) fits in the default 64 KiB pipe buffer, so the GUI
worker never blocks on the write while the Polkit prompt is open.

## 9. Open Points

- `store_snapshot(&[u8])` still writes the historical `.webp.enc` suffix because an existing
  contract test pins it; renaming it (e.g. `.raw.enc`) needs an explicit decision to update
  `crates/evidence-store/tests/permissions_tests.rs`.
- No `soos-admin evidence export` command exists yet (suggested by the review); `load_snapshot`
  + `to_rgb24` provide the building blocks.
- In Polkit mode the Profiles tab cannot run the in-process match test on system templates
  (they are not readable by the user); this is intentional.
