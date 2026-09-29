# Walkthrough 82 — `debug-vision` Report Safety (Atomic 0600 Creation, Symlink Refusal, Opt-in Frame)

- **Date**: 2026-09-30
- **Issue**: Review finding STO-02 (GitHub #149) from `AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md` — **Branch**: `fix/debug-vision-report-safety`
- **Matrix criteria**: EN12, EN13, EN14 (`AI/VERIFICATION_MATRIX.md`, component `enrollment-cli`)

---

## 1. Context & Objectives

`soos-enroll debug-vision` (walkthrough 11) captures one camera frame, runs the face detector and
writes a standalone HTML report. The full project review (STO-02, CRITICAL, verifier CONFIRMED)
found that the command, which is root-only (`main.rs` calls `check_privileges(true)`):

1. wrote the report with `std::fs::write(cwd.join("soos-debug.html"), html)` — `O_CREAT | O_TRUNC`,
   process umask (typically `0644`), and a pre-planted symbolic link at that name was followed. Any
   unprivileged user able to plant `soos-debug.html` in the administrator's working directory
   (for example `/tmp`) obtained a root truncate/overwrite-with-HTML primitive on an arbitrary file;
2. always embedded the full raw RGB frame as base64 in the report. `AI/ARCHITECTURE.md` lists
   "storing raw frames on disk" as an anti-pattern and states that raw enrollment frames are deleted
   immediately, yet the diagnostic left an unencrypted face image readable by every local user;
3. swallowed detector errors with `.unwrap_or_default()` (a broken model looked like "zero faces") and
   carried a block of stream-of-consciousness comments in `service.rs`.

Objectives: create the report atomically with mode `0600`, refuse symbolic links and pre-existing
files, default to a root-only directory, make frame embedding an explicit opt-in, propagate detector
errors, and document the biometric sensitivity of the report.

## 2. Architect Design

Scope: `crates/enrollment-cli` only (`args.rs`, `error.rs`, `html_report.rs`, `service.rs`,
`main.rs`, `lib.rs`). No other crate consumes the changed items.

```rust
// args.rs
pub enum Commands { /* ... */ DebugVision(DebugVisionArgs) }
pub struct DebugVisionArgs { pub output: Option<PathBuf>, pub embed_frame: bool } // clap Args, Default

// service.rs — one source of truth for the report location and modes
pub const DEFAULT_DEBUG_REPORT_DIR: &str = "/var/lib/soos/debug";
pub const DEBUG_REPORT_DIR_MODE: u32 = 0o700;
pub const DEBUG_REPORT_FILE_MODE: u32 = 0o600;
pub fn ensure_debug_report_dir(dir: &Path) -> Result<(), EnrollmentCliError>;
pub fn resolve_debug_report_path(output: Option<&Path>) -> Result<PathBuf, EnrollmentCliError>;
pub fn write_debug_report(path: &Path, html: &str) -> Result<(), EnrollmentCliError>;
impl EnrollmentService {
    pub fn debug_vision(&self, args: &DebugVisionArgs) -> Result<PathBuf, EnrollmentCliError>;
}

// html_report.rs
pub fn generate_html_report(width: u32, height: u32, embedded_frame_base64: Option<&str>,
                            detections: &[FaceDetection]) -> String;

// error.rs
EnrollmentCliError::DebugReportRefused { path: PathBuf, reason: String }
```

Sentinel and edge semantics:
- `--output` absent → `DEFAULT_DEBUG_REPORT_DIR/soos-debug-<unix_secs>-<pid>.html`; the directory is
  created with `DirBuilder::mode(0o700)` (non-recursive) when missing; a pre-existing directory is
  accepted as is (never `chmod`-ed, following the STO-04 lesson); a symbolic link or non-directory is refused.
- `--output` present → `validate_fhs_path` (absolute, no `..`, allowed FHS prefix) and must name a file
  with a parent. Validation runs before the camera is touched.
- Parent directory must be a real directory (`symlink_metadata`); a symlinked parent is refused.
- Anything already present at the output path (regular file, symbolic link, directory) is refused;
  the file is then opened with `write | create_new | mode(0600) | O_NOFOLLOW | O_CLOEXEC` in a single
  `open(2)`, so a link planted between the check and the open cannot be followed either
  (`O_EXCL` never follows symbolic links), and nothing is ever truncated.
- `--embed-frame` absent (default) → geometry-only report (boxes, scores, landmarks on a black
  canvas) with a notice explaining how to opt in; present → base64 RGB24 frame embedded and a visible
  privacy warning in the report plus a CLI reminder to delete the file after use.
- Detector errors propagate as `EnrollmentCliError::Inference` (via `From<InferenceError>`); no file is written.
- The RGB buffer and the base64 payload are held in `Zeroizing` wrappers.

Invariants touched: `AI/ARCHITECTURE.md` anti-pattern "storing raw frames on disk" (privacy §9);
auditor checklist #6 (create_new + 0600, `O_NOFOLLOW`, no chmod after write) and #7 (no biometric
data without explicit consent).

