# Walkthrough 183 — `soos-gui` Brand Redesign

- **Date**: 2026-10-06
- **Issue**: no backlog id (owner request: restyle every `soos-gui` page to the soos design
  direction) — **Branch**: `feat/gui-brand-redesign`
- **Matrix criteria**: GUX1–GUX13 (component `gui-brand-redesign`)

## 1. Context & Objectives

The owner supplied a design direction archive: a 1512x982 mockup of the Live Model Diagnostic
page (`New_Gui_Interface.png`), the palette (`Colors_Direction.svg`), the star logo, the "SOOS"
wordmark and brand compositions. Objective: make every `soos-gui` page match that direction
while keeping every behavior, threading rule, privileged flow, fail-closed camera rule and
documented message unchanged, with no new crate, no downloaded font and no image loader.

## 2. Architect Design

No separate spec file; the design was carried by the workflow task and is recorded in
`Docs/GUI_APPLICATION.md` §5.

- New modules in `crates/gui/src`: `theme` (palette tokens, sizes, `Metrics`, `content_rect`,
  `fit_video`, `apply`, `faux_bold_offsets` / `paint_faux_bold`), `brand` (SVG path data,
  rasterizer, `paint_wordmark`, `paint_star`, `window_icon`), `widgets` (cards, tiles, banners,
  `BrandButton`, `progress_bar`, overlay chips, checklist, toggle switch, column stacking) and
  `header` (band, tabs, `DaemonSwitch`, daemon panel).
- `app.rs`: `render_header` paints the chrome and returns the body rectangle; the three pages
  are re-laid out with `Metrics::split_columns`. The Pause/Resume buttons become a toggle switch
  that submits the same `PrivilegedAction`s through `submit_privileged`.
- `main.rs`: window icon only; window sizes and the logging order are unchanged.
- The layout formulas change (right column `(259 * k).clamp(250, 300)` instead of the 0.32 / 0.35
  sidebars); `layout_tests.rs` is left untouched and GUX12 checks the minimums on the new
  formulas.

## 3. Plan Evaluation

No `AI/plan_evaluator_report.md` was produced for this pass. The plan was instead checked by a
review pass over screenshots and code (18 findings). Applied: pixel-snapped faux bold, faux-bold
wrapped titles, custom progress bar, centered video, card-first right column with compact
checklist rows, backed video captions, one status-word decision per checklist, Profiles column
filling the height, faux-bold status card title, `egui::Modal` deletion dialog, blue in-progress
dot, compact overlay chips, layout contract test, docs and traceability, match-score gating,
band-text tooltip, non-panicking path mapping. None was rejected.

## 4. Tester Contract

- `crates/gui/tests/theme_tests.rs`: 15 tests (GUX1–GUX8), written before the modules existed
  (red: the crate did not compile without `theme`, `brand`, `header`).
- `crates/gui/tests/brand_layout_tests.rs`: 9 tests (GUX9–GUX13), written together with the
  review fixes (no separate red run was recorded for this file).
- Migrated tests: none. No existing test was modified, weakened or deleted.

## 5. Auditor Constraints

CLEARED.

1. No `unwrap`, `expect` or panic macro in production code; `brand::map_shapes` no longer uses
   `f32::clamp` (normalized rect, non-finite rect maps to nothing).
2. Every new module keeps `#![forbid(unsafe_code)]` and scopes its lint allowances with a reason.
3. The UI thread still never spawns or blocks: `app.rs` keeps `DaemonMonitor`, `TaskRunner`,
   `StoreTask::Enroll`, `StoreTask::Delete`, `CameraSourceSupervisor`, `HandoverExecutor` and no
   `Command::new`.
4. The switch submits only `PauseDaemon` / `ResumeDaemon`, is inert while the state is
   `Unknown` and disabled while an action is pending; `request_refresh()` still follows.
5. Liveness color comes from `frame.pad_live`; `match_threshold` comes from
   `pipeline.config()`; no float literal reached the threshold scans.
6. No frame, embedding or credential is logged or displayed; only scores and percentages.
7. The deletion confirmation keeps its texts and the StoreTask / PrivilegedAction paths; a
   backdrop click or Escape only cancels.
8. No dependency or feature change; default fonts only.

## 6. Implementation

