# Candid Review Report

- **Date**: 2026-10-21
- **Target Branch**: `test/remote-playwright` (GitHub #464, PR 2 of #355), single commit `bed5c47`
- **Base (merge-base)**: `5ec99fd` (equal to `origin/main` at review time)
- **Reviewed-Diff-Fingerprint**: `00171270de1b2b9f06c01cbdbf164ae2254b5b178deafe4ae71c65580a8323ca`
- **Reviewer**: independent context-free sub-agent (not the author of the branch)
- **Audited Files**: `.github/dependabot.yml`, `.github/workflows/browser.yml`, `.gitignore`, `AGENTS.md`,
  `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/248_remote_playwright.md`,
  `Docs/CI_CD_AND_SECURITY.md`, `Docs/REMOTE_COMPANION.md`, `scripts/remote_browser_tests.sh`,
  `tests/browser/.npmrc`, `tests/browser/fixtures.mjs`, `tests/browser/package-lock.json`,
  `tests/browser/package.json`, `tests/browser/playwright.config.mjs`, `tests/browser/specs/cards.spec.mjs`,
  `tests/browser/specs/degradation.spec.mjs`, `tests/browser/specs/motion.spec.mjs`,
  `tests/browser/specs/signin.spec.mjs`, `tests/browser/specs/tour.spec.mjs`, `tests/invariants/src/lib.rs`,
  `tests/invariants/src/remote_playwright_contract.rs`

## 1. Executive Summary

The diff (23 files, 1921 patch lines) adds a dev-only Playwright bench of the `soos-remote` web app on
Chromium, Firefox and WebKit, its runner script, a separate advisory workflow, a Dependabot npm entry, the
static contract `remote_playwright_contract` (RPW-S1 to RPW-S5), an additive ADR amendment, matrix rows
RPW1-RPW9, documentation and walkthrough 248. No production code, asset, route, header, CSP, crate
manifest or `Cargo.lock` changes; `crates/` is untouched. The owner decision cited by the ADR amendment
exists on GitHub #355 ("Owner decision (2026-10-21)", PR 2 approved, advisory at first, pinned toolchain,
no secrets). No CRITICAL, MAJOR or MINOR finding; five suggestions.

## 2. Test Changes (mechanical listing from the frozen patch)

- Test files touched: `tests/browser/**` (all new), `tests/invariants/src/remote_playwright_contract.rs`
  (new), `tests/invariants/src/lib.rs` (one `mod` with its doc comment added, nothing removed).
- Removed or changed assertions (`^-` with `assert`/`#[test]`/`proptest!`/`should_panic`): **none**. The
  only `-` lines of the patch are in `.github/dependabot.yml` (header comment), `AI/DECISIONS.md` (the
  2026-10-13 browser ADR line; the word diff shows a pure append of the amendment) and
  `Docs/CI_CD_AND_SECURITY.md` (one sentence extended).
- New escape hatches: none. The only hits (`test.only`, `test.skip`, `test.fixme`) are inside the new
  contract, which forbids them in the specs; the Playwright config also sets `forbidOnly: true` and
  `retries: 0`.
- Inline `mod tests` added or removed: none.
- `scripts/pr_checks_gate.sh` is not in the diff: `ADVISORY_CHECKS` keeps exactly its two entries, and
  `portable_crates_contract::test_mac_pr_loop_treats_only_the_macos_job_as_advisory` is untouched.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Tried: a new state added to the fixture crate that would break immutable RAC tests iterating
  `WebScene::ALL` → the fixture crate is unchanged; the two missing states are fulfilled with `page.route`
  in the browser. PASS.
- Tried: specs that pass against a wrong page → read every spec. Expected sentences are extracted from
  `app.js` (`appText`, which throws when the constant is missing), the EventSource test counts exactly one
  extra `/api/status` per `page.clock.runFor(15000)` and asserts no `/api/events` request, the motion test
  checks both directions, the sign-in test checks the hidden cards, and an auto fixture fails any test on
  an uncaught page error. The walkthrough red-proof table (one deliberate asset break per spec) is
  consistent with what the assertions check. PASS.
- Ran the bench once: `./scripts/remote_browser_tests.sh -- --project=chromium` → 18 passed (2.8 s), the
  working tree stayed clean (outputs are git-ignored).

### PAM Concurrency & Deadlines
- `crates/pam` and the daemon are not touched; nothing here runs at authentication time. N/A → PASS.

### Panic Safety & Fail-Closed
- No production Rust code. The runner script is fail-closed: `set -euo pipefail`; a cargo build failure,
  zero or several example executables, a missing/non-executable fixture, missing Docker or an unreachable
  daemon each stop with exit 2 and a message; the Playwright status is propagated unchanged. Shellcheck
  (koalaman/shellcheck:stable) is clean. Empty `playwright_args` under `set -u` is safe on bash >= 4.4.
  PASS.

