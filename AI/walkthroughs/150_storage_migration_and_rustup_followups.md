# Walkthrough 150 — Legacy Storage Migration Command and rustup PATH Follow-ups

- **Date**: 2026-10-01
- **Issue**: GitHub #287 ("Storage" and "Install" items; owner decisions of 2026-10-01)
- **Branch**: `fix/p3fu-storage-install`
- **Matrix criteria**: SMI1–SMI9 (new component `storage-migration-and-rustup-followups`)
- **ADRs**: 2026-10-01 "Operator-Run Migration of Legacy v1 Storage Envelopes", 2026-10-01
  "rustup Bootstrap Leaves Shell Profiles Untouched"
- **Context**: walkthrough 138 (AES-GCM AAD, legacy v1 envelopes), walkthrough 139 (pinned rustup)

---

## 1. Findings

1. Walkthrough 138 kept legacy v1 (unbound) templates and evidence snapshots readable and only
   upgraded a template on its next `enroll` (or one UID at a time through
   `migrate_legacy_template`). Legacy evidence was never rewritten. The owner decided to add an
   operator-run command that re-encrypts every v1 file to v2, with v1 staying readable by default.
2. `scripts/install_rustup.sh` ran `rustup-init` without `--no-modify-path`, so rustup appended
   `cargo/env` lines to `~/.profile`, `~/.bashrc` and similar files of whoever ran it (a previous
   test run did exactly that on a developer host). The manual bump procedure of the pinned
   `rustup-init` digests was only a one-line comment.

## 2. Phase 1 — Specification

### Store crates (no cryptography in the CLI)

- `soos-biometric-store`:
  - `enum TemplateMigration { Missing, AlreadyCurrent, Migrated, WouldMigrate }`;
  - `struct TemplateMigrationFailure { uid, error }`,
    `struct TemplateMigrationReport { dry_run, migrated: Vec<u32>, already_current: Vec<u32>, failed }`;
  - `BiometricStore::migrate_template(uid, dry_run)`: bounded read, authentication, full CBOR and
    UID validation in both modes; a legacy template is rewritten only through `enroll` (the
    existing atomic write path); `migrate_legacy_template(uid)` now delegates to it;
  - `BiometricStore::migrate_legacy_templates(dry_run)`: every UID of `list_enrolled`, per-file
    errors recorded, only a directory listing error fails the call.
- `soos-evidence-store`:
  - `struct SnapshotMigrationFailure { path, error }`,
    `struct SnapshotMigrationReport { dry_run, migrated: Vec<PathBuf>, already_current, failed }`;
  - `EvidenceStore::migrate_legacy_snapshots(dry_run)`: exclusive `flock` on the base directory
    (same as `rotate_retention`), real `YYYY-MM-DD` partitions only, files from
    `list_snapshots_for_date`; each file opened `O_NOFOLLOW | O_NONBLOCK`, bounded, authenticated
    against its path binding, record decoded; a legacy record must carry the id of its file name;
    the decrypted CBOR is re-sealed byte for byte into an exclusive `0600` temporary file
    (`write_new_file`, synced), renamed only if the path still holds the inode that was read, then
    the partition is synced;
  - `MasterKey::load_existing(path)`: the existing-key validation of `load_or_create`, never
    creates a key;
  - private `read_bounded_evidence`, now shared by `load_snapshot` (same bound and messages).

### CLI: `soos-enroll migrate`

`soos-enroll` already owns template management and opens the master key and the biometric store;
`soos-admin` is the non-biometric diagnostic CLI with no store dependency, so it is not the home
(ADR). New items: `MigrateArgs { dry_run, format, evidence_dir, evidence_key_file }`,
`Commands::Migrate`, `EnrollmentService::migrate(args, Option<&EvidenceStore>)`,
`open_evidence_store_for_migration(dir, key)` (`Ok(None)` when the key is missing),
`build_evidence_for_migration(args)` (defaults `/var/lib/soos/evidence`,
`/var/lib/soos/evidence.key`, FHS-validated), `MigrationSummary`, `StoreMigrationSummary
{ skipped, migrated, already_current, failed, failures }`, `MigrationFailureSummary { item, error }`,
`format_migration_json` (`serde_json`, the existing `--format json` convention of `list`),
`EnrollmentCliError::EvidenceStore`. Exit status 1 when anything failed.

### Install

`rustup-init -y --profile minimal --no-modify-path --default-toolchain <x.y.z>`; afterwards
`cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"`, `export PATH="${cargo_bin}:${PATH}"`, check that
`${cargo_bin}/rustc` exists and `rustc --version` runs, print the `export PATH=...` line. Digest
bump procedure in the script header and in `Docs/CI_CD_AND_SECURITY.md`.

### Invariants touched

Encrypted at rest with AAD (unchanged codecs), files `0600` created atomically, symlink-safe,
bounded reads, no embedding / frame / key in any output, root-only CLI, fail closed (a refused
file is never rewritten, a failure is a non-zero exit). No PAM, daemon or socket code changes.

## 3. Phase 2 — Tests (Red)

New test files only; no existing test file was modified except a new `mod` declaration in
`tests/invariants/src/lib.rs`:

