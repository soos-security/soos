# Walkthrough 247 — webpki crates for RUSTSEC-2026-0381, RUSTSEC-2026-0382 and RUSTSEC-2026-0383

- **Date**: 2026-10-20
- **Issue**: GitHub-only #462, no backlog id. **Branch**: `fix/webpki-advisories`. It is not registered in `scripts/sync_issue.py`; the commit message carries `Closes #462`.
- **Matrix criteria**: none (lockfile-only security bump; the existing `cargo deny` gate and the test suites are the contract).
- **Precedent**: walkthrough 211 (#395/#396, the rustls 0.23.46 bump), followed step by step.

## 1. Context & Objectives

Since 2026-10-20, `cargo deny --locked check` fails on `main` and on every open pull request. `Cargo.lock` pinned three crates affected by new advisories, all aliases of CVE-2026-111943 (emSign intermediates whose private keys are now well known):

- **RUSTSEC-2026-0382**: `rustls-webpki` 0.103.15. Certificate verification may accept paths through the distrusted emSign intermediates. Patched in `>= 0.103.16, < 0.104.0-alpha.1` and `>= 0.104.0-alpha.8`.
- **RUSTSEC-2026-0381**: `webpki-roots` 1.0.9. It bundles `emSign ECC Root CA - G3`, the root of those intermediates. Patched in `>= 1.0.10`.
- **RUSTSEC-2026-0383**: `webpki-root-certs` 1.0.9. Same root, same fix. Patched in `>= 1.0.10`.

`deny.toml` keeps `ignore = []`; no advisory is waived.

## 2. Architect Design

No type, trait, constant or invariant changes. The three crates reach the workspace only through `ureq` 3.4.2 (exact pin in the workspace `Cargo.toml`), which has two dependents:

- `soos-push-sender` (normal dependency, features `["rustls"]`): `rustls` 0.23.46 → `rustls-webpki` and `webpki-roots`.
- `ort-sys` 2.0.0-rc.13 (build dependency, `tls-native`): `ureq`'s `native-tls` feature → `webpki-root-certs`. It is the build script that downloads the prebuilt ONNX Runtime; nothing from it ships in a binary.

The fix is three lockfile-only patch bumps inside the existing semver ranges:

| Crate | Before | After | Choice |
|---|---|---|---|
| `rustls-webpki` | 0.103.15 | 0.103.16 | lowest patched 0.103.x (advisory `patched`) |
| `webpki-roots` | 1.0.9 | 1.0.10 | lowest patched release |
| `webpki-root-certs` | 1.0.9 | 1.0.10 | lowest patched release |

MSRV, features and dependency graph, checked against the published manifests:

- `rustls-webpki` 0.103.16: `rust-version = "1.71"`, unchanged from 0.103.15. Features and dependencies unchanged; the manifests differ only in `version`.
- `webpki-roots` and `webpki-root-certs` 1.0.10: `rust-version` 1.70 → 1.71. Features and dependencies unchanged (only `rustls-pki-types`).
- The workspace MSRV stays 1.92 (ADR 2026-10-13); 1.71 is far below it.
- The dependency lists of the three lockfile entries are identical before and after; no package is added or removed.

## 3. Plan Evaluation

Condensed for a lockfile-only change. Alternatives rejected:

- Waiving the advisories in `deny.toml`: forbidden while fixed releases exist.
- Bumping `ureq` or `rustls`: unnecessary; `ureq` is exactly pinned because its resolver API is unversioned, and 3.4.2 with rustls 0.23.46 already accept the patched versions.
- Switching the push sender to the platform verifier: a design change outside the scope of an advisory fix.
- Keeping Cargo's full re-resolution: as in walkthrough 211, `cargo update` also re-resolved `tempfile`'s `getrandom` edge (0.4.3 → 0.3.4). That unrelated hunk was reverted by hand; `cargo metadata --locked` confirms the lockfile stays consistent without it.

Verdict: APPROVED.

## 4. Tester Contract

No new test. The contract is the existing gates:

- `cargo deny --locked check` (advisories must pass with `ignore = []`);
- the `soos-push-sender` tests, which build `UreqDeliverer` (and so the rustls client configuration with the webpki roots) and map every `ureq` error class;
- the `soos-invariants` repository checks.

Red evidence: `cargo deny --locked check` on `main` (da320a4) fails the advisories check on the three advisories above.

## 5. Auditor Constraints

1. **Trust roots of the push sender.** `soos-push-sender` neither pins nor customises its trust roots. `UreqDeliverer::build` sets no `TlsConfig`, so `ureq` uses its default: provider rustls (ring), `RootCerts::WebPki`, SNI on, verification on. `RootCerts::WebPki` is the root set compiled in from `webpki-roots`. The platform verifier is not used: `ureq`'s `platform-verifier` feature is off and `rustls-platform-verifier` is absent from `Cargo.lock`. The push sender's `ureq` build has only the `rustls` feature set; `native-tls` and `webpki-root-certs` are enabled only for the `ort-sys` build-script edge (resolver 2 keeps build-dependency features separate). No source in the workspace configures `root_certs`, a custom verifier or `disable_verification`. Met.
2. **TLS behaviour change, push sender.** Two effects, both limited to the emSign hierarchy:
   - `webpki-roots` 1.0.10 removes exactly one trust anchor, `emSign ECC Root CA - G3`; no anchor is added. A push endpoint whose chain ends only at that root now fails verification.
   - `rustls-webpki` 0.103.16 rejects any path whose certificates or trust anchor carry one of eight distrusted emSign intermediate public keys, with the new `Error::CertDistrusted`. The rest of the path-building code is unchanged. `webpki::Error` is `#[non_exhaustive]`, and rustls 0.23.46 maps unknown variants to `CertificateError::Other` through its catch-all arm, so no new code path appears in rustls or `ureq`.
   In `soos-push-sender`, any TLS failure is already a `ureq` error other than the resolver marker, so `send_error_outcome` classifies it as `Outcome::Retry`, exactly as an unknown-issuer failure did before. No other behaviour (ciphers, protocol versions, SNI, timeouts, redirects, proxy) changes. Met.
3. **Build-time path.** `webpki-root-certs` feeds only the `native-tls` client of the `ort-sys` build script, which runs only at build time; the only effect is that a download server chaining to the removed root would now be refused. Shipped binaries do not contain it. No constraint.
4. **No logging change.** `install_logging` still sets the `ureq`, `ureq_proto` and `rustls` targets to `OFF`; the new error variant is never formatted by soos code. Met.
5. **No unrelated lockfile churn.** Exactly three package entries change (version and checksum lines only); the `tempfile` `getrandom` hunk was reverted. Met.
6. **Checksum source.** The new checksums come from the crates.io index through `cargo update`. Met.

Clearance: CLEARED.

## 6. Implementation

- `Cargo.lock`: `rustls-webpki` `0.103.15` → `0.103.16`, `webpki-roots` `1.0.9` → `1.0.10`, `webpki-root-certs` `1.0.9` → `1.0.10` (version and checksum lines only).
- `AI/walkthroughs/247_webpki_advisories.md`: this walkthrough.

No source, `Cargo.toml`, `deny.toml`, `Docs/` or ADR change. `AI/DECISIONS.md` has no entry for earlier dependency bumps, and this one changes no decision.

## 7. Candid Review

See `AI/candid_review_report.md` (bound to the final diff fingerprint).

## 8. Verification Results

Run with `LIBONNXRUNTIME_NO_PKG_CONFIG=1`, `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_BUILD_JOBS=4`:

- `cargo metadata --locked`: accepted.
- `cargo deny --locked check`: `advisories ok, bans ok, licenses ok, sources ok`.
- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked -p soos-push-sender --all-targets -- -D warnings`: clean.
- `cargo test --locked -p soos-push-sender -p soos-invariants -- --test-threads=3`: 869 passed, 0 failed, 0 ignored.

## 9. Known Limitations / Follow-ups

- `ureq` stays exactly pinned at 3.4.2; future advisories in `rustls`, `rustls-webpki` or the root crates need the same lockfile-only bump.
- The push sender's trust store is compiled in: a root removal or distrust only reaches users with a new build.
- rustls, ring and the RustCrypto crates are still not independently audited (`Docs/REMOTE_COMPANION.md`).
