# Walkthrough 125 — Byte-Stable Log Redaction, Validated Key Files and a Persisted Evidence Cap

- **Date**: 2026-09-30
- **Issues**: GitHub #229 (STO-13), #230 (STO-14), #234 (STO-18), area Storage & CLIs
- **Branch**: `fix/p2-storage-redaction-keys-evidence-cap`
- **Matrix criteria**: SRK1–SRK6 (new, component `storage-redaction-keys-evidence-cap`)
- **ADR**: 2026-09-30 "Byte-Stable Redaction, Validated Key Files and Persisted Evidence Cap"
- **Scope**: `soos-admin-cli` (redaction), `soos-biometric-store` and `soos-evidence-store`
  (key loading), `soos-evidence-store` (daily cap). No pre-existing test was changed.

---

## 1. Findings

| Issue | Defect | Consequence |
|---|---|---|
| #229 | `redact_kv_field`, `redact_brackets` and `redact_bearer_token` searched `input.to_lowercase()` and sliced `input` with the offsets found in the lowered copy | `İ` (2 → 3 bytes) or the KELVIN SIGN (3 → 1 byte) earlier on a line disabled redaction for the rest of it (`user=İbob password=hunter2` stayed in clear) or cut a bearer token one byte late |
| #230 | An existing master key was read without checking owner, mode or length (unbounded `read_to_end`); the evidence crate opened it without `O_NOFOLLOW` after a `symlink_metadata` check; key creation ran `create_dir_all(parent)` and forced `0700` on the created parent | A key leaked to `0644` was used silently; a symlink swap between check and open was followed; a first `soos-enroll` on an unprovisioned host made `/var/lib/soos` `0700` and broke non-root access to the models |
| #234 | The evidence daily cap was a `Mutex<HashMap<(uid, date), u32>>` incremented before any I/O | Reset on every restart, never pruned (one entry per UID and day for the daemon lifetime), and a failed write burned a slot |

## 2. Red evidence (tests written first)

| Test file | Before the fix |
|---|---|
| `crates/admin-cli/tests/redact_utf8_offset_tests.rs` | 7 of 8 failed (only `test_229_multibyte_alphanumeric_char_is_not_a_key_boundary` passed, it is a regression guard) |
| `crates/biometric-store/tests/master_key_hardening_tests.rs` | 5 of 9 failed: `0644`/group modes accepted, parents created `0700`, and the FIFO test hit its 5 s timeout because the open blocked |
| `crates/evidence-store/tests/key_hardening_tests.rs` | 5 of 10 failed (same cases) |
| `crates/evidence-store/tests/daily_cap_persistence_tests.rs` | 6 of 7 failed (restart, pruning, failed write, private counter file, symlinked and corrupt counter); the concurrency test is a guard |
| `crypto::key_open_tests::test_230_key_open_does_not_follow_symlinks` (both crates) | Mutation check: removing `O_NOFOLLOW` from the evidence open makes it fail |

The two root-dependent tests (`test_230_foreign_owned_key_is_refused` needs `chown`, and
`test_234_failed_write_does_not_consume_a_slot` needs a non-root writer) return early in the
environment where they cannot provoke the condition.

## 3. Changes

### 3.1 Redaction (#229)

`ascii_lowered(input)` returns `input.to_ascii_lowercase()`: only ASCII bytes change, so every
offset found in the copy is a valid char boundary of the original, and a non-ASCII byte can
never match an ASCII key. The key boundary test now takes the previous full character
(`input[..idx].chars().next_back()`) instead of a one-byte slice that returned `None` for any
multi-byte predecessor, so `—password=x` is redacted while `épass=x` stays an unrelated
identifier. The output format is unchanged.

### 3.2 Key files (#230)

Both crates gained `read_existing_key` and `create_missing_key_parents`:

- open with `O_NOFOLLOW | O_NONBLOCK`, then `fstat` the descriptor: regular file, owner root
  or the effective UID, `mode & 0o077 == 0`, length 32; read through `take(33)` into a
  `Zeroizing` buffer. Any violation is `KeyError` and the file is not touched.
- missing ancestors are created one by one with `KEY_PARENT_DIR_MODE` (`0755`) and set
  through an `O_NOFOLLOW | O_DIRECTORY` descriptor of the directory just created, so the result
  does not depend on the umask; an existing parent is never modified.

The evidence crate uses `nix::libc` for the flags (it has no direct `libc` dependency).

### 3.3 Evidence daily cap (#234)

The in-memory map is gone. `store_record` now takes a store-wide `write_lock`, validates the
date partition (symlink / not a directory), reads `YYYY-MM-DD/.daily_count.<uid>` (missing: 0;
symlink, oversized or non-decimal: fail closed), refuses at the cap, writes the snapshot
through an exclusive temporary file, renames it, then persists `count + 1` atomically. A
failed write or rename removes the temporary file and leaves the counter as it was; a counter
that cannot be written removes the new snapshot and returns the error. `daily_count` reads the
same file, so it reports the persisted value after a restart and `0` once retention has pruned
the partition.

## 4. Audit notes

- No `unwrap`/`expect` in production paths; every new read is bounded (33 bytes for keys,
  16 bytes for counters); no key material, frame or embedding appears in an error message.
- All three crates keep `#![forbid(unsafe_code)]`; `geteuid` comes from `nix`.
- The counter is written only inside the `0700` partition and never ends in `.enc`, so it is
  never listed, loaded or decrypted as a snapshot.

## 5. Gate

`cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`,
`cargo test --locked --workspace --all-targets --all-features --no-fail-fast` and
`./scripts/candid_review.sh` (results in the branch report).

## 6. Follow-ups

- A root-run CI job would exercise `test_230_foreign_owned_key_is_refused`; unprivileged runs
  skip it.
- The daemon writes evidence from a single process; a second writer process (none exists
  today) would need a cross-process lock (`flock` on the partition) around the counter.
