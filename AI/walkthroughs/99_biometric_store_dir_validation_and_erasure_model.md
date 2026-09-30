# Walkthrough 99 — Biometric Store Directory Validation and Honest Erasure Model

- **Date**: 2026-09-30
- **Issues**: Review findings STO-04 (GitHub #178) and STO-06 (GitHub #179) — **Branch**: `fix/biometric-store-perms-erasure`
- **Matrix criteria**: B8, B9, B10 (new, ✅ Verified); B6 and EN4 (scope corrected)
- **ADRs**: 2026-09-30 "Biometric Template Erasure Model" and "Biometric Store Directory Is Validated, Never Chmod-ed" in `AI/DECISIONS.md`

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, area Storage & CLIs)
confirmed two MAJOR findings in `crates/biometric-store/src/store.rs`:

- **STO-04**: `BiometricStore::new` ran `set_permissions(base_dir, 0o700)` on any existing
  directory, ignoring the result. A probe showed a `1777` directory rewritten to `0700`, so
  `sudo soos-enroll --biometrics-dir /tmp list` would drop the sticky bit of `/tmp` and lock every
  non-root user out of it. Ownership was never checked.
- **STO-06**: `enroll` replaced an existing template by rename, so the previous inode's blocks were
  freed without any overwrite (a hard-link probe still read the old ciphertext). The `delete` doc
  comment, `Docs/ENROLLMENT_CLI.md` and `soos-enroll delete` ("securely shredded") promised that
  deleted templates could not be recovered from disk sectors, which in-place overwrite cannot
  deliver on copy-on-write filesystems, data journaling or wear-levelled flash.

Objectives: never modify an administrator-chosen directory; overwrite the replaced inode on
re-enrollment without losing atomicity; state the real erasure guarantee everywhere.

## 2. Architect Design

- `BiometricStore::new`: `symlink_metadata` first. Missing → `create_store_dir` (parents with
  default permissions, final component `DirBuilder::mode(0o700)` then exact `0700`, an
  `AlreadyExists` race falls through to validation). Always → `validate_store_dir(path, facts,
  euid)`: refuse symlink, non-directory, owner not in `{0, euid}`, `mode & 0o022 != 0`, all with
  `BiometricStoreError::InvalidPath`. No chmod on an existing directory. The effective UID comes
  from `nix::unistd::geteuid()` (safe API, `nix` is already a workspace dependency).
- `StoreDirFacts` (private) decouples the rule from `std::fs::Metadata` so a foreign owner can be
  unit-tested without root.
- New public constants `STORE_DIR_MODE = 0o700`, `FORBIDDEN_STORE_DIR_BITS = 0o022`.
- Erasure: shared helpers `open_existing_template_for_overwrite` (regular file only,
  `O_NOFOLLOW | O_NONBLOCK`, inode/device re-checked after `open`), `overwrite_file_contents`
  (3 CSPRNG passes, `fsync` each) and `sync_dir`. `enroll` opens the old inode **before** the
  rename, commits (temporary file + `fsync` + rename + directory `fsync`), then overwrites the old
  inode through the retained handle. `delete` reuses the helpers and syncs the directory after
  unlinking. The temporary file is removed if the write or rename fails.
- `BiometricTemplate::to_cbor` returns `Zeroizing<Vec<u8>>`, reserves the buffer up front (no
  reallocation copies) and zeroizes the wire struct's embedding copy.
- Rejected: per-template data keys stored in the template file (deleting the key and the file are
  the same operation on the same blocks). The master key stays the single crypto-erasure point.

## 3. Tester Contract (Red Phase)

