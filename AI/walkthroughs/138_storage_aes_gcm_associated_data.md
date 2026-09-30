# Walkthrough 138 — AES-GCM Associated Data for Stored Templates and Evidence

- **Date**: 2026-09-30
- **Issue**: GitHub #266 (review finding STO-22, severity SUGGESTION)
- **Branch**: `fix/p3-storage-aad`
- **Matrix criteria**: SAD1–SAD7 (new component `storage-aad-binding`)
- **ADR**: 2026-09-30 "AES-GCM Associated Data for Stored Templates and Evidence"

---

## 1. Finding

`encrypt_payload` / `decrypt_payload` in `soos-biometric-store` and `soos-evidence-store` called
`cipher.encrypt(nonce, plaintext)` with no associated data. A template copied to another UID's
file was caught only by the plaintext UID check after decryption and CBOR parsing, and an evidence
snapshot had no binding to its date partition or snapshot id at all.

## 2. Specification

- Payload format version 2 (`PAYLOAD_FORMAT_VERSION = 2`) in both crates, marked by the 4-byte
  `BOUND_FORMAT_MARKER = b"AAD\x02"` right after the unchanged 8-byte magic (`SOOSBIO1`,
  `SOOSEVD1`). The magic is kept because existing readers and contract tests identify files by it.
- Templates: `MAGIC || marker || uid (BE u32) || nonce || ciphertext+tag`, AAD
  `template_aad(uid)` = `"soos/biometric-template" || 0x00 || clear header`. New public items:
  `PayloadFormat { LegacyV1, BoundV2 }`, `template_aad`, `encrypt_template_payload`,
  `decrypt_template_payload`, `BiometricStore::template_format`,
  `BiometricStore::migrate_legacy_template`.
- Evidence: `MAGIC || marker || nonce || ciphertext+tag`, AAD `snapshot_aad(date, id)` with
  big-endian `u64` length prefixes. New public items in `crypto`: `PayloadFormat`,
  `snapshot_aad`, `encrypt_snapshot_payload`, `decrypt_snapshot_payload`.
- `encrypt_payload` / `decrypt_payload` stay as the legacy unbound (version 1) codec, never used
  by a store.

### Why the UID is in clear in the template header

An existing contract test (`bounded_read_tests::test_get_metadata_refuses_uid_mismatch`) expects a
renamed template to be refused with `CorruptFile`, and another
(`test_get_accepts_file_at_exact_bound_boundary_only_below`) expects random bytes after the magic
to fail with `Crypto`. With the AAD derived only from the file name, both cases fail
authentication identically. Authenticating under the UID declared in the header and then comparing
it with the file UID gives `CorruptFile` for a moved template (before CBOR parsing, as the issue
asks) and `Crypto` for junk or a rewritten header.

## 3. Migration (no template is ever lost)

- A payload without the marker is decoded with the legacy codec and reported as `LegacyV1`; the
  plaintext UID check still applies.
- A payload with the marker that does not authenticate as bound is retried as legacy, because a
  legacy random nonce begins with the marker with probability 2^-32.
- The next `enroll` writes version 2 (atomic replace, best-effort overwrite of the legacy inode).
  `migrate_legacy_template(uid)` re-encrypts a legacy template without changing it and refuses
  (without rewriting) a legacy file whose embedded UID does not match.
- Legacy evidence is read but never rewritten; it expires with the 7-day retention.

## 4. Anti-rollback: what is and is not detected

| Scenario | Detected |
|---|---|
| Bound template moved or copied to another UID | Yes, `CorruptFile` before CBOR parsing |
| Clear UID rewritten, bit flip, truncation, wrong key | Yes, `Crypto` |
| Bound snapshot moved to another date partition or renamed to another id | Yes, `Crypto` |
| Older still-valid template of the same UID restored | No (same AAD) |
| Older snapshot restored at its own path, a whole directory restored, files deleted | No |
| Legacy (unbound) file moved | Templates: caught by the plaintext UID check. Evidence: no |

A monotonic counter in the AAD would only help if the counter lived outside the attacker's reach
(TPM NV index or similar); a counter file beside the templates is rolled back with them. Only root
can write the stores, and root attackers are out of scope, so none was added (SAD7 is Pending).

## 5. TDD Evidence

Red, on `origin/main` (abd6016) with the new test files only:

- `crates/biometric-store/tests/aad_binding_tests.rs`: 3 of 7 failed
  (`test_sad_enrolled_template_declares_bound_format_and_uid`,
  `test_sad_enrolled_template_is_not_readable_as_unbound_payload`,
  `test_sad_reenroll_upgrades_legacy_template_to_bound_format`); the other 4 are regression
  guards for behavior that must be kept.
- `crates/evidence-store/tests/aad_binding_tests.rs`: 4 of 5 failed (bound marker, unbound
  decrypt, moved partition, renamed id).
- `aad_migration_tests.rs` and `aad_codec_tests.rs`: compile failure (new API absent).
- `soos-invariants::storage_aad_contract::test_storage_docs_state_the_rollback_limit` cannot
  pass on the `origin/main` documents: neither crate document contains "Associated Data".

Green: all of the above pass, and every pre-existing test is unchanged and passes.

## 6. Audit Notes

- No `unwrap`/`expect`/indexing in the new production code: slices use `get`, the clear UID is
  read with `try_into`, lengths use checked or saturating arithmetic, `Nonce::from_slice` only
  receives slices checked to be `NONCE_LEN` bytes.
- Reads stay bounded by `MAX_TEMPLATE_FILE_BYTES` / `MAX_EVIDENCE_FILE_BYTES` before decryption;
  plaintexts stay in `Zeroizing` buffers. Error messages carry UIDs only, never key material,
  embeddings or frames.
- Both crates keep `#![forbid(unsafe_code)]`. No PAM, daemon or socket code changed.

## 7. Follow-ups

- The daemon does not call `migrate_legacy_template` at start-up; legacy templates are upgraded
  on the next enrollment. An operator command (`soos-enroll migrate`) could be added later.
