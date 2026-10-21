# Candid Review Report

- **Date**: 2026-10-21
- **Target Branch**: `chore/playwright-bench-followups`
- **Base (merge-base)**: `fdf6039`
- **Reviewed-Diff-Fingerprint**: `4aa5d83e2401c835b5179754dd13f24ce855eee73ad482d4b0a1526e35b734d9`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/249_playwright_bench_followups.md`, `Docs/CI_CD_AND_SECURITY.md`, `scripts/remote_browser_tests.sh`, `tests/browser/specs/tour.spec.mjs`

## 1. Executive Summary

Dev-only follow-up to PR #465. The offline Playwright test container now mounts `tests/browser`
read-only and exposes a single writable host directory (`tests/browser/test-results`, already
git-ignored) at `/work/output`, outside the bench tree, with `--output /work/output/run` so
Playwright's delete-and-recreate of its output directory never targets a mount point. Two doc
clarifications, RPW3 reworded to name the exact key/value, and one additive assertion in
`tour.spec.mjs`. No production Rust, PAM, IPC, daemon or shipped asset is touched. No CRITICAL or
MAJOR finding.

## 2. Test Changes

Mechanical listing from the frozen patch:

- Test files touched: `tests/browser/specs/tour.spec.mjs` (and `scripts/remote_browser_tests.sh`, matched by the `tests` substring; it is the bench runner, not a test).
- Removed/changed assertions (`^-` lines with `assert`/`expect(`/`test(`): **none**.
- New escape hatches (`.skip`, `.fixme`, `test.only`, `#[ignore]`, tolerance): **none**.
- Inline Rust test modules: none touched.

The single change is one **added** assertion after the existing flag check:
`expect(Object.keys(window.localStorage)).toEqual([TOUR_STORAGE_KEY])`. It strengthens the
contract (RPW3 "no other key"); it fails against a plausible wrong implementation that writes an
extra key (the walkthrough records a red proof on all three engines). Test names are unchanged, so
`test_rpw_s3_specs_cover_the_rxb_behaviours` still matches. `Object.keys(localStorage)` returns
stored keys only (not methods), and each Playwright test gets a fresh context, so the assertion is
deterministic. `TOUR_STORAGE_KEY`/`TOUR_STORAGE_SEEN` in `tests/browser/fixtures.mjs` match
`crates/remote/assets/app.js` (`soos-remote-tour-v1` / `seen`), and that is the only
`localStorage.setItem` in `app.js`.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Scenario: Playwright `rmdir`s its output directory → a writable mount at the output path inside an
  `:ro` tree would fail with EROFS. The script uses a subdirectory `run` of the writable mount, so the
  delete/recreate happens inside a writable bind. PASS.
- Scenario: user passes their own `--output` after `--` → it appears later on the command line and
  wins; harmless. PASS.
- Scenario: `test-results` missing on a fresh clone → `mkdir -p "$OUTPUT"` on the host as the calling
  user before `docker run`, so Docker does not create it root-owned. PASS.
- Scenario: the first (`npm ci`) container still needs a writable `tests/browser` for
  `node_modules` → unchanged (only the second run is `:ro`). PASS.
- `readonly OUTPUT`/`CONTAINER_OUTPUT` do not collide with existing names. Config still has
  `outputDir: "test-results"` for direct runs; CLI overrides it. Trace/screenshot/video stay off. PASS.
- Docs: the glibc note is accurate (host-built binary run in Ubuntu 24.04 image) and offers a concrete
  workaround; the "Cargo updates" sentence now reads correctly. PASS.

### PAM Concurrency & Deadlines
- Not applicable: no `crates/pam` change. PASS.

### Panic Safety & Fail-Closed
- No Rust production code changed. Script keeps `set -euo pipefail` and propagates the
  `playwright test` exit status. PASS.

### Test Integrity & Anti-Weakening
- See §2: zero removed or modified assertions; one additive assertion; RPW3 reworded to describe the
  stronger check precisely (no criterion dropped). PASS.

### Memory, Bounds & Secrets
- Writable surface shrinks from the whole bench (specs, config, lockfile, `node_modules`) to one
  ignored output directory; `--network none`, `--cap-drop ALL`, `no-new-privileges`, non-root user
  retained; no secret, credential or owner data introduced. PASS.

### Supply Chain & Automation
- No change to image digest, lockfile, `package.json`, `.npmrc`, workflows or `deny.toml`. The
  `remote_playwright_contract` invariants (`:ro`, `--network none`, `npx --no-install`) still hold
  (reported 871 passed). PASS.

### English-Only Policy
- All added text (script comments, docs, matrix row, walkthrough, test comment) is English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `Docs/CI_CD_AND_SECURITY.md:115` — the rewrapped isolation bullet leaves one
  line noticeably longer than its neighbours ("… passed to Playwright as `--output`. No trace,
  screenshot or video is recorded."); rewrap for consistency at the next doc touch. Cosmetic only.

## 5. Final Verdict

**VERDICT: APPROVED**
