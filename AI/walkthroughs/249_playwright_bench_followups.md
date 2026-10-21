# Walkthrough 249 — Playwright Bench Follow-ups (Read-Only Mount, Docs, RPW3)

- **Date**: 2026-10-21
- **Issue**: GitHub #464 follow-up (non-blocking suggestions of the candid review of PR #465) —
  **Branch**: `chore/playwright-bench-followups`
- **Matrix criteria**: RPW3 reworded to match its spec (still verified). RPW1–RPW2, RPW4–RPW9 unchanged.

## 1. Context & Objectives

The candid review of PR #465 (walkthrough 248) approved the Playwright bench of the `soos-remote` web
app with five non-blocking suggestions. This change applies the three that need no owner decision:

1. the offline test container mounted the whole `tests/browser` directory read-write;
2. `Docs/CI_CD_AND_SECURITY.md` did not say the fixture server is a host binary run inside the
   Ubuntu 24.04 image, and its "Cargo updates" bullet ended in an awkward "… only;";
3. matrix row RPW3 claimed "Skip stores only the fixed flag" while `tour.spec.mjs` checked the flag
   value only.

Out of scope (owner decision): adding `scripts/remote_browser_tests.sh` to the named exceptions of
RAC15 (`readme_assets_contract`). RAC15 and every existing assertion are unchanged.

## 2. Architect (Phase 1)

- **Mounts**: the second container keeps `--network none`, `--cap-drop ALL`, `no-new-privileges` and
  the calling user. The bench (`specs`, config, `node_modules` from the first container) is mounted
  `:ro`. The only writable mount is the host directory `tests/browser/test-results` (already in
  `.gitignore`, RPW-S4), at `/work/output`, outside the bench tree.
- **Output directory**: Playwright deletes and recreates its output directory before a run. A first
  attempt mounted the writable directory over `/work/tests/browser/test-results` inside the read-only
  tree and failed with `EROFS: read-only file system, rmdir`; a mount point can never be removed. The
  script therefore passes `--output /work/output/run`, a subdirectory of the writable mount. The
  config keeps `outputDir: "test-results"` for direct runs; the CLI flag overrides it.
- **RPW3**: extend the spec additively (preferred over weakening the row): after Skip, the list of
  `localStorage` keys equals exactly `[TOUR_STORAGE_KEY]`. The row names the key, the value and "no
  other `localStorage` key".

## 3. Tester (Phase 2)

One new assertion in `the tour shows once, then is remembered` (`tests/browser/specs/tour.spec.mjs`),
after the existing flag check; no existing assertion moved or changed. Test names are unchanged, so
`test_rpw_s3_specs_cover_the_rxb_behaviours` still finds both tour tests.

Red proof: a temporary break of `crates/remote/assets/app.js` (an extra
`localStorage.setItem("stray", "1")` next to the flag write, reverted afterwards) made the test fail
on Chromium, Firefox and WebKit (`Expected - 0 / Received + 1`), while the blocked-storage test
stayed green.

## 4. Auditor (Phase 3)

- No new network, capability, secret or privileged flag; `remote_playwright_contract` still finds
  `:ro`, `--network none`, `--cap-drop ALL`, the user flag and `npx --no-install playwright test`.
- The writable surface shrinks from the whole bench (specs, config, lockfile, `node_modules`) to one
  ignored output directory; a compromised test run can no longer rewrite the specs or the
  dependencies used by a later run.
- No trace, screenshot or video is recorded; the output directory only receives `.last-run.json`.

## 5. Developer (Phase 4)

- `scripts/remote_browser_tests.sh`: `OUTPUT` / `CONTAINER_OUTPUT`, `mkdir -p` on the host, the bench
  mount `:ro`, the output mount, `--output "$CONTAINER_OUTPUT/run"` before the user arguments.
- `Docs/CI_CD_AND_SECURITY.md`: the isolation bullet names the read-only bench and the single writable
  output directory; the local-run bullet notes that the fixture server is built on the host and run
  in the Ubuntu 24.04 image, so a host with a much newer glibc may produce a binary that does not
  start there (workaround: `--fixture-server` with a binary built on an older base); the "Cargo
  updates" bullet now reads "Dependabot does not update crates; it covers only …".
- `AI/VERIFICATION_MATRIX.md` RPW3 reworded as in §2.

## 6. Verification

- `CARGO_TARGET_DIR=~/.cache/soos-targets/<worktree> CARGO_BUILD_JOBS=3 ./scripts/remote_browser_tests.sh`:
  54 passed (Chromium, Firefox, WebKit).
- `cargo test --locked -p soos-invariants -- --test-threads=3`: 871 passed, 0 failed.

## 7. Open question for the owner

Should `scripts/remote_browser_tests.sh` join the named exceptions of RAC15
(`readme_assets_contract`), so it can name the fixture server example directly instead of reading it
from cargo's build report? Left unchanged here.
