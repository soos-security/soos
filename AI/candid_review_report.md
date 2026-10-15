# Candid Review Report

- **Date**: 2026-10-15
- **Target Branch**: `fix/rustls-advisories`
- **Base (merge-base)**: `6f32ef8`
- **Reviewed-Diff-Fingerprint**: `c4e1ae444cf5861dd56d95d9df1f331ece31dde73ef774b4469fa79e12f3a2d5`
- **Audited Files**: `Cargo.lock`, `AI/walkthroughs/211_rustls_advisories.md` (new)

## 1. Executive Summary

The patch is a lockfile-only security bump: `rustls` 0.23.45 → 0.23.46 to clear RUSTSEC-2026-0346
(secrets exposed through `Debug`) and RUSTSEC-2026-0347 (data leak on a rejected ECH offer), plus
walkthrough 211. Exactly four `Cargo.lock` lines change (the version and checksum of one package
entry). The dependency list of the `rustls` entry is unchanged. No source, `Cargo.toml`, `deny.toml`,
workflow, script or test file is touched. I checked each claim independently and found no defect.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

All four mechanical greps over `target/candid_diff.patch` returned no matches:

- test/fixture files touched: none;
- removed or changed assertions / `#[test]` / `proptest!` / `#[should_panic]`: none;
- new escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): none;
- inline `mod tests` changes: none.

No test was added, modified or removed. No justification is needed.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Scenario: the bump pulls in a new transitive dependency or changes another package. The diff
  hunk touches only the `version`/`checksum` lines. The `dependencies` array of the `rustls` entry
  (`log`, `once_cell`, `ring`, `rustls-pki-types`, `rustls-webpki`, `subtle`, `zeroize`) is
  identical. `git diff origin/main -- Cargo.lock` shows four changed lines. → PASS.
- Scenario: the lockfile is inconsistent with the manifests (for example, the reverted
  `getrandom` hunk the walkthrough mentions leaves a dangling edge). `cargo metadata --locked`
  succeeds. → PASS.
- Scenario: the version falls outside the `ureq` 3.4.2 requirement. Resolution under `--locked`
  succeeds, and `cargo tree --locked -i rustls` resolves `rustls v0.23.46 ← ureq v3.4.2 ←
  soos-push-sender`. → PASS.
- Scenario: the walkthrough's claim about the `ort-sys` build-dependency edge is wrong. The
  `ort-sys` 2.0.0-rc.13 lock entry does list `ureq`. `cargo tree -i ureq -e all --target all`
  shows the build-dependency path through `soos-inference-ort`. The claim is accurate. → PASS.

### PAM Concurrency & Deadlines
- `crates/pam` is not touched, and `rustls` is not in the PAM dependency graph (the only
  dependent chain is `ureq` → `soos-push-sender` and the `ort-sys` build script). → PASS (N/A).

### Panic Safety & Fail-Closed
- No production code changes. A patch release of `rustls` changes no API the workspace uses
  (`ureq` is exactly pinned and unchanged). → PASS (N/A).

### Test Integrity & Anti-Weakening
- See §2. No test is modified. The existing `soos-push-sender` tests and `cargo deny` are a
  sufficient contract for a lockfile patch bump. → PASS.

### Memory, Bounds & Secrets
- Scenario: RUSTSEC-2026-0346 was exploitable through soos logging before the bump. I checked
  `crates/push-sender/src/lib.rs`: `UreqDeliverer` (line 257) and `FilteringResolver` (line 180)
  have hand-written `Debug` impls that print only the type name. `install_logging` turns off the
  `ureq`, `ureq_proto` and `rustls` targets (lines 359–361). No `log` bridge (`LogTracer` /
  `tracing-log`) is installed. The walkthrough's auditor constraints match the code. → PASS.
- Scenario: ECH is configured anywhere (RUSTSEC-2026-0347). No ECH configuration exists in
  `crates/push-sender/src`. → PASS.

### Supply Chain & Automation
- Scenario: the checksum is forged or wrong. I computed the SHA-256 of the cached
  `rustls-0.23.46.crate` from the crates.io registry: `48e13bd8…8fdf`, which matches the lockfile. → PASS.
- Scenario: the advisories are waived instead of fixed. `deny.toml` is unchanged and keeps
  `ignore = []`. `cargo deny --locked check` → `advisories ok, bans ok, licenses ok, sources ok`. → PASS.
- Source stays `registry+https://github.com/rust-lang/crates.io-index`. No git or path source is
  introduced. → PASS.

### English-Only Policy
- The walkthrough is in English. `./scripts/candid_review.sh` Audit 7 reports OK. → PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `AI/walkthroughs/211_rustls_advisories.md:5` — numbering skips from 199 to 211.
  The walkthrough explains this (lower numbers are reserved for other open branches), and no
  repository check enforces contiguous numbering. No action is required.

No CRITICAL, MAJOR or MINOR finding.

## 5. Final Verdict

**VERDICT: APPROVED**