## 3. Plan Evaluation

Condensed inline (review-issue branch, single crate, no ADR conflict). Findings: the recommendation
in the review offered "drop the image and keep only detection geometry" as an alternative; the
chosen design keeps the image reachable behind an explicit flag because lighting and exposure
problems (the original purpose of walkthrough 11) are not diagnosable from geometry alone.
Verdict: APPROVED.

## 4. Tester Contract

All tests live in `crates/enrollment-cli/tests/debug_vision_tests.rs` and use a `TempDir` under `/tmp`
(an allowed FHS prefix), `MockCameraManager` (warm-up 0) and `MockFaceDetector::new_centered_face`.

| Test | Criterion | Red evidence (scaffolded API on top of the defective behavior) |
|---|---|---|
| `test_debug_vision_creates_report_with_mode_0600` | EN12 | `assertion left == right failed: report must be created with mode 0600 regardless of umask — left: 420 (0644), right: 384 (0600)` |
| `test_debug_vision_refuses_symlink_output_and_leaves_target_untouched` | EN12 | `Expected DebugReportRefused for a symlinked output, got: Ok("/tmp/.tmpPAirf6/report.html")` |
| `test_debug_vision_refuses_existing_file_without_truncation` | EN12 | `Expected DebugReportRefused for an existing file, got: Ok(...)` |
| `test_debug_vision_refuses_symlinked_parent_directory` | EN12 | `Expected DebugReportRefused for a symlinked parent directory, got: Ok(".../linked/report.html")` |
| `test_debug_vision_rejects_traversal_output_path` | EN14 | `Expected InvalidPath for a '..' component, got: Ok("/tmp/.tmpEK21fl/../report.html")` |
| `test_debug_vision_omits_frame_without_embed_flag` | EN13 | panicked at the size assertion (frame always embedded, report > 64 KiB) |
| `test_debug_vision_embeds_frame_only_with_explicit_flag` | EN13 | panicked at the "must warn that it contains biometric data" assertion |
| `test_debug_vision_propagates_detector_error` | EN14 | panicked at `matches!(res, Err(Inference(_)))` (error swallowed by `unwrap_or_default`) |
| `test_ensure_debug_report_dir_creates_0700_and_refuses_symlink` | EN14 | `assertion left == right failed — left: 493 (0755), right: 448 (0700)` |
| `test_default_debug_report_dir_is_root_only_location` | EN14 | passed once the constant existed (constant contract) |
| `test_cli_parse_debug_vision_subcommand_flags` | EN13 | passed once `DebugVisionArgs` existed (parsing contract) |

Before the scaffold the whole file failed to compile on exactly the specified API. Re-proven at
delivery time by reverting the `crates/enrollment-cli/src` hunks and running
`cargo test --locked -p soos-enrollment-cli --all-features --test debug_vision_tests --no-run`
(18 errors): `E0432 unresolved import soos_enrollment_cli::args::DebugVisionArgs`, `E0432 unresolved
imports ensure_debug_report_dir, DEBUG_REPORT_DIR_MODE, DEBUG_REPORT_FILE_MODE,
DEFAULT_DEBUG_REPORT_DIR`, `E0599 no variant named DebugReportRefused` (x5), `E0061 this method takes
0 arguments but 1 argument was supplied` (x9), `E0532 expected tuple struct or tuple variant, found
unit variant Commands::DebugVision` (x2). The production hunks were then restored unchanged.

Migrated existing tests: none. Flakiness check: the suite was run 10 times in a row, 11/11 passing each time.

## 5. Auditor Constraints

