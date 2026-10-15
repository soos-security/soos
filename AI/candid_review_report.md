# Candid Review Report

- **Date**: 2026-10-15
- **Target Branch**: `test/readme-capture`
- **Base (merge-base)**: `bcfef93`
- **Reviewed-Diff-Fingerprint**: `cfbe3f91c301ed7ecbc6ed93d8786cf1ad143518efadd77bdb1d5806012032da`
- **Audited Files**: AI/DECISIONS.md, AI/VERIFICATION_MATRIX.md, AI/architect_spec_readme_capture.md,
  AI/auditor_constraints_readme_capture.md, AI/walkthroughs/207_readme_capture.md, Cargo.lock,
  Docs/DEVELOPMENT_WORKFLOW.md, Docs/GUI_APPLICATION.md, Docs/README.md, Docs/README_ASSETS.md,
  Docs/assets/MANIFEST.toml, Docs/assets/screenshots/gui-guided-enrollment.png,
  Docs/assets/screenshots/gui-live-inspection.png, Docs/assets/screenshots/gui-profiles.png,
  crates/gui/Cargo.toml, crates/gui/src/app.rs, crates/gui/src/app/capture.rs,
  crates/gui/tests/readme_capture/mod.rs, crates/gui/tests/readme_capture/png_io.rs,
  crates/gui/tests/readme_capture/raster.rs, crates/gui/tests/readme_snapshots.rs,
  scripts/capture_readme_assets.sh, scripts/capture_readme_web.sh,
  tests/fixtures/examples/readme_web_fixture_server.rs, tests/fixtures/mod.rs,
  tests/fixtures/tests/readme_fixture_tests.rs, tests/invariants/src/lib.rs,
  tests/invariants/src/readme_assets_contract.rs

## 0. Re-review After the Rebase Onto `main` `bcfef93` (2026-10-15)

The branch was rebased from merge-base `94a7cb3` onto `origin/main` `bcfef93`. Main gained #383
(push-test deflake), #394 (actions bump), #384 (motion spec), #388 (lock-screen state request),
#386 (remote offline history, including `crates/remote/assets/app.js`/`style.css`/`sw.js`), #385
(`scripts/setup.sh`) and #390 (system ONNX Runtime, MSRV 1.92, a new `[features]` table in
`crates/gui/Cargo.toml`). The previous APPROVED report was bound to fingerprint `cb76b5a5...`;
this review re-freezes the diff (`--prepare`) and re-checks it, including the follow-up commit
`f6c7511`.

- `git range-diff origin/test/readme-capture...HEAD`: the seven `main` commits appear as new; the
  branch commit `e88843b` maps to `b8d7ddb` with interdiff hunks only in the three "both sides
  appended" files and in the per-branch report files.
