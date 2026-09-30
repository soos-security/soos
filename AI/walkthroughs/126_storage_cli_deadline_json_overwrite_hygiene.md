# Walkthrough 126 — Storage CLI Deadline, JSON Output, Dead Code and Overwrite Hygiene

- **Date**: 2026-09-30
- **Issues**: GitHub #231 (STO-15), #232 (STO-16), #233 (STO-17), #237 (STO-21)
- **Branch**: `fix/p2-cli-deadline-json-deadcode-root`
- **Matrix criteria**: CDJ1–CDJ8 (new, verified), CDJ9 (pending test change proposal)
- **ADR**: 2026-09-30 "Storage CLI Output and Overwrite Hygiene"
- **Scope**: `soos-admin-cli`, `soos-enrollment-cli`, `tests/invariants`, docs. No pre-existing
  test was modified.

---

## 1. Findings

| Issue | Defect |
|---|---|
| #231 | `soos-admin test-pam` filled `deadline_monotonic_ns` from `SystemTime` (about 1.7e18 ns), so the daemon, which compares it against `CLOCK_MONOTONIC`, never applied `--timeout-ms`. `--timeout-ms 0` failed with `EINVAL` from `set_read_timeout(0)`. |
| #232 | `status`, `test-pam` and `soos-enroll list` built JSON with `format!`, without escaping: a `"` or `\` in a path, service or model id produced invalid JSON. |
| #233 | `encode_bmp` and `build_service` were dead; `AlreadyEnrolled` was never produced and an empty `else if already_enrolled {}` branch hid the overwrite; `secure_shred_file` followed symbolic links (`metadata`, no `O_NOFOLLOW`) and used 2 passes instead of the store's 3. |
| #237 | `soos-enroll` checked root before parsing, so `--help` failed for a normal user; `import` replaced an existing template silently. |

## 2. Red evidence

Tests were written first:

- `tests/invariants/src/cli_hygiene_contract.rs`: 7 tests, all 7 failed on `origin/main`
  (`cargo test -p soos-invariants cli_hygiene`: `0 passed; 7 failed`).
- `crates/admin-cli/tests/cli_deadline_json_tests.rs`: did not compile (`nix::time` not
  enabled, `effective_timeout_ms`, `MIN_TIMEOUT_MS` / `MAX_TIMEOUT_MS` missing).
- `crates/enrollment-cli/tests/cli_hygiene_tests.rs`: did not compile (12 errors:
  `format_enrolled_json`, `import_with_overwrite`, `already_enrolled`, `replaced_existing`,
  `ImportCommand::yes` missing).
- Runtime: the `origin/main` binary `soos-enroll --help` run as a normal user printed
  `[ERROR] Root privileges (EUID 0) are required for this operation` and exited 1.

## 3. Changes

### 3.1 `soos-admin test-pam` (#231)

- `nix` gets the `time` feature in `crates/admin-cli/Cargo.toml` (no lockfile change).
- `monotonic_now_ns()` reads `CLOCK_MONOTONIC`; a failure or negative value is an error, not 0.
- `effective_timeout_ms()` clamps to `MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS` (10..=5000 ms,
  `crates/admin-cli/src/args.rs`), the same range as `crates/pam/src/config.rs`; an invariant
  compares both source files. `main.rs` warns on stderr when the value was clamped.

### 3.2 JSON output (#232)

`DaemonStatusReport::to_json`, `PamTestReport::to_json` and the new
`soos_enrollment_cli::format_enrolled_json` call `serde_json::to_string_pretty` on the structs
that already derive `Serialize`. The field names are unchanged, so `soos-gui`'s
`refresh_profiles` still parses the list.

### 3.3 Dead and duplicated code (#233)

- `encode_bmp` and `build_service` are deleted.
- `EnrollmentSummary::already_enrolled` tells the confirmation prompt that saving replaces a
  template (the CLI prints a warning line); `EnrollmentOutcome::replaced_existing` reports it
  afterwards, including with `--yes`.
- `AlreadyEnrolled` is now produced by `import` (see 3.4).
- `secure_shred_file`: `symlink_metadata` then refusal of links and non-regular files
  (`InvalidPath`), `O_NOFOLLOW | O_NONBLOCK` open, device/inode check of the opened file,
  `SHRED_PASSES = 3` CSPRNG passes like `BiometricStore::delete`.

