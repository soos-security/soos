# Candid Review Report

- **Date**: 2026-11-20
- **Target Branch**: `fix/ci-unblock`
- **Base (merge-base)**: `0cce947`
- **Reviewed-Diff-Fingerprint**: `ceae31a36dd24e803faf0cf61dc18eddc537b5ac7d7e5e63d9739e0fc97027da`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/walkthroughs/271_arch_matrix_full_upgrade.md`, `Cargo.lock`, `Cargo.toml`,
  `Docs/CI_CD_AND_SECURITY.md`, `Docs/PACKAGING_AND_PROVISIONING.md`,
  `crates/remote/src/webpush.rs`, `crates/remote/tests/common/passkey.rs`,
  `crates/remote/tests/common/push.rs`, `deny.toml`, `tests/docker/setup_packages_test.sh`,
  `tests/invariants/src/remote_passkey_contract.rs`,
  `tests/invariants/src/remote_push_contract.rs`, `tests/invariants/src/setup_deps_contract.rs`

## 1. Executive Summary

The diff unblocks two required CI checks: (a) the Arch leg of the `setup-packages` Docker matrix
now runs a full `pacman -Syu` instead of the partial-upgrade `pacman -Sy`, guarded by a new
invariant SDP30; (b) RUSTSEC-2026-0387 (`crypto-bigint` timing variability) is resolved by
moving the workspace `p256` from `0.13.2` to `0.14.0`, without any advisory exception.

Claims verified independently, not taken from the walkthrough:

- The local advisory database entry for RUSTSEC-2026-0387 lists `patched = [">= 0.7.6"]`;
  `Cargo.lock` now locks only `crypto-bigint 0.7.11`. `crypto-bigint 0.5.5` and `der 0.7.10`
  are gone from the lock (only `der 0.8.2` remains).
- `deny.toml` `[advisories] ignore` is untouched; `cargo deny --locked check` →
  `advisories ok, bans ok, licenses ok, sources ok` (run by the reviewer).
- Every new `[bans] skip` (`block-buffer@0.12.1`, `cpufeatures@0.3.1`, `crypto-common@0.2.2`,
  `digest@0.11.3`, `hmac@0.13.0`, `rand_core@0.10.1`, `sha2@0.11.1`) is a locked version with a
  second locked version, and each reason names real direct dependents (checked against the lock
  diff; enforced by `dependency_tooling_contract`, which passes).
- `p256` is used only by `crates/remote`; no other manifest is affected. No OpenSSL, `ring`,
  `aws-lc` or `rand` enters the `soos-remote` normal tree (RMC-S13 passes with its `cargo tree`
  branch).
- `cargo test --locked -p soos-remote -p soos-invariants -- --test-threads=4`: 46 suites,
  1386 passed, 0 failed (includes RFC 8291 Appendix A, the WebAuthn known-answer vector, the
  high-S `n - s` acceptance test and the VAPID signature checks).
- `cargo clippy --locked -p soos-remote -p soos-invariants --all-targets --all-features --
  -D warnings` clean; `cargo fmt --all -- --check` clean.

Not verified by the reviewer: the Arch Docker container run end to end (no Docker run in this
review); it is exercised by the required `setup-packages` CI job itself.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Test files touched (frozen patch):

| File | Change | ADR 2026-11-20 item (4) | Verdict |
|---|---|---|---|
| `tests/invariants/src/remote_passkey_contract.rs` Test 46 (RMC-S13) | expected `p256` line `0.13.2` → `0.14.0` in the `assert!`, its message string and the doc comment | listed | Matches |
| same file, Test 47 (RMC-S14) | `test_rmc_s14_deny_skip_for_der_is_documented` (exactly one `der@0.7.10` skip naming `ecdsa`, `sec1`, `ureq`) → `test_rmc_s14_no_stale_der_skip_remains` (zero such skips) | listed ("inverted to `test_rmc_s14_no_stale_der_skip_remains`") | Matches |
| same file, module doc (`//!` lines for RMC-S13/S14) | text updated to `0.14.0` / "no stale skip" | not listed explicitly; documentation of the two listed tests, no assertion | Accepted (cosmetic, consistent with the listed edits) |
| `tests/invariants/src/remote_push_contract.rs` Test 43 (RMC-S33) | expected `p256` line `0.13.2` → `0.14.0`; message unchanged | listed | Matches |
| `crates/remote/tests/common/passkey.rs` | `to_encoded_point(false)` → `to_sec1_point(false)` | listed (helpers, setup only) | Matches |
| `crates/remote/tests/common/push.rs` | import `ToEncodedPoint` → `ToSec1Point`, same call rename | listed | Matches |
| `tests/invariants/src/setup_deps_contract.rs` | NEW test `test_sdp30_arch_matrix_never_partially_upgrades` (additive, no existing line removed) | not an edit of an existing test | Additive contract, no weakening |

Removed/changed assertions (`grep '^-[^-].*(assert|#[test]…)'`): only the Test 47 body
(`assert_eq!(lines.len(), 1, …)` and the per-needle loop), covered by the ADR. No `#[ignore]`,
`#[cfg(any())]`, `should_panic`, tolerance or epsilon was added. No inline `mod tests` touched.

The diff matches exactly what the 2026-11-20 ADR lists, plus the cosmetic module/doc comments of
the same tests and one purely additive test. No other test change exists.