### Test Integrity & Anti-Weakening
- See §2: purely additive. The new contract is not vacuous: it pins the exact package versions and
  integrity hashes against the image tag, the container flags, the single `continue-on-error` step, the
  two-entry `ADVISORY_CHECKS` line, the spec file list and the RXB behaviours each spec must reference,
  the dev-only scope (no packaging mention, not a workspace member) and the docs. PASS.
- **RAC15 judgement** (`readme_assets_contract::test_rac_capture_tooling_is_dev_only`): the scanner flags
  any file under `scripts/` or `.github/` other than the two README capture scripts that contains
  `readme-capture` or `readme_web_fixture_server`; its stated purpose (code comment and matrix row RAC15)
  is that nothing that builds, installs or packages soos enables the capture tooling, i.e. it stays
  dev-only. The bench script builds the fixture example through `cargo build --locked -p
  soos-test-fixtures --examples` and takes the single example executable from cargo's JSON report,
  refusing anything but exactly one. This avoids the literal name, but it is an honest design rather than
  an evasion of intent: (1) it is disclosed openly in the script header, the commit message and
  walkthrough 248; (2) it does not enable the `readme-capture` feature, touch packaging, or change the
  fixture crate; (3) CI already compiles that example today (`cargo test` builds examples), so the
  bench adds no new exposure of the fixture server to any shipped artifact; (4) the exactly-one check
  fails closed instead of guessing. The residual cost is that the textual scanner no longer sees this
  consumer (see suggestion 1). Not blocking.

### Memory, Bounds & Secrets
- One fixture server per test (60 s lifetime, 5 s start-up wait, SIGKILL at teardown), test timeout 30 s,
  job timeout 20 min. Bodies are synthetic (no identity, a dummy VAPID-shaped key, no endpoint). No trace,
  screenshot, video or artifact upload; no `console.log` of page data. No secret is read or passed. PASS.

### Supply Chain & Automation
- Image `mcr.microsoft.com/playwright:v1.64.0-noble@sha256:06a9…971f`, referenced twice through one
  constant; `@playwright/test` 1.64.0 exact; lockfile v3 with exactly three registry packages and
  `sha512` integrity, no `hasInstallScript`; `.npmrc` `ignore-scripts=true`, registry pinned;
  `npm ci --ignore-scripts` with `PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1` (the only networked step); test
  container with `--network none`, `--cap-drop ALL`, `no-new-privileges`, `--user $(id -u):$(id -g)`,
  fixture binary and assets mounted `:ro`. PASS.
- Workflow: `permissions: contents: read`, no `secrets.`, no `pull_request_target`, checkout pinned to
  the same full SHA as the 19 other uses, `persist-credentials: false`, matrix and step outcome passed
  through `env:` (no `${{ }}` expression inside a `run:` body), no upload. PASS.
- Advisory semantics: separate workflow (never a `CI Success` need), `continue-on-error` on the bench
  step only, outcome written to the job summary and a `::warning` annotation, so a failure is surfaced
  honestly while the job concludes green. Dependabot npm entry with grouping (title length) and a 7-day
  cooldown. PASS.

### English-Only Policy
- All added code, comments, docs and the commit message are English. PASS.

## 4. Detailed Findings & Action Items

1. **[SUGGESTION]** `scripts/remote_browser_tests.sh` (fixture build block) — the RAC15 workaround is
   honest but makes the textual scanner blind to this consumer. A later owner-approved, additive contract
   migration that names this script among RAC15's exceptions (instead of avoiding the name) would keep the
   invariant self-describing.
2. **[SUGGESTION]** `scripts/remote_browser_tests.sh` (second `docker run`) — the host-built fixture
   binary runs inside the Ubuntu 24.04 image; on a host with a much newer glibc it could fail to start in
   the container (it worked here on Arch; on the `ubuntu-24.04` runner the libc matches). Worth one
   sentence in the CI doc.
3. **[SUGGESTION]** `scripts/remote_browser_tests.sh` (second `docker run`) — the offline test container
   mounts `tests/browser` read-write (including `node_modules`); only `test-results/` needs to be writable.
4. **[SUGGESTION]** `AI/VERIFICATION_MATRIX.md` RPW3 — "Skip stores only the fixed flag": the tour spec
   asserts the flag's value, not that no other key is written.
5. **[SUGGESTION]** `Docs/CI_CD_AND_SECURITY.md` "Cargo updates" bullet — "Dependabot covers GitHub
   Actions and the dev-only npm bench … only" reads awkwardly under a "Cargo updates" heading; consider
   rewording.

Validation run on the reviewed tree:
- `cargo fmt --all -- --check`: clean.
- `cargo test --locked -p soos-invariants -- --test-threads=3`: 871 passed, 0 failed, 0 ignored
  (includes RPW-S1 to RPW-S5, RAC15 and the portable-crates advisory contract).
- `shellcheck scripts/remote_browser_tests.sh` (koalaman/shellcheck:stable): clean.
- `./scripts/remote_browser_tests.sh -- --project=chromium`: 18 passed.

## 5. Final Verdict

**VERDICT: APPROVED**