### 3.4 Privilege order and import overwrite (#237)

- `main.rs` parses the arguments before `check_privileges(true)`.
- `Commands::Import` now holds `ImportCommand { args: ImportArgs, yes: bool }`, which
  dereferences to `ImportArgs`, so every existing `ImportArgs { .. }` literal and
  `Commands::Import(args) => args.file` match keeps compiling.
- `EnrollmentService::import_with_overwrite(args, allow_overwrite)`: a file import onto an
  enrolled UID returns `AlreadyEnrolled(uid)` before the file is read. `import(args)` is the
  strict form (`allow_overwrite = false`).
- `--file -` (the GUI helper channel) still replaces without `--yes` and reports
  `replaced_existing`. Reason: `import_helper_args` is fixed by the existing test
  `test_import_helper_args_use_stdin`, so the GUI cannot add `--yes`, and requiring it would
  break GUI re-enrollment. See section 5.

## 4. Audit

- No `unwrap`/`expect` in production code; clock and serialization failures are handled.
- No PAM crate change; no Tokio, no `unsafe`, `#![forbid(unsafe_code)]` kept.
- The JSON outputs carry metadata only (no embedding, frame or credential).
- `secure_shred_file` can no longer be used by root to overwrite a symlink target.
- The check-then-store window of the import guard is a local root-only race with no privilege
  gain (the caller is already root); it was accepted.

## 5. Open items

- **Test change proposal (CDJ9)**: add `"--yes"` to `import_helper_args` in
  `crates/gui/src/privileged.rs` and to the expected vector of
  `crates/gui/tests/import_privacy_tests.rs::test_import_helper_args_use_stdin`, then apply the
  `AlreadyEnrolled` guard to the stdin channel too.
- **Test change proposal**: STO-17 recommends deleting `shred.rs`. It is kept (hardened)
  because `crates/enrollment-cli/tests/shred_tests.rs` imports it; retiring both needs approval.

## 6. Gate

`cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features
-- -D warnings`, `cargo test --locked --workspace --all-targets --all-features --no-fail-fast`
and `./scripts/candid_review.sh` pass (see the branch report).

## Integration: user-approved test changes (2026-09-30)

- `--yes` (#237): `import_helper_args` in `crates/gui/src/privileged.rs` passes `--yes`, and the two
  assertions that pin the GUI helper command (`import_privacy_tests::test_import_helper_args_use_stdin`
  and the argument check of `test_import_finalize_pipes_embedding_without_temp_file`) expect it.
- `shred.rs` retired (#233): `crates/enrollment-cli/src/shred.rs`, `tests/shred_tests.rs`, the
  `secure_shred_file` re-export, the branch-local hygiene tests and invariant that only covered it
  are removed; the single erasure path is `BiometricStore::delete` (matrix EN4 re-pointed, CDJ5
  superseded).

## Correction (2026-09-30, candid review finding 1)

The integration above did not complete #237: `import_with_overwrite` sent `--file -` to
`import_from_reader` before it checked `--yes`, so a stdin import still replaced an enrolled
template silently. Since the GUI now passes `--yes`, the stdin exception is withdrawn. The new
`import_with_overwrite_from_reader` resolves the UID and refuses with `AlreadyEnrolled` when
`!allow_overwrite && store.exists(uid)`, before the reader is touched. `import_with_overwrite`
uses it for stdin. `import_from_reader` stays the ungated library entry point, and the CLI no
longer calls it directly. Red → green: `crates/enrollment-cli/tests/import_stdin_overwrite_tests.rs`
(4 tests) first failed to compile (no gated reader entry point), then passed. It proves that the
refused stdin import reads zero bytes and leaves the template byte-identical, and that `--yes`
replaces it. Matrix CDJ7 / CDJ9 are `✅ Verified`, and ADR "Storage CLI Output and Overwrite
Hygiene" is amended. Follow-ups (the NSS stall on the password-failed path and the
`test-pam` connect-before-deadline order) are recorded in walkthrough 121 §7.
