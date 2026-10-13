# Walkthrough 207 — Reproducible README Screenshot Capture

- **Date**: 2026-10-13
- **Issue**: GitHub #381 (PR 1 of epic #356; no backlog id, branch not registered in `scripts/sync_issue.py`) — **Branch**: `test/readme-capture`
- **Matrix criteria**: RAC1–RAC18 (component `readme-capture`)

## 1. Context & Objectives

Epic #356 asks for an illustrated, product-grade README whose visuals are regenerated from mock or synthetic data
instead of going stale. Its planned split puts the capture tooling first: this PR adds a hardware-free capture of the
`soos-gui` screens, a capture path for the `soos-remote` phone web app from a fixture server, the `Docs/assets/` layout,
an asset budget and a README asset invariant. The README text itself is PR 2 (`chore/readme-redesign`).

The epic's owner questions have no answer yet. The recommended options are assumed and marked "assumed — owner to
confirm" everywhere (spec §0, ADR, `Docs/README_ASSETS.md` §5):
- OD-1: ship with the current UI and refresh later;
- OD-2: static PNG/WebP only, no animated media yet;
- OD-3: no Node toolchain or Playwright container; the browser step is an optional script;
- OD-4: 1 MiB per file, 8 MiB and 32 files in total.

## 2. Architect Design

Spec `AI/architect_spec_readme_capture.md` (round 2; §10 binding):
- **Renderer (D-1)**: `egui_kittest` is not adopted. Its pixel snapshots need `wgpu` and a GPU or lavapipe. The capture
  drives the real `eframe::App::ui` through the egui 0.35 harness (`eframe::CreationContext::_new_kittest`,
  `eframe::Frame::_new_kittest`, fixed `RawInput`, 3 passes, 1120×780 points at 1 pixel per point). It tessellates the
  output and rasterizes it with a small CPU rasterizer in test code.
- **Scene builder**: `soos_gui::app::capture` (child module of `app`, so it can fill private `SoosApp` fields), behind
  the non-default, empty `readme-capture` feature. `capture_app` replaces every side effect:
  - thread-free `StaticCaptureCamera`;
  - `DaemonMonitor` with a fixed probe (no `systemctl`);
  - `RefusingExecutor` (no `pkexec`);
  - `GuiStore::Polkit` without `refresh_profiles`;
  - no vision worker: the mock pipeline runs once on the caller's frame, and telemetry is fixed (30 FPS, 12.5 ms).

  Identity is `alice` / 1000, and `shown_identities` exposes it for the tests.
- **Synthetic image**: `soos_test_fixtures::readme::create_placeholder_face_frame`, a flat brand-coloured drawing
  whose eyes, nose tip and mouth sit on `MockFaceDetector::canonical_landmarks_for_box` for the centred box.
- **Web path**: `soos_test_fixtures::readme_web` contains:
  - `route`: pure and total, serving the 11 assets of `crates/remote/assets` plus canned, identity-free JSON in the
    exact key sets of `StatusView`, `AlertsView`, `AlertRecord`, `BatteryView` and `PushView::disabled()`;
  - `serve`: loopback, bounded;
  - `strip_png_metadata`.

  The example `readme_web_fixture_server` wraps them. `soos-remote` is untouched.
- **Assets**: `Docs/assets/screenshots/gui-*.png` plus `Docs/assets/MANIFEST.toml` (path, generator, SHA-256,
  `source = "synthetic"`), written by `scripts/capture_readme_assets.sh`.