- Scripted per-file comparison of the sorted added/removed lines of `git diff 94a7cb3 1b276b8`
  (old branch) versus `git diff bcfef93 b8d7ddb` (rebased): same 30 files; 25 identical,
  including `AI/DECISIONS.md` (one appended ADR line) and `tests/invariants/src/lib.rs` (the
  8-line `readme_assets_contract` registration, appended after main's modules).
  `AI/VERIFICATION_MATRIX.md` differs only by an added blank line and `---` separator before the
  `readme-capture` section (placement only). `AI/candid_review_report.md` and
  `AI/plan_evaluator_report.md` differ only because main's versions changed; the branch blobs are
  byte-identical. The three PNG blobs are identical.
- **Semantic conflict found and fixed**: after the rebase, `crates/gui/Cargo.toml` held two
  `[features]` tables (main's #390 table and this branch's), so `cargo metadata` failed with
  "duplicate key". Commit `f6c7511` moves the `readme-capture = []` line and its comment into
  main's single table, `default = ["download-binaries"]` unchanged. The frozen patch now shows
  the gui manifest as additions only (the feature line plus the three dev-dependencies); the
  `system_onnxruntime_contract` feature lines and RAC15 (`readme-capture=[]`, not in `default`,
  `png`/`soos-test-fixtures` only in `[dev-dependencies]`) both hold.
- Rendering interaction: main changed only `crates/gui/src/main.rs` (an ONNX Runtime provider
  check before the model registry, not reachable from the capture scenes) and the gui manifest;
  no `Cargo.lock` package version changed (egui/eframe/epaint identical). RAC1
  `test_rac_capture_scenes_match_committed_assets` passes, so the committed PNGs are not stale.
- Web app interaction (#386): `crates/remote/assets` still holds exactly 11 files (RAC8 holds);
  the view structs bound by RAC9 are unchanged and the RAC9 test passes. The new
  `GET /api/offline` endpoint is not served by the README fixture (see the new MINOR in section 4).
- `tests/docker/system_onnxruntime_test.sh` (from #390) runs `cargo check --workspace
  --all-features`, which compiles the gated module in a check build only; nothing is packaged
  from it and RAC15 still finds no packaging/install file naming the feature.
- Mechanical test listing on the new patch: no removed line in the whole patch (append-only), no
  escape hatch beyond the RAC1 drift tolerance, no inline `mod tests` change. The previous MINOR
  line references (`tests/fixtures/mod.rs:199/285/495/587`, `readme_snapshots.rs:43-46`) still
  point at the same code.
- Re-run with `LIBONNXRUNTIME_NO_PKG_CONFIG=1 CARGO_INCREMENTAL=0 CARGO_PROFILE_*_DEBUG=0
  CARGO_BUILD_JOBS=4`: `cargo clippy --locked -p soos-gui --all-targets --all-features -- -D
  warnings` clean; `cargo test --locked -p soos-gui -p soos-invariants --all-features --
  --test-threads=4` 843 passed, 0 failed (`readme_snapshots` 9/9 including RAC1, all
  `readme_assets_contract` RAC10-RAC16 tests).

All previous findings are carried forward unchanged; one MINOR is added (section 4). Sections 1-3
below are the original 2026-10-13 audit, which still applies to this diff.

## 1. Executive Summary

GitHub #381 (PR 1 of #356) adds reproducible README screenshot tooling: a feature-gated
(`readme-capture`, empty, non-default) `soos_gui::app::capture` module that builds a real `SoosApp`
with every side effect replaced, a headless CPU rasterizer and PNG writer in the gui test tree, a
placeholder-face illustration and a loopback-only web fixture server in the dev-only
`soos-test-fixtures` crate, three committed 1120x780 PNGs with a SHA-256 manifest, two capture
scripts, and a `soos-invariants` contract enforcing manifest, hash, budget, metadata and dev-only
rules. No production behaviour changes (`app.rs` only gains the gated `mod capture;`). No existing
test is modified. Locally verified: `readme_snapshots` 9/9, `soos-test-fixtures` 9/9 (3 + 6),
`soos-invariants` 614/614, clippy `-D warnings` on the three crates with `--all-targets
--all-features`, `cargo fmt --check`. No CRITICAL or MAJOR finding; three MINOR findings and one
cosmetic observation.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched (all new, or append-only): `crates/gui/tests/readme_capture/{mod,png_io,raster}.rs`,
  `crates/gui/tests/readme_snapshots.rs`, `tests/fixtures/examples/readme_web_fixture_server.rs`,
  `tests/fixtures/mod.rs` (append-only: two new modules after the existing `onnx` module),
  `tests/fixtures/tests/readme_fixture_tests.rs`, `tests/invariants/src/lib.rs` (one new
  `mod readme_assets_contract;` declaration), `tests/invariants/src/readme_assets_contract.rs`.
- Removed/changed assertions (`^-` with assert/#[test]/proptest/should_panic): **none**.
- New escape hatches: no `#[ignore]`, no `#[cfg(any())]`, no `should_panic`. The only
  `tolerance` hits are the RAC1 drift tolerance (`CAPTURE_DIFF_CHANNEL_TOLERANCE = 16`,
  `CAPTURE_MAX_DIFF_FRACTION = 0.00002`, i.e. at most 17 of 873 600 pixels) and its documentation;
  this is a new contract, not a relaxation of an existing one, and RAC2 still requires byte
  identity within one process.
- Inline `mod tests` changes: none.

`git diff --cached --diff-filter=M` confirms no pre-existing test file other than the two
append-only module registrations above is modified.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Tried: can the capture module exist in a shipped binary? `capture` is behind
  `#[cfg(feature = "readme-capture")]`; the feature is `[]`, not in `default`; no packaging file
  uses `--all-features` (only `save.sh`/CI test commands do) and RAC15 scans `packaging`,
  `scripts`, `.github`, `tests/docker`, `tests/distro`, `Dockerfile`, `save.sh`, `run_tests.sh`
  for the feature name. PASS.
- Tried: does a capture scene reach the camera, systemctl, pkexec, the store or the network?
  `StaticCaptureCamera` is thread-free and never yields a frame; `FixedDaemonProbe` answers a
  constant (the only spawned thread is `DaemonMonitor` polling that constant); `RefusingExecutor`
  refuses every `PrivilegedAction`; `store_tasks: None`, `GuiStore::Polkit` keeps no local store,
  `switchable_camera`/`camera_source` are `None`, `_worker_thread: None`. The only other
  process-level reads are `EnrollmentGuiState::default()` reading the host uid/login, which are
  immediately overwritten by `alice`/1000 (RAC3 asserts the state exactly). PASS.
- Tried: `capture_app` with YUYV, short buffer and 0x0 frames: `frame_is_supported` uses checked
  multiplication and is tested (RAC4). PASS.
- Tried: rasterizer correctness on shared edges, clipping, premultiplied blending, ppp scaling,
  NEAREST/LINEAR fetch, partial deltas, freed textures: all covered by RAC5 with exact pixel
  values that a plausible wrong implementation (no top-left rule, straight-alpha blending,
  no clamp) would fail. PASS.
- Tried: drift test bypass in CI via update mode: `SOOS_UPDATE_README_ASSETS=1` panics when `CI`
  is set, and the script also refuses under `CI`. PASS.

### PAM Concurrency & Deadlines
- No file under `crates/pam` is touched; no Tokio added anywhere. N/A, PASS.

### Panic Safety & Fail-Closed
- `capture.rs`: no `unwrap`/`expect`/indexing; errors are a `thiserror` enum; the daemon-state wait
  is bounded by `CAPTURE_DAEMON_STATE_TIMEOUT` (2 s). PASS.
- `tests/fixtures/mod.rs`: `route` is total (non-`GET`, query strings, traversal, unknown paths
  give `NotFound`; asset names come from a fixed allow-list, never from the request path).
  `strip_png_metadata` uses `get(..)` and `checked_add`; the `header[i]` indexing is on an 8-byte
  slice obtained by `get`. No path to `PAM_SUCCESS`/`Allow` exists in this diff. PASS (see MINOR 2
  on the lint allowances).

### Test Integrity & Anti-Weakening
- No removed or weakened assertion (section 2). New tests can fail: RAC1 fails on any glyph or
  layout change beyond 17 pixels; RAC3 fails if the host login, `/home/`, `/tmp`, `@` or a measured
  latency is rendered; RAC11 fails on a hash mismatch, unlisted file, absent file, unknown
  generator, wrong `gui-`/`web-` prefix or non-`synthetic` source; RAC13 fails on any ancillary PNG
  chunk outside `gAMA/cHRM/sRGB/pHYs/PLTE/tRNS` (iCCP, tEXt, zTXt, iTXt, eXIf, tIME, acTL refused)
  and on trailing data after `IEND`; RAC12 enforces 1 MiB/file, 8 MiB total, 32 files. The
  synthetic detector cases exercise each rule. PASS.
- RAC9 binds the canned web JSON keys to the real `soos-remote` view structs, so the fixture
  cannot silently drift from the served contract. PASS.

### Memory, Bounds & Secrets
- PNG inspection (independent Python chunk walk): all three PNGs are RGB8 1120x780 with exactly
  `IHDR`, `IDAT`, `IEND` and no trailing bytes; `strings` finds no host/user/path text; SHA-256
  values match `MANIFEST.toml`. Sizes 56-78 KB, far under budget. PASS.
- Visual inspection of the three PNGs: the face is a flat analytic illustration (ellipse head,
  hair cap, dot eyes, rectangle nose, pink mouth), not a photo; the only identity shown is `alice`
  / uid 1000 / timestamp 1790812800; telemetry is the fixed 12.5 ms. Screens look correct
  (live diagnostic with overlays and SFace crop, guided enrollment form, profiles table). PASS.
- Fixture server: binds `127.0.0.1:0` only (asserted by RAC15 together with the absence of
  `0.0.0.0`/`[::]`), 8 KiB request-head cap, 16 live connections, 2 s per-I/O timeouts, SSE hold
  capped by the serve deadline, `--serve-ms` clamped to 1-300 s, `Cache-Control: no-store`.
  Lives in the dev-only `soos-test-fixtures` crate (only `[dev-dependencies]` users), keeps its
  single normal dependency (`soos-frame`), and is never referenced by `crates/remote`. PASS with
  MINOR 1.
- No logging of frames, embeddings or credentials in new code. PASS.

### Supply Chain & Automation
- `Cargo.lock`: only three dependency-list lines added to `soos-gui` (`png`, `soos-frame`,
  `soos-test-fixtures`); `png 0.18.1` was already locked on `origin/main`; no new package entry.
  PASS.
- No `.github/` change. Scripts: `set -euo pipefail`, no download/install/sudo/pkexec/systemctl,
  manifest rewrite through `mktemp` + `mv` with an `EXIT` trap, refuses to run under `CI`; the
  browser script is optional, uses a throw-away profile and only `http://127.0.0.1:<port>/`.
  (`shellcheck` is not installed on the review host; not run.) PASS.
- Cross-host drift (Arch host vs Ubuntu CI runner, both x86_64): the rasterizer itself is pure
  `f32` without transcendental calls; epaint uses precomputed circle vertices; the remaining
  platform-dependent pieces are glibc `powf` (text alpha gamma) and `vello_cpu`'s runtime SIMD
  dispatch for glyph rasterization. The host is AVX2+FMA (glibc 2.44); GitHub `ubuntu-latest`
  runners are AVX2+FMA (glibc 2.39, same `powf` implementation family), so the same SIMD level is
  selected. A one-ulp `powf` difference moves glyph alpha by at most a level or two, well inside
  16. Residual risk judged LOW, tracked by the pending RAC17 row. See MINOR 3.

### English-Only Policy
- Code, comments, docs, scripts and the ADR/matrix rows are English. Non-ASCII characters are
  typographic only. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `tests/fixtures/mod.rs:495` — `read_request_line` bounds each `read` by the 2 s
  socket timeout and the head size by 8 KiB, but has no absolute deadline: a local client trickling
  one byte per <2 s keeps a handler alive for hours, and `serve` (`tests/fixtures/mod.rs:587`)
  joins every worker, so the server can outlive `serve_for`. Loopback-only dev tool, so not a
  security issue; check `deadline` (or a per-connection head deadline) inside the read loop.
- **[MINOR]** `tests/fixtures/mod.rs:199` and `:285` — module-wide
  `#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing, ...)]` contradicts the
  literal wording of auditor constraint AC-5 ("no indexing/unchecked arithmetic in
  `tests/fixtures/mod.rs`"). Every actual use is bounded (sides clamped to 16..=4096, `header[i]`
  on an 8-byte `get` slice), so no panic path exists; narrow the allowances to the specific items
  or record the deviation in the auditor constraints.
- **[MINOR]** `crates/gui/tests/readme_snapshots.rs:43-46` — the RAC1 cross-host tolerance is
  unconfirmed until the first Ubuntu CI run (RAC17 pending). Risk is low (same SIMD level and
  glibc `powf` family on both machines), but if CI fails, the fix is to regenerate or tune the
  tolerance in a new reviewed diff, never to skip the test.
- **[MINOR]** `tests/fixtures/mod.rs:442-447` — the README web fixture serves no
  `GET /api/offline` (added to `soos-remote` by #386). The web app swallows the 404 and keeps the
  new "Offline history" card hidden, so `scripts/capture_readme_web.sh` captures would omit that
  feature rather than look broken. No web capture is committed yet (RAC18 pending), so nothing in
  the repository is stale. Open follow-up for the RAC18 web captures: serve a synthetic
  `/api/offline` body and extend RAC8/RAC9 to bind it to the real view type.
- **[SUGGESTION]** `Docs/assets/screenshots/gui-guided-enrollment.png` — a 2-pixel pale band
  crosses the chin at y = 535-536 (pixels `(235,226,227)` over skin `(232,196,170)`), at the
  bottom edge of the mock detection box; it is drawn by the real GUI, not by the fixture or the
  rasterizer, and is cosmetic only.

## 5. Final Verdict

No CRITICAL or MAJOR finding (the post-rebase duplicate `[features]` table is fixed by
`f6c7511`). Capture code is compile-gated and side-effect-free, the committed
PNGs are synthetic, identity-free and metadata-free, the fixture server is loopback-only, bounded
and dev-only, the invariant enforces the manifest/hash/budget/metadata rules, no test is weakened,
and all targeted suites, clippy and fmt pass locally.

**VERDICT: APPROVED**