- `crates/biometric-store/tests/bulk_migration_tests.rs` (8 tests): legacy fixtures written with
  `encrypt_payload` exactly like `aad_migration_tests.rs`. Red: compile failure on the missing
  `TemplateMigration`, `TemplateMigrationReport`, `migrate_template`, `migrate_legacy_templates`.
- `crates/evidence-store/tests/legacy_migration_tests.rs` (7 tests): legacy fixtures written like
  `aad_binding_tests::test_sad_legacy_unbound_snapshot_in_partition_is_still_readable`. Red:
  compile failure on the missing `migrate_legacy_snapshots` and `MasterKey::load_existing`.
- `crates/enrollment-cli/tests/migrate_tests.rs` (9 tests). Red: compile failure on the missing
  `MigrateArgs`, `Commands::Migrate`, `format_migration_json`,
  `open_evidence_store_for_migration`.
- `tests/invariants/src/rustup_path_contract.rs` (3 tests). Red, on the unchanged script: all 3
  fail at runtime (`rustup-init args lack --no-modify-path: -y --profile minimal
  --default-toolchain 1.98.1`, no bump procedure in the script header or the docs).

The hermetic install test stubs `curl` (copies a fake `rustup-init` that records its arguments and
creates a stub `rustc`), `sha256sum` (prints the pinned digest) and `uname`; HOME, CARGO_HOME and
RUSTUP_HOME are scratch directories under the workspace `target/`, and six shell profile files are
pre-created with a sentinel and compared byte for byte afterwards. Both the explicit `CARGO_HOME`
and the `$HOME/.cargo` default are exercised. Nothing is downloaded; the real home directory is
never touched.

## 4. Phase 3 — Audit Constraints

1. No `unwrap` / `expect` / `panic` / indexing in production code: `try_from(...).unwrap_or`,
   `is_ok_and`, `ok_or_else` only.
2. Reads bounded by `MAX_TEMPLATE_FILE_BYTES` / `MAX_EVIDENCE_FILE_BYTES` before decryption; a
   re-sealed snapshot larger than the bound is refused instead of written.
3. Plaintext stays in the stores' `Zeroizing` buffers and is dropped before the write; reports
   carry UIDs, paths, snapshot ids and error messages only (asserted for embedding values).
4. Writes reuse the stores' own paths: templates through `enroll`; evidence through
   `write_new_file` (`create_new`, `0600`, `fsync`) plus `rename` and a directory `fsync`.
   Temporary names never end in `.enc`, so a crash leaves nothing counted as a snapshot.
5. Symlinks: template paths are refused by `template_path`; symlinked partitions and snapshot files
   are skipped; snapshot files are opened `O_NOFOLLOW`; the rename happens only over the inode
   that was read.
6. The evidence key is never created by the migration; the biometric key is opened exactly like
   `list` / `delete`.
7. Root check in the service (`check_privileges`) after argument parsing, like every subcommand.
8. `#![forbid(unsafe_code)]` kept in the three crates; no new external dependency
   (`soos-enrollment-cli` gains the workspace crate `soos-evidence-store`).
9. Script: `--no-modify-path`; `PATH` exported only inside the script process; a missing or
   non-running `rustc` after install fails with exit 1.

Clearance: CLEARED.

## 5. Phase 4 — Implementation (Green)

- `crates/biometric-store/src/store.rs`, `lib.rs`: migration types and methods.
- `crates/evidence-store/src/store.rs`, `snapshot.rs`, `crypto.rs`, `lib.rs`: migration, report
  types, `MasterKey::load_existing`, shared bounded read.
- `crates/enrollment-cli/src/{args,service,main,error,lib}.rs`, `Cargo.toml`: `migrate` command.
- `scripts/install_rustup.sh`: `--no-modify-path`, exported PATH, post-install check, printed PATH
  line, digest bump procedure.
- Docs: `Docs/BIOMETRIC_STORE_CRATE.md` (bulk migration), `Docs/EVIDENCE_STORE_CRATE.md` (legacy
  migration; the former "legacy evidence is never rewritten" statement now names the migration),
  `Docs/ENROLLMENT_CLI.md` (`soos-enroll migrate`), `Docs/CI_CD_AND_SECURITY.md` (profiles never
  edited, digest bump procedure), `README.md` (PATH line).

## 6. Evidence

```bash
export CARGO_BUILD_JOBS=4
cargo test --locked --all-features -p soos-biometric-store -p soos-evidence-store \
  -p soos-enrollment-cli -p soos-invariants
cargo clippy --locked --all-targets --all-features -p soos-biometric-store \
  -p soos-evidence-store -p soos-enrollment-cli -p soos-invariants -- -D warnings
cargo fmt --all -- --check
bash -n scripts/install_rustup.sh
./scripts/candid_review.sh
```

All green; every pre-existing test is unchanged and passes.

## 7. Limits and Follow-ups

- A legacy snapshot carries no date-partition binding: a legacy file moved to another partition
  before the migration is bound to the partition where it is found (its id is checked).
- The superseded legacy evidence ciphertext is not overwritten in place (templates keep the
  best-effort overwrite through `enroll`); it stays encrypted under the evidence key.
- `soos-enroll migrate` opens the biometric master key like `list` and `delete`
  (`load_or_create`), so on a host without a master key one is created, as those commands
  already do.
- The migration is not serialized against a concurrent `soos-enroll enroll` / `import` of the same
  UID (documented); the daemon may keep running.