| Test | Red evidence before the fix |
|---|---|
| `directory_validation_tests::test_new_refuses_world_writable_sticky_dir_and_leaves_mode_unchanged` | FAILED: `new()` returned `Ok` (and rewrote `1777` to `0700`) |
| `directory_validation_tests::test_new_refuses_group_writable_dir_and_leaves_mode_unchanged` | FAILED: `Ok` returned |
| `directory_validation_tests::test_new_refuses_world_writable_dir_without_sticky_bit` | FAILED: `Ok` returned |
| `directory_validation_tests::test_new_accepts_safe_existing_dir_without_touching_its_mode` | FAILED: `0755` rewritten to `0700` |
| `directory_validation_tests::test_new_preserves_setgid_bit_on_safe_existing_dir` | FAILED: `2750` rewritten to `0700` |
| `directory_validation_tests::test_new_refuses_existing_non_directory` | FAILED: a regular file accepted |
| `directory_validation_tests::test_new_refuses_symlinked_store_dir`, `test_new_creates_missing_dir_with_0700`, `test_new_ownership_rule_against_effective_uid` | Passed already (regression guards; the foreign-owner branch runs only as root) |
| `erasure_tests::test_reenroll_overwrites_previous_template_inode_before_release` | FAILED: hard-link probe still held the original ciphertext after re-enroll |
| `erasure_tests::test_first_enroll_without_previous_template_succeeds` | Passed already (regression guard) |
| `cbor_zeroize_tests::test_to_cbor_returns_zeroizing_buffer` | Compile error `E0308`: `to_cbor` returned `Vec<u8>` |
| `store::tests::test_validate_store_dir_*` (4 unit tests) | Written with the specified private API (foreign owner, writable modes, symlink, non-directory) |

No existing test was modified.

## 4. Auditor Constraints

1. No `unwrap`/`expect`/`panic` in production code; `#![forbid(unsafe_code)]` stays (euid via `nix`).
2. Never chmod a pre-existing directory; chmod only the directory this call created.
3. Opening a template for overwrite must refuse symlinks, non-regular files and a swapped inode, and
   must not block on a FIFO.
4. Re-enrollment stays atomic: the new template is committed before the old inode is touched.
5. No embedding, key or ciphertext in error messages (messages carry paths, UIDs and modes only).
6. No document or message may claim unrecoverable deletion.

## 5. Implementation (Green Phase)

- `crates/biometric-store/src/store.rs`: validation, creation, overwrite helpers, erasure-model
  module documentation, unit tests.
- `crates/biometric-store/src/template.rs`: zeroizing, pre-reserved `to_cbor`.
- `crates/biometric-store/Cargo.toml` / `Cargo.lock`: `nix` (workspace) dependency.
- `crates/enrollment-cli/src/main.rs`: delete prompt and confirmation no longer say "securely
  shred(ded)"; they state best-effort overwrite and encryption of residual copies.
- Docs: `Docs/BIOMETRIC_STORE_CRATE.md` §3.3 (directory validation) and §3.4 (erasure model),
  `Docs/ENROLLMENT_CLI.md` erasure wording, `AI/VERIFICATION_MATRIX.md` B6/EN4 corrected and
  B8–B10 added, two ADRs.

## 6. Verification

- `cargo fmt --all -- --check`: OK.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: OK.
- `cargo test --locked --workspace --all-targets --all-features --no-fail-fast`: 890 passed, 0 failed.
- `./scripts/candid_review.sh`: PASSED.

## 7. Residual Risks & Follow-ups

- `soos-enroll` still accepts broad `--biometrics-dir` prefixes (`ALLOWED_FHS_PREFIXES` in
  `crates/enrollment-cli/src/args.rs`); a root-owned `0755` directory such as `/etc` is accepted
  by the store (it is not modified). Tightening that list belongs to the enrollment CLI and is left
  for a separate change.
- `crates/gui/src/app.rs` still shows "shred and delete" / "shredded" wording in its delete
  dialog; it is owned by the GUI work stream and should adopt the ADR wording.
- `secure_shred_file` in `crates/enrollment-cli/src/shred.rs` keeps a doc comment promising
  prevention of physical recovery; same wording follow-up.
- A crash between the re-enroll commit and the overwrite of the old inode leaves the old blocks
  unoverwritten; this is within the best-effort scope of the ADR.
- Directory validation checks the final component only, not the ownership of every ancestor.