Justification of the Test 47 inversion: the old contract (exactly one `der@0.7.10` skip) cannot
hold once `der 0.7` leaves the lock, because the pre-existing
`dependency_tooling_contract::test_deny_skip_entries_are_locked_versions` rejects any skip that is
not locked. The two contracts were mutually exclusive after the upgrade; the inversion keeps a
regression guard against a stale skip. The skip-reason correctness the old test partly covered is
still enforced generically by `dependency_tooling_contract`.

## 3. Deep Reasoning Audit

### Logic & Architecture
- `webpush.rs` `uncompressed()`: `to_sec1_point(false)` is the renamed SEC1 encoder; the
  65-byte output is pinned by the RFC 8291 Appendix A vector, which passes → PASS.
- `hmac_sha256`: the output is still copied into a `Zeroizing<[u8; 32]>`; only the wipe of the
  intermediate `GenericArray` changed from a `Zeroizing` drop to an explicit
  `as_mut_slice().zeroize()` after the copy. Scenario: an early return between `finalize` and
  the wipe would leave the buffer unwiped; none exists (the copy loop cannot fail or panic) → PASS.
- `generic-array` lost its `zeroize` feature in the lock; any other `Zeroizing<GenericArray>` would
  fail to compile. The crate builds, and `SecretKey::to_bytes()` (line 118) now returns a
  `hybrid-array` type that implements `Zeroize` (`hybrid-array` depends on `zeroize`) → PASS.
- WebAuthn: ecdsa 0.17 could have started refusing high-S signatures (WebAuthn authenticators may
  emit them). `test_rmc_verify_assertion_accepts_a_valid_uv_assertion` verifies both `s` and
  `n - s` and passes → PASS.
- `SecretKey::from_slice` is always given exactly 32 bytes; `from_sec1_bytes` inputs are bounded
  SEC1 points; no exposure to changed length semantics → PASS.
- SDP30 classifier: tried `-Sy`, `-yS`, `--sync --refresh`, `-Syu`, `--sync --refresh
  --sysupgrade`, the new comment lines (back-quoted `` `pacman ``, not the bare word, so ignored),
  `pacman -Si`, `pacman -S --needed` (no refresh) → behaves as intended. Gap: split short flags
  (`pacman -S -y`) are not recognised as a refresh (SUGGESTION S1).

### PAM Concurrency & Deadlines
- No file under `crates/pam` or the daemon changes; no Tokio/thread/IPC change → PASS (N/A).

### Panic Safety & Fail-Closed
- No new `unwrap`/`expect`/indexing in production code (`webpush.rs` adds none). Errors still map
  to `WebPushError`/`WebAuthnError`; nothing can reach an `Allow`/`PAM_SUCCESS` → PASS.
- `expect` calls in `crates/remote/tests/common/*` are pre-existing test helpers → PASS.

### Test Integrity & Anti-Weakening
- See §2. Every changed assertion is listed in the ADR; the remaining edits are doc comments of
  those tests and one new additive test. SDP30 fails against the old `pacman -Sy` line (the
  `-Sy` case is asserted as `(true, false)` and the script loop rejects it) → PASS.
- The owner approval itself is recorded in an ADR authored in the same diff; the reviewer cannot
  independently confirm the owner's statement (MINOR M1).

### Memory, Bounds & Secrets
- HMAC output buffer is still wiped; ECDH x-coordinate and VAPID secret handling unchanged; no new
  log line or `Debug` added → PASS.

### Supply Chain & Automation
- Sources: all new packages from crates.io (`sources ok`). Licenses: `licenses ok`.
- No advisory exception; `multiple-versions = "deny"` kept; seven new skips documented and
  validated by the dependency tooling contract; the `rand_core` three-version documentation is
  added to the comment block.
- No `.github/`, `scripts/` or `.githooks/` change. The `tests/docker/setup_packages_test.sh`
  change is a one-flag change aligned with `tests/docker/Dockerfile.arch` and
  `tests/docker/Dockerfile.system-ort` (both already `pacman -Syu`, verified) → PASS.
- Note: a full `-Syu` on a digest-pinned image makes the Arch leg depend on the live repository
  state, which package resolution already did; documented → acceptable.

### English-Only Policy
- All added code, comments, docs, ADR, matrix rows and walkthrough are in English → PASS.

## 4. Detailed Findings & Action Items

- **[MINOR] M1** `AI/DECISIONS.md:162` (new 2026-11-20 entry, item (4)) — the owner approval of
  the test edits is asserted only by the ADR written in this same diff. The edits are technically
  justified (§2) and exactly scoped, but the lead must confirm the owner decision of 2026-11-20
  exists before merging.
- **[SUGGESTION] S1** `tests/invariants/src/setup_deps_contract.rs` (`pacman_refresh_upgrade`) —
  the classifier requires `S` and `y` in the same short-option word, so `pacman -S -y` would pass
  undetected. Optional: accumulate `S`/`y`/`u` across all short-option words of the line.
- **[SUGGESTION] S2** `AI/architect_spec_remote_passkey_funnel.md`, `AI/research_webauthn.md`,
  `AI/auditor_constraints_remote_passkey_funnel.md` still describe `p256 0.13.2` and the
  `der@0.7.10` skip. They are historical design records superseded by the new ADR; optionally add a
  pointer to the 2026-11-20 ADR.
- **[SUGGESTION] S3** `AI/VERIFICATION_MATRIX.md` — the new SDP30 row sits between SDP29 and
  SDP26; cosmetic ordering only.

No CRITICAL or MAJOR finding.

## 5. Final Verdict

**VERDICT: APPROVED**
