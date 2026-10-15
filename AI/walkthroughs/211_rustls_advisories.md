# Walkthrough 211 — rustls 0.23.46 for RUSTSEC-2026-0346 and RUSTSEC-2026-0347

- **Date**: 2026-10-15
- **Issue**: GitHub-only #395, no backlog id. **Branch**: `fix/rustls-advisories`. It is not registered in `scripts/sync_issue.py`; the commit message carries `Closes #395`.
- **Matrix criteria**: none (lockfile-only security bump; the existing `cargo deny` gate and the workspace test suite are the contract).
- **Numbering**: 211 was reserved for this change; lower free numbers belong to other open branches.

## 1. Context & Objectives

The scheduled CI run 38716015544 (2026-10-15) failed `cargo deny --locked check` on `main`. `Cargo.lock` pinned `rustls` 0.23.45, which is affected by two advisories:

- **RUSTSEC-2026-0346**: the `Debug` implementations of some rustls types expose TLS secrets.
- **RUSTSEC-2026-0347**: a data leak when an Encrypted Client Hello (ECH) offer is rejected.

Both are fixed in `rustls` 0.23.46. Until `main` is fixed, every open pull request fails the Security (cargo-deny) job. `deny.toml` keeps `ignore = []`; no advisory is waived.

## 2. Architect Design

No type, trait, constant or invariant changes. `rustls` reaches the workspace through one package, `ureq` 3.4.2 (exact pin in the workspace `Cargo.toml`, features `["rustls"]`), which has two dependents:

- `soos-push-sender` (normal dependency): the production Web Push client.
- `ort-sys` 2.0.0-rc.13 (build dependency): the build script that downloads the prebuilt ONNX Runtime. Nothing from it ships in a binary.

The fix is a lockfile-only patch bump inside the semver range of `ureq`: `rustls` 0.23.45 → 0.23.46, the latest non-yanked 0.23.x release.

## 3. Plan Evaluation

Condensed for a lockfile-only change. Alternatives rejected:

- Waiving the advisories in `deny.toml`: forbidden while a fixed release exists.
- Bumping `ureq`: unnecessary churn; `ureq` is exactly pinned because its resolver API is unversioned, and 3.4.2 already accepts rustls 0.23.46.
- A plain `cargo update -p rustls`: Cargo also re-resolved `tempfile`'s `getrandom` edge (0.4.3 → 0.3.4). That unrelated hunk was reverted; `cargo metadata --locked` confirms the lockfile stays consistent without it.

Verdict: APPROVED.

## 4. Tester Contract

No new test. The contract is the existing gates:

- `cargo deny --locked check` (advisories must pass with `ignore = []`);
- the `soos-push-sender` tests, which build `UreqDeliverer` (and so the rustls client configuration) and map every `ureq` error class;
- the rest of the workspace tests under `--locked --all-features`.

Red evidence: CI run 38716015544 fails the advisories check with 0.23.45.

## 5. Auditor Constraints

1. **No Debug formatting of rustls types (RUSTSEC-2026-0346).** `crates/push-sender/src` never formats a rustls or `ureq` value. Every `tracing` call in `main.rs` uses a static message. `UreqDeliverer` and `FilteringResolver` have hand-written `Debug` impls that print only the type name, so the `ureq::Agent` and its TLS configuration are never formatted. `install_logging` sets the `ureq`, `ureq_proto` and `rustls` targets to `OFF` and installs no `log` bridge, so rustls's own `log` records are discarded. Met: the product was not exposed through this path; the bump removes the latent risk.
2. **ECH not used (RUSTSEC-2026-0347).** No crate configures Encrypted Client Hello. `UreqDeliverer::build` uses `ureq`'s default rustls client configuration (ring provider, webpki roots), with no ECH settings. Met: not exploitable in soos; fixed regardless.
3. **Build-time path.** The `ort-sys` build script runs only at build time, and the shipped binaries do not contain its HTTP client. No constraint.
4. **No unrelated lockfile churn.** Exactly one package entry changes (version and checksum). Met.
5. **Checksum source.** The new checksum comes from the crates.io index through `cargo update`. Met.

Clearance: CLEARED.

## 6. Implementation

- `Cargo.lock`: `rustls` `0.23.45` → `0.23.46` (version and checksum lines only).
- `AI/walkthroughs/211_rustls_advisories.md`: this walkthrough.

No source, `Cargo.toml`, `deny.toml` or ADR change. `AI/DECISIONS.md` has no entry for earlier dependency bumps, and this one changes no decision.

## 7. Candid Review

See `AI/candid_review_report.md` (bound to the final diff fingerprint).

## 8. Verification Results

Run with `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=4`:

- `cargo deny --locked check`: `advisories ok, bans ok, licenses ok, sources ok`.
- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean.
- `cargo test --locked --workspace --all-targets --all-features -- --test-threads=4`: 3549 passed, 0 failed, 6 ignored across 398 test binaries.

## 9. Known Limitations / Follow-ups

- `ureq` stays exactly pinned at 3.4.2; future rustls advisories inside 0.23.x need the same lockfile-only bump.
- rustls, ring and the RustCrypto crates are still not independently audited (`Docs/REMOTE_COMPANION.md`).