- **ADR** "[2026-10-13] README Assets Are Generated From Synthetic Data, Never Hand-Edited", appended to
  `AI/DECISIONS.md`. It records the deviations from the epic: no egui_kittest/wgpu, no Docker, no Playwright, and the
  test branch name.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`, run by an independent sub-agent. A first launch died on an API limit and was
relaunched.

**Round 1: REVISION_REQUIRED** (1 MAJOR, 11 MINOR).
- **F-1 (MAJOR):** the host-login check was a plain substring scan. It would trip on "root-protected" when run as root
  and pass trivially for short logins. It was replaced by exact state checks plus a whole-word scan with a documented
  skip list.
- **Other findings:**
  - mock camera thread replaced by a static camera (F-2);
  - drift tolerance tightened to 16 levels on at most 0.002 % of pixels (F-3);
  - update mode refused under `CI`, and the failure names the script (F-4);
  - Docker deviation recorded (F-5);
  - fixture key sets pinned against the remote sources (F-6);
  - browser PNG stripping, plus viewport and variant pinning (F-7);
  - static loopback-bind check (F-8);
  - fact fixes: base `6f32ef8`, `MODEL_ID_EMBEDDING`, explicit `LatestFrameData` (F-9);
  - exact RAC15 rule (F-10);
  - example printing (F-11);
  - mock-output disclosure (F-12).

**Round 2: APPROVED**, with three non-blocking notes:
- a login equal to an unlisted UI word fails loudly, so run the tests under another account;
- the first CI run must confirm the cross-host tolerance;
- 1 pixel per point is a cosmetic choice.

## 4. Tester Contract

| Test | Criterion | Red evidence |
|---|---|---|
| `readme_snapshots::test_rac_capture_scenes_match_committed_assets` | RAC1 | `E0432 unresolved import soos_gui::app::capture` |
| `readme_snapshots::test_rac_capture_render_is_deterministic` | RAC2 | same |
| `readme_snapshots::test_rac_capture_scenes_show_only_the_synthetic_identity` | RAC3 | same |
| `readme_snapshots::test_rac_capture_app_rejects_non_rgb_frames` | RAC4 | same |
| `readme_snapshots::test_rac_rasterizer_*` (3) | RAC5 | same (whole test target) |
| `readme_snapshots::test_rac_png_writer_emits_only_critical_chunks`, `test_rac_png_round_trip` | RAC6 | same |
| `readme_fixture_tests::test_rac_placeholder_face_is_deterministic_and_centred` | RAC7 | `E0432 unresolved import soos_test_fixtures::readme` |
| `readme_fixture_tests::test_rac_web_routes_serve_only_allow_listed_assets`, `test_rac_web_fixture_server_serves_assets_and_refuses_the_rest` | RAC8 | same |
| `readme_fixture_tests::test_rac_web_fixture_bodies_carry_no_identity`, `test_rac_web_fixture_keys_match_the_remote_types`, `test_rac_strip_png_metadata_keeps_only_image_chunks` | RAC9 | same |
| `readme_assets_contract::test_rac_readme_images_resolve_under_docs_assets_with_alt_text`, `test_rac_readme_image_refs_are_checked` | RAC10 | passed from the start (README has no image yet; synthetic cases) |
| `readme_assets_contract::test_rac_docs_assets_are_listed_in_the_manifest_with_matching_sha256` | RAC11 | `Docs/assets/MANIFEST.toml must exist: No such file or directory` |
| `readme_assets_contract::test_rac_docs_assets_stay_within_the_budget` | RAC12 | `Docs/assets: No such file or directory` |
| `readme_assets_contract::test_rac_docs_assets_carry_no_metadata_and_no_animation` | RAC13 | same |
| `readme_assets_contract::test_rac_png_webp_svg_metadata_detectors`, `test_rac_sha256_known_vectors`, `test_rac_manifest_parser_rejects_malformed_entries` | RAC13, RAC14, RAC11 | detector unit tests, green by construction |
| `readme_assets_contract::test_rac_capture_tooling_is_dev_only` | RAC15 | `readme-capture` feature absent |
| `readme_assets_contract::test_rac_capture_scripts_are_hardware_free` | RAC16 | `Cannot read scripts/capture_readme_assets.sh` |

**Contract amendments**, made by the tester before the implementation was complete. They are listed for
transparency, and no existing repository test changed.
- **TA-1 (RAC3)**: the draft asserted that every scene shows the header text `30 FPS · 12.5 ms latency · 640×480`. At
  1120 points with three tabs, the header moves the frame statistics into a tooltip (`render_header`, existing
  behaviour), so the text is never painted. The assertion became: the live scene shows `12.5 ms`, and no scene shows
  any other `… ms` value. This keeps the intent (fixed telemetry only, never a measured value).
- **TA-2 (lint only)**: Clippy 1.98 `chunks_exact` with a constant size became `as_chunks::<N>()`, and
  `&[red.clone()]` became `std::slice::from_ref(&red)` in the test and harness code. Assertions are untouched.

Determinism: two update runs and the manifest rewrite produced identical SHA-256 digests for the three PNGs.

## 5. Auditor Constraints

`AI/auditor_constraints_readme_capture.md`, AC-1 to AC-12, **Clearance: CLEARED**. How they are met:
- **Feature gate**: the module is declared under `#[cfg(feature = "readme-capture")]`, and `SoosApp::new` and `ui` are
  untouched (AC-1, AC-2).
- **No side effects**: static camera, fixed probe, refusing executor, Polkit store without listing (AC-3).
- **Identity**: `alice` / 1000 overwrite the host defaults (AC-4).
- **Lints**: no `unwrap`, `expect` or unchecked indexing in `capture.rs` or `tests/fixtures/mod.rs`, under clippy
  `-D warnings` (AC-5).
