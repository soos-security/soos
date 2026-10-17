# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `chore/readme-redesign`
- **Base (merge-base)**: `9e82795`
- **Reviewed-Diff-Fingerprint**: `c9ed6b0fe236f381e82bf13d4254bc512301f23280410008d3dbba46350c9d88`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_readme_redesign.md`,
  `AI/auditor_constraints_readme_redesign.md`, `AI/walkthroughs/216_readme_redesign.md`,
  `Docs/DEVELOPMENT_WORKFLOW.md`, `Docs/README.md`, `README.md`, `tests/invariants/src/lib.rs`,
  `tests/invariants/src/readme_contract.rs`

## 1. Executive Summary

Documentation-only change (GitHub #408, PR 2 of #356): `README.md` is restructured as a product landing
page, and a new std-only invariant module `readme_contract` (RDM1-RDM8) pins its links, badges, wording,
section order, security section and cited comparison. No runtime crate, workflow, script, dependency or
asset is touched. The existing Quick Start, Upgrade and License text is preserved: the only README lines
removed are the old title, the old "enterprise-grade" intro paragraph and two section headers/separators.
Scoped gates re-run by this reviewer: `cargo test -p soos-invariants -- --test-threads=3` (715 passed,
0 failed, 0 ignored), `cargo clippy -p soos-invariants --all-targets -- -D warnings` (clean),
`cargo fmt --check -p soos-invariants` (clean). No CRITICAL or MAJOR finding.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `tests/invariants/src/lib.rs` (+8 lines: doc comment and
  `#[cfg(all(test, unix))] mod readme_contract;` only) and `tests/invariants/src/readme_contract.rs` (new).
- Removed or changed assertions (`^-` lines with `assert`, `#[test]`, `proptest!`, `should_panic`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: none.
- `git diff $(git merge-base origin/main HEAD) HEAD -- '*tests*' 'tests/'` confirms only the two files above;
  no pre-existing assertion was removed or changed. No setup-only edit of an existing test.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Section order: README headings are Why soos, Features, Screenshots, Quick Start, Upgrade, How it works,
  Security model, Comparison, FAQ, Roadmap, Documentation, Contributing and security, License. The ordered
  matcher picks "Security model" as index 5 before "Contributing and security", so RDM7 inspects the right
  section. PASS.
- Facts checked against the repository: `ci.yml` and `fuzz.yml` exist; toolchain `1.98.1`;
  `PrivateNetwork=yes` in `packaging/soos-daemon.service`; AES-256-GCM in `crates/biometric-store/src/crypto.rs`;
  `Verdict::{Allow, Deny, Unavailable}` in `crates/protocol/src/types.rs`; 2.7x single-scale PAD, IR
  exposure/contrast/texture gate and stricter uncalibrated IR threshold (ARCHITECTURE PAD stage, #169, #212);
  `/etc/soos/disabled` also stops presence (`crates/daemon/src/presence/worker.rs:45`); evidence 7-day retention
  and daily cap; `soos-remote` single `0600` Unix socket; doc anchors `DAEMON.md` §6,
  `PACKAGING_AND_PROVISIONING.md` §3.0/§7/§9, `DISTRIBUTION_DEPLOYMENT.md` §5.5, `REMOTE_COMPANION.md` §8;
  screenshot commit `a3cae16` is an ancestor of HEAD; no git tag and no `CHANGELOG.md` (no release badge
  is correct); roadmap issues #352-#363 are all open with titles matching the summaries. PASS.
- Acceptance vs #408 scope: hero, at most 6 badges (5 shown), captioned synthetic hero image, Why/Features/
  Screenshots, Quick Start kept, Mermaid diagram, security model with PAM_IGNORE, kill switch, #212 caveat and
  no certification, sourced comparison without accuracy numbers, FAQ, static roadmap, existing License text.
  No SECURITY.md/CONTRIBUTING.md, no logo. PASS.

### PAM Concurrency & Deadlines
- No change in `crates/pam` or any runtime code. README statements (blocking, no Tokio, deadline from
  `timeout_ms` default 1000 ms) match AGENTS.md and the ADR. PASS (not applicable to code).

### Panic Safety & Fail-Closed
- Only test code; panics are confined to the invariant module with the usual `#![allow]` reason.
  `has_release_evidence` treats a missing or failing `git` as "no tag" (stricter), and strips `GIT_*`
  variables so it behaves inside hooks. The README never claims an error path leads to `PAM_SUCCESS`;
  it states the opposite. PASS.

### Test Integrity & Anti-Weakening
- Tried to break the parser: nested badges `[![..](a)](b)`, reference definitions, `href`/`src`/`srcset`,
  `srcset` not mistaken for `src` (the `=` check), inline and fenced code ignored, `../` climbing, absolute
  paths and empty paths refused (RDM2 asserts each). Whole-word matcher rejects `uncertified` and accepts
  `self-certified` (RDM4). A plausible wrong README (missing section, extra badge, banned word, missing
  #212) fails the contract; the walkthrough's red run is consistent with this. PASS.

### Memory, Bounds & Secrets
- Test reads two repository files, runs one `git tag --list`, writes a scratch tree under
  `target/readme_contract/` and removes it. No image added; the README only references the three synthetic
  PNGs under the existing RAC contract. No real host name, account or face. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change; std only. PASS.

### English-Only Policy
- All added text is English; no accented non-English characters found in the added files. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `README.md` Features row "Native Android app" — cites `Docs/REMOTE_COMPANION.md` §2j, but that
  document has two `## 2j.` headings (offline history at line 985, Android app at line 1063; pre-existing
  duplication on `main`). The reference is ambiguous; renumbering the second heading in a follow-up would
  remove the ambiguity. Not introduced by this diff.
- **[MINOR]** `tests/invariants/src/readme_contract.rs` `link_targets` / `image_urls` — parsing is line-based,
  so an HTML tag whose `src`/`href` attribute sits on a different line than the tag start, or a Markdown link
  split across lines, escapes RDM1/RDM3. The current README keeps every tag on one line, so the contract is
  correct today; a future multi-line `<img>` would only reduce coverage.
- **[SUGGESTION]** `is_release_badge` matches the substring `release` anywhere in the URL, so a future
  `release.yml` workflow badge would be refused while no tag exists. This is strict in the safe direction.

## 5. Final Verdict

No CRITICAL or MAJOR finding; no pre-existing test assertion removed or changed; scoped tests, clippy and
fmt are green.

**VERDICT: APPROVED**