- Palette `BLUE` #0047BB, `PALE` #EDF1FF, `INK` #101820, `PINK` #E59BDC plus tints and state
  colors; `theme::apply` sets the light brand look for both system themes.
- Header: blue band, painted wordmark, folder-shaped active tab with painted icons, daemon panel
  with the toggle switch; frame statistics in the band, or the wordmark tooltip when the band is
  too narrow.
- Faux bold: offsets snapped to whole physical pixels (`faux_bold_offsets`), so headings keep
  their weight at any position; wrapped card titles are bold on every line.
- Live: centered 4:3 video with floating overlay chips (compact when one row does not fit),
  backed captions, telemetry card, stat tile (match score only while the selected profile is
  listed, clamped to 0–100 %) and star tile. A selected profile that leaves the list clears the
  match reference through `select_match_reference(None)`.
- Enrollment: reticle colors, caption pill, enrollment card with a custom progress bar and a
  compact four-step checklist on short columns, fixed action footer, blue in-progress dot.
- Profiles: main card with table, star-filled right column, `egui::Modal` confirmation.
- Right column: card height measured on the previous frame; the stat tile shrinks, then the
  tiles are dropped, before the card scrolls.
- Docs: `Docs/GUI_APPLICATION.md` §1, §1b, §2, §4 updated and §5 added; `Docs/README.md` row.

## 6a. Mock Camera Warmup Flapping

Running `soos-gui --mock` showed the camera status flapping between `Ready` and `Starting` for the
whole warmup (11 transitions in the first second). Root cause: the GUI worker calls
`CameraManager::notify_activity` for every analyzed frame, and `MockCameraManager::notify_activity`
published a synthetic frame and the ready flag even during warmup; the warmup thread withdrew both
on its next tick. This also broke Criterion C5 ("must not serve un-stabilized frames during
warmup").

- Red: new `crates/camera-v4l/tests/mock_warmup_activity_tests.rs` (activity during the initial
  warmup never marks the camera ready nor serves a frame; activity after warmup still serves a
  fresh frame immediately).
- Green: a `stabilized` flag, set when the capture thread publishes its first frame, gates the
  forced frame in `notify_activity`. A wake from suspension (stabilized, re-warmup pending) still
  serves a frame immediately, as `mock_camera_tests::test_mock_camera_idle_throttling_and_wake`
  requires, and now sets `warmup_remaining` to 0 so the thread does not withdraw it. The capture thread
  decrements the counter with a saturating `fetch_update`, so that concurrent reset can never wrap
  it (candid review M4). `stabilized` is never reset: the same shortcut applies after an injected
  error or starvation clears (M5, documented).
- Result: one `Starting -> Ready` transition at startup. No existing test was modified.

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`): PASSED, see §8. Layer 2 (`scripts/candid_subagent.sh`): a context-free
reviewer audited the raw diff and wrote `AI/candid_review_report.md` (VERDICT APPROVED, minor
findings only). Header label for an unknown daemon state with a pending action: the docs now
describe the tested behavior ("Daemon status unknown").

## 8. Verification Results

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean (also `-p soos-gui
  --all-features`).
- `cargo test -p soos-gui` (and `--all-features`): 149 passed, 0 failed.
- `cargo test -p soos-invariants`: 448 passed, 0 failed.
- `cargo test -p soos-camera-v4l -p soos-daemon -p soos-gui` (after the §6a mock camera fix):
  1053 passed, 0 failed, 5 ignored (pre-existing `#[ignore]`); camera tests repeated 3 times, no
  flake. `cargo clippy -p soos-camera-v4l --all-targets -- -D warnings`: clean.
- `./scripts/candid_review.sh`: PASSED (Audits 1–7 OK). It diffs tracked files only, so the
  new untracked files were scanned by hand for `unsafe` and the Audit 7 word list: no match.
- `python3 scripts/sync_issue.py --check`: run by `save.sh` (no backlog id; the branch is not mapped).

## 9. Known Limitations / Follow-ups

- At 900x600 the overlay chips still wrap onto two rows even in compact size.
- The "Pose & Angles" toggle still has no effect (unchanged behavior: the pose rows always show).
- The Completed enrollment state (green reticle, Save button) and the match bar / verdict pill
  were not reached in screenshots with the mock pipeline and an empty developer store.
- The default fonts have one weight; headings use faux bold, which is heavier than a real
  medium weight at small sizes.