| # | Constraint | How it was met |
|---|---|---|
| 1 | `create_new(true)` + `mode(0o600)` + `O_NOFOLLOW`; never `fs::write`, never chmod after write | `write_debug_report` in `service.rs`; `grep fs::write crates/enrollment-cli/src/service.rs` is empty |
| 2 | Pre-existing target (file or symlink) refused and left byte-identical | `symlink_metadata` pre-check + `O_EXCL`; tests EN12 |
| 3 | Symlinked parent directory refused | `symlink_metadata(parent)` check; `test_debug_vision_refuses_symlinked_parent_directory` |
| 4 | Default directory root-only `0700`, existing directory never chmod-ed, symlink refused | `ensure_debug_report_dir`; `test_ensure_debug_report_dir_creates_0700_and_refuses_symlink` |
| 5 | Explicit output passes `validate_fhs_path` | `resolve_debug_report_path`; `test_debug_vision_rejects_traversal_output_path` |
| 6 | Raw frame only with `--embed-frame` | `args.embed_frame.then(...)`; EN13 tests |
| 7 | Detector errors propagated | `?` on `detect(...)`; `test_debug_vision_propagates_detector_error` |
| 8 | No unwrap/expect/panic/indexing in production code; `#![forbid(unsafe_code)]` kept | `cargo clippy -D warnings` green; candid audit 1 green |
| 9 | RGB buffer and base64 payload zeroized on drop | `Zeroizing::new(...)` for both |
| 10 | No pixel data printed or logged | `main.rs` prints only the path and a privacy notice |
| 11 | No new dependencies | `Cargo.toml` unchanged (`std::os::unix::fs` and the existing `libc` dependency) |

Pre-existing, out of scope: `ALLOWED_FHS_PREFIXES` still includes `/tmp` and `/home` (STO-04) so an
explicit `--output` under them is accepted; `encode_bmp` remains unused (STO-17).

Clearance: CLEARED.

## 6. Implementation

- `crates/enrollment-cli/src/args.rs`: `DebugVisionArgs { --output, --embed-frame }`;
  `Commands::DebugVision(DebugVisionArgs)`.
- `crates/enrollment-cli/src/error.rs`: `DebugReportRefused { path, reason }`.
- `crates/enrollment-cli/src/service.rs`: constants, `ensure_debug_report_dir`,
  `resolve_debug_report_path`, `write_debug_report`; `debug_vision` validates the path first, zeroizes
  the RGB buffer, propagates detector errors, embeds the frame only on request, returns the `PathBuf`.
  The leftover comment block was removed.
- `crates/enrollment-cli/src/html_report.rs`: `generate_html_report` takes `Option<&str>`; geometry-only
  rendering with an opt-in notice, or frame rendering with a privacy warning; stale comment removed.
- `crates/enrollment-cli/src/main.rs`: passes the arguments, prints the mode and the privacy notice.
- `crates/enrollment-cli/src/lib.rs`: re-exports.
- Docs: `Docs/ENROLLMENT_CLI.md` (new `debug-vision` section with the filesystem and privacy contract),
  `AI/VERIFICATION_MATRIX.md` (EN12–EN14).

Notable decision: the default file name is timestamped (`soos-debug-<unix_secs>-<pid>.html`) because
`O_EXCL` makes a fixed name fail on the second run; the administrator can still pick a name with `--output`.

## 7. Candid Review

Layer 1 `./scripts/candid_review.sh`: `Candid Review PASSED` on the working tree (audits 1–7 green:
no unsafe in business crates, no panics in PAM paths, no async in PAM, no OpenCV/nokhwa, shell syntax,
PAM output isolation, English-only policy). Layer 2 (fingerprint-bound `AI/candid_review_report.md`
via `./scripts/candid_subagent.sh`) is produced by the release step before push, as the repository
workflow mandates; findings and their resolution are recorded there.

## 8. Verification Results

```bash
cargo fmt --all -- --check                                                       # clean
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings     # Finished, 0 warnings
cargo test   --locked -p soos-enrollment-cli --all-features --test debug_vision_tests  # 11 passed; 0 failed
cargo test   --locked --workspace --all-targets --all-features --no-fail-fast     # 104 result lines, 490 passed, 0 failed, exit 0
./scripts/candid_review.sh                                                       # Candid Review PASSED
```

During development the `debug_vision_tests` suite was additionally run 10 times in a row (11/11 each
time) to rule out flakiness. Docker PAM matrix (`./run_tests.sh`) not run: the PAM module is not
touched by this change.

## 9. Known Limitations / Follow-ups

- `validate_fhs_path` still accepts `/tmp` and `/home` prefixes (STO-04); once that list is tightened,
  `--output` inherits the stricter policy automatically.
- Walkthrough 11 states the report lands in `/tmp/soos-debug.html`; that statement is superseded by
  this walkthrough (the historical record is left unchanged).
- `encode_bmp` in `html_report.rs` is dead code (STO-17) and is left for that issue.
- The parent-directory symlink check covers the final directory component only; intermediate
  components of an explicit `--output` are the administrator's responsibility.