- **Fixture server**: binds `127.0.0.1:0` only; `route` is total and maps requests only to allow-listed names (AC-6).
  Its bounds are constants: 8 KiB head, 2 s timeouts, 16 connections, 1–300 s lifetime (AC-7).
- **PNG metadata**: the stripper refuses truncation, a missing `IEND` and `acTL` (AC-8). The writer emits critical
  chunks only, and the invariant checks the committed assets (AC-9).
- **CI**: update mode is refused under `CI` (AC-10).
- **Dependencies**: the fixtures crate keeps `soos-frame` as its only dependency (AC-11).
- **Logging**: no logging in the new code (AC-12).

## 6. Implementation

- **`soos-gui`**:
  - `crates/gui/Cargo.toml`: `[features] readme-capture = []`; dev-dependencies `soos-test-fixtures`, `soos-frame` and
    `png = "0.18"` (already locked at 0.18.1). `Cargo.lock` gains only dependency edges.
  - `crates/gui/src/app.rs`: the feature-gated `pub mod capture;`.
  - `crates/gui/src/app/capture.rs`: new.
  - `crates/gui/tests/readme_snapshots.rs` and `crates/gui/tests/readme_capture/{mod,raster,png_io}.rs`: new. The
    rasterizer uses pixel-centre sampling, the top-left rule, premultiplied gamma-space blending, bilinear or nearest
    fetch, partial deltas, and frees after painting.
- **Fixtures**:
  - `tests/fixtures/mod.rs`: new modules `readme` and `readme_web`.
  - `tests/fixtures/examples/readme_web_fixture_server.rs` and `tests/fixtures/tests/readme_fixture_tests.rs`: new.
- **Invariant**: `tests/invariants/src/readme_assets_contract.rs`, registered in `lib.rs`. It is pure std and carries
  its own SHA-256, manifest grammar, PNG/WebP/SVG walkers and README image parser.
- **Assets and scripts**:
  - `Docs/assets/MANIFEST.toml` and `Docs/assets/screenshots/gui-{live-inspection,guided-enrollment,profiles}.png`:
    about 195 KB in total.
  - `scripts/capture_readme_assets.sh` and `scripts/capture_readme_web.sh`.
- **Docs**:
  - `Docs/README_ASSETS.md`: new, indexed in `Docs/README.md`.
  - `Docs/GUI_APPLICATION.md` §4 and `Docs/DEVELOPMENT_WORKFLOW.md` §9.
  - `AI/DECISIONS.md` (ADR), `AI/VERIFICATION_MATRIX.md` (RAC1–RAC18) and this walkthrough.

## 7. Candid Review

Not run by this agent. The orchestrator handles candid review layer 2, commit and push after owner approval. Layer 1
(`./scripts/candid_review.sh`) result: see §8.

## 8. Verification Results

All runs used a private `CARGO_TARGET_DIR`, `CARGO_BUILD_JOBS=4` and `--test-threads=4`:
- `cargo fmt --all -- --check`
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
- `cargo test --locked --workspace --all-targets --all-features`
- `cargo deny --locked check`
- `./scripts/candid_review.sh`

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy ... -D warnings` | pass |
| `cargo test ...` | pass: 3574 passed, 0 failed, 6 ignored (workspace); `soos-invariants` 614 passed after the documentation edits |
| `cargo deny --locked check` | advisories ok, bans ok, licenses ok, sources ok |
| `./scripts/candid_review.sh` (layer 1) | PASSED (new scripts are untracked, so they were checked with `bash -n` separately: pass) |

The snapshot test runs in about 45 s in a debug build. The three PNGs are about 195 KB in total.

## 9. Known Limitations / Follow-ups

- **Web captures not run**: the phone web app captures (RAC18) need an installed Chrome/Chromium. The owner runs
  `scripts/capture_readme_web.sh` and checks the images; no `web-*.png` is committed yet.
- **Cross-host tolerance unconfirmed**: the first CI run on Ubuntu must confirm the drift tolerance for PNGs generated on
  the Arch host (RAC17). If it fails, regenerate the PNGs in a matching environment; never loosen the constants.
- **Owner decisions pending**: OD-1 to OD-4 remain assumed until the owner confirms.
- **Mock values in screenshots**: the detection, liveness and match values come from mock models on a drawn face. PR 2
  captions must say so.
- **Cosmetic choices**: 1 pixel per point (sharper HiDPI images would need about 4× the bytes), and the video panel
  shows square corners where the live GUI may differ slightly. Both can be revisited in the visual refresh (PR 3).
