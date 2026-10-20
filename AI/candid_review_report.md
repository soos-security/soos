# Candid Review Report

- **Date**: 2026-10-20
- **Target Branch**: `fix/webpki-advisories`
- **Base (merge-base)**: `da320a4`
- **Reviewed-Diff-Fingerprint**: `4b02338bc89f3ed648cc3e4f81204579414cd2caf3f75717666174e9868c78e4`
- **Audited Files**: `Cargo.lock`, `AI/walkthroughs/247_webpki_advisories.md`

## 1. Executive Summary

Lockfile-only security bump for RUSTSEC-2026-0381/0382/0383 (CVE-2026-111943, emSign distrust):
`rustls-webpki` 0.103.15 -> 0.103.16, `webpki-roots` 1.0.9 -> 1.0.10, `webpki-root-certs`
1.0.9 -> 1.0.10, plus walkthrough 247. Every claim checked against the published crate sources,
the local advisory database and the workspace code holds. No findings of any severity.

## 2. Test Changes

Mechanical listing on `target/candid_diff.patch`:

- Test files touched: none.
- Removed/changed assertions (`assert`, `#[test]`, `proptest!`, `should_panic`): none.
- New escape hatches (`#[ignore]`, `cfg(any())`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.

No test was added, modified or removed.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Advisory ranges verified in `~/.cargo/advisory-dbs/.../RUSTSEC-2026-038{1,2,3}.md`:
  `webpki-roots` and `webpki-root-certs` patched `>= 1.0.10`; `rustls-webpki` patched
  `>= 0.103.16, < 0.104.0-alpha.1`. The new versions are the lowest patched releases. PASS.
- Checksums: SHA-256 of the cached `.crate` files for all six versions (old and new) match the
  `checksum` lines in the diff exactly. PASS.
- Manifests: `rustls-webpki` 0.103.15 vs 0.103.16 `Cargo.toml` identical except `version`;
  `webpki-roots`/`webpki-root-certs` differ only in `version` and `rust-version` 1.70 -> 1.71
  (well under workspace MSRV 1.92). Dependency lists in the lockfile entries unchanged. PASS.
- Source behaviour: `rustls-webpki` 0.103.16 adds `DISTRUSTED_SPKIS` (eight emSign SPKIs),
  checks the path head and the trust anchor, and adds `Error::CertDistrusted`; `webpki-roots`
  1.0.10 and `webpki-root-certs` 1.0.10 only remove `emSign ECC Root CA - G3` (no anchor added).
  rustls 0.23.46 maps unknown webpki errors through its `_ => CertificateError::Other(..)` arm. PASS.
- Push sender: no `TlsConfig`, `root_certs`, `RootCerts` or `disable_verification` anywhere in
  `crates/`; `rustls-platform-verifier` absent from `Cargo.lock`. `send_error_outcome`
  (`crates/push-sender/src/lib.rs:227`) maps any non-resolver error to `Outcome::Retry`, so a
  `CertDistrusted` failure is handled like any other TLS failure, never as success. PASS.

### PAM Concurrency & Deadlines

No change to `crates/pam` or its dependency graph (these crates reach the workspace only via
`ureq` in `soos-push-sender` and the `ort-sys` build script). PASS.

### Panic Safety & Fail-Closed

No production code changed. The new verification error strictly reduces trust (fail-closed). PASS.

### Test Integrity & Anti-Weakening

See section 2: zero test changes. PASS.

### Memory, Bounds & Secrets

No code or logging change; `rustls`/`ureq` log targets remain unchanged. PASS.

### Supply Chain & Automation

- `deny.toml` untouched; still `ignore = []`.
- `cargo deny --locked check` re-run on this worktree: `advisories ok, bans ok, licenses ok, sources ok`.
- `cargo metadata --locked`: accepted.
- `git diff --stat origin/main -- Cargo.lock`: 6 insertions / 6 deletions, exactly the three
  version + checksum pairs; no unrelated churn (the `getrandom` re-resolution mentioned in the
  walkthrough is absent from the diff). Source stays the crates.io registry. PASS.

### English-Only Policy

Walkthrough 247 is in English; numbering follows 246 with no collision; the
"not registered in `scripts/sync_issue.py`" wording matches the walkthrough 211 precedent. PASS.

## 4. Detailed Findings & Action Items

None.

Note (not a finding): the walkthrough's `cargo test` count (869 passed) was not re-run by this
reviewer; the lockfile change alters no API used by the workspace, and CI re-runs the suites.

## 5. Final Verdict

**VERDICT: APPROVED**
