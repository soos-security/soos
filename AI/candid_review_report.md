# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `feat/gui-motion`
- **Base (merge-base)**: `3f76090`
- **Reviewed-Diff-Fingerprint**: `0b57aff928278602583af70e64b752b7146c8eb43259a5c99a0b569abaefbc15`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_gui_motion.md`, `AI/auditor_constraints_gui_motion.md`, `AI/tester_contract_gui_motion.md`, `AI/walkthroughs/234_gui_motion.md`, `Cargo.lock`, `Docs/GUI_APPLICATION.md`, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`, `Docs/UI_MOTION.md`, `crates/gui/Cargo.toml`, `crates/gui/src/app.rs`, `crates/gui/src/app/capture.rs`, `crates/gui/src/args.rs`, `crates/gui/src/brand.rs`, `crates/gui/src/camera_status.rs`, `crates/gui/src/header.rs`, `crates/gui/src/lib.rs`, `crates/gui/src/main.rs`, `crates/gui/src/motion.rs`, `crates/gui/src/motion/portal.rs`, `crates/gui/src/overlay.rs`, `crates/gui/src/theme.rs`, `crates/gui/src/widgets.rs`, `crates/gui/tests/motion_tests.rs`, `tests/invariants/src/gui_motion_contract.rs`, `tests/invariants/src/lib.rs`, `tests/invariants/src/presence_unlock_contract.rs`

## 1. Executive Summary

The branch adds the `soos-gui` motion system (GitHub #438 = PR 3 of #358): motion tokens in `theme.rs`, a
pure `motion.rs` (closed-form spring, cubic-bezier, tween, entrances, pending star, reduced-motion resolution)
with egui glue, a read-only XDG portal reader `motion/portal.rs` (zbus + current-thread Tokio on one background
thread, 200 ms deadline), animated widgets, and the face overlay moved verbatim into `overlay.rs`. The code is
sound: no panic path, no blocking on the UI thread, bounded state, the overlay is not interpolated, and no
security-relevant state depends on animation. Re-run here: `cargo test --locked -p soos-gui -p soos-invariants`
(793 invariants passed), `cargo test --locked -p soos-gui --all-features` (motion_tests 33, readme_snapshots 9,
responsiveness_tests 11, all green), `cargo clippy --locked -p soos-gui -p soos-invariants --all-targets
--all-features -- -D warnings` clean, `cargo fmt --all -- --check` clean (all with the scoped environment and
`--test-threads=3`).

However, the diff **changes a pre-existing assertion on `origin/main`**:
`presence_unlock_contract::test_pau_zbus_is_used_only_by_the_daemon` (PAU17) widens its `ALLOWED` list from two
to three manifests. This is the Planned Contract Migration B.2a already recorded on `origin/main` (ADR
2026-10-13 "Motion System for soos-gui and soos-remote" item (6) and `AI/tester_contract_motion.md` §B.2a, both
from #384), so it is justified under the skill rules, but the standing rule of this run states that any changed
pre-existing assertion yields `CHANGES_REQUESTED`. The verdict is therefore CHANGES_REQUESTED for that process
reason; no code defect of MAJOR or CRITICAL severity was found.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- `grep '^-[^-].*(assert|#[test]|...)'` on the frozen patch: **no removed line**.
- `grep '^+.*(#[ignore|should_panic|tolerance|epsilon)'`: one hit, a comment in the new
  `crates/gui/tests/motion_tests.rs` ("the epsilon is relative to the span"); no escape hatch.
- `grep '^[-+].*mod tests'`: none.
- Test files touched:
  - `crates/gui/tests/motion_tests.rs` (new, 33 tests, GMO1–GMO14).
  - `tests/invariants/src/gui_motion_contract.rs` (new, GMO15–GMO19 plus a helper self-test).
  - `tests/invariants/src/lib.rs`: module registration only.
  - `tests/invariants/src/presence_unlock_contract.rs`: **pre-existing test changed**. In
    `test_pau_zbus_is_used_only_by_the_daemon`, `const ALLOWED: [&str; 2] = ["crates/daemon/Cargo.toml",
    "crates/remote/Cargo.toml"]` becomes `[&str; 3]` with `"crates/gui/Cargo.toml"` added. Every `assert!` /
    `assert_eq!` line and the test name are textually unchanged, but the set they check is widened
    (`crates/gui/Cargo.toml` is now required to declare zbus once instead of being forbidden to name it).
    Justification found on `origin/main`: ADR 2026-10-13 item (6) ("The later `feat/gui-motion` PR applies a
    planned Contract Migration of ... PAU17") and `AI/tester_contract_motion.md` §B.2a (commit `3d9879a`, #384).
    The change matches that table exactly.
- `git diff $(git merge-base origin/main HEAD) HEAD -- tests 'crates/*/tests' android/app/src/test`: the same
  four files; no other pre-existing assertion touched; `android/app/src/test` untouched.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Resolution order (`resolve_motion_prefs`, `MotionControl::from_sources`): `--reduce-motion` → env exactly
  `1`/`0` (≤ 8 bytes, byte compare via `as_encoded_bytes`) → portal (`Pending` counts as reduced) → full. Tried:
  env `"1\n"`, `"10"`, 9 bytes, non-UTF-8 → ignored, portal consulted. PASS. The portal is never spawned when the
  switch or the variable decides. PASS.
- `PortalReader::poll_at`: a `Pending` answer from the thread, a disconnected channel and a missing receiver all
  map to `Unavailable`; `Empty` after the deadline → `Unavailable`, cached; a late answer is discarded because the
  receiver is dropped. PASS.
- `ZbusPortalBus`: a `ReadOne` reply `v` deserializes to the inner value; a `Read` reply `v(v)` is unwrapped by
  `setting_value(.., 1)`; deeper nesting or another type → `Other` → fallback or `Unavailable`. UnknownMethod is
  recognised both as `Error::FDO(UnknownMethod)` and as `MethodError("org.freedesktop.DBus.Error.UnknownMethod")`.
  The whole read is under `tokio::time::timeout(deadline, ..)` plus `method_timeout(deadline)`. PASS.
- `SpringState`: closed form with ζ = 0.75 (wd > 0), dt clamped to [0, 0.05], non-finite target ignored,
  non-finite intermediate snaps, 1.6 s termination guard, settle test relative to the span. Tried NaN/±inf dt,
  10 s dt, an infinite span (snaps). PASS.
- `cubic_bezier`: clamped Newton, bisection fallback, exact endpoints, NaN → 0. PASS.
- `pending_star_angle`: in [0, π); an f32 result rounding to exactly π falls back to 0, the same mark (half-turn
  symmetry). PASS.
- Star turn follows `DaemonState::Inactive` only (confirmed state), never a pending action. PASS.
- `banner`: hashed slots (`% 1024`); a collision inside one pass draws at rest; identical texts share a slot
  harmlessly. `banner_slot` copies the text only when it changes. PASS.
- Opacity push/restore pairs in the three tab renderers: no early `return` between push and restore (checked by
  line listing). PASS.
- Overlay moved with identical behaviour (same order, colour from `pad_live` only). PASS.
- `Docs/UI_MOTION.md` G4 and G7 are reworded by this implementation branch → MINOR finding 2.

### PAM Concurrency & Deadlines

- `crates/pam` untouched; zbus/Tokio not added to it (PAU17 PAM assertion still runs and passes). The new Tokio
  runtime lives only on the GUI's `soos-motion-portal` thread (GMO16). PASS.

### Panic Safety & Fail-Closed

- `motion.rs` denies `indexing_slicing`, `unwrap_used`, `expect_used`, `panic`; `portal.rs` has no unwrap/expect
  (`NonZeroUsize` const via `match`); a runtime build or connection failure returns default readings; a panic in
  the reader thread drops the sender → `Unavailable`. No PAM or authorization path is reachable from this code;
  its only output is a cosmetic boolean. PASS.

### Test Integrity & Anti-Weakening

- See §2. The PAU17 change is the one pre-existing test modification; ADR-mandated, but still a changed assertion
  set → finding 1.
- New tests can fail against plausible wrong implementations (overshoot band 1–4 %, deadline lower bound "never
  before it", disconnected-channel case, `FACE_SPOOF` on the flip frame, reduced motion with no repaint). The
  wall-clock check is an upper bound with a 2 s CI margin, not a tight bound. PASS.

### Memory, Bounds & Secrets

- Connection `max_queued(8)`; one `sync_channel(1)`; at most four method calls; banner state at most 1024 slots
  plus one `banner_slot` string. The preference value is never logged (no logging macro in `portal.rs`, GMO18).
  No frames, embeddings or credentials are touched. The session bus address comes from the environment, as the
  ADR and the guideline state; a hostile session bus can only choose a cosmetic preference. PASS.

### Supply Chain & Automation

- `Cargo.lock`: only the `soos-gui` dependency list gains `tokio` and `zbus`; no new package. Both are
  `{ workspace = true }` (workspace zbus 6 `default-features = false, features = ["tokio"]`). No `.github/`,
  `deny.toml` or script change. PASS.

### English-Only Policy

- All code, comments and docs in the diff are English. PASS.

## 4. Detailed Findings & Action Items

- **[MAJOR]** `tests/invariants/src/presence_unlock_contract.rs:202` — a pre-existing test on `origin/main`
  (PAU17 `test_pau_zbus_is_used_only_by_the_daemon`) has its checked set changed (`ALLOWED` 2 → 3 entries, adding
  `crates/gui/Cargo.toml`). It exactly matches the Planned Contract Migration B.2a already recorded on
  `origin/main` (ADR 2026-10-13 item (6), `AI/tester_contract_motion.md` §B.2a), and no `assert` line or the test
  name changed, but the standing rule of this run forbids modifying an existing test assertion and requires
  `CHANGES_REQUESTED` for any such change. Required action: not a code fix. The owner (or the orchestrator on the
  owner's documented behalf) must explicitly accept this ADR-mandated migration for this PR; otherwise the portal
  reader must be replaced by a design that does not need zbus in `soos-gui`.
- **[MINOR]** `Docs/UI_MOTION.md` G4/G7 — the normative spec is amended inside the implementation PR (G7 drops the
  old-out/new-in cross-fade; G4 limits stat fades to caption/presence changes). Both changes are safer, but the
  owner's prototype approval (UIM7) refers to this spec; the PR description should flag the rewording.
- **[MINOR]** `crates/gui/src/camera_status.rs:150`, `Docs/UI_MOTION.md` G7, `Docs/GUI_APPLICATION.md` — "a new
  banner state is drawn on the frame it arrives" / "drawn at once": with full motion `fade_in_on_change` returns
  opacity `cubic_bezier(EASE_ENTER, 0) = 0` on that first frame, so the new text is painted but invisible on its
  first frame and becomes legible over the following frames (180 ms). The security point (the old text is never
  kept) holds; the wording overstates it. Suggest "replaces the previous text on the frame it arrives and fades
  in".
- **[SUGGESTION]** `crates/gui/src/widgets.rs` `banner_slot` — the retained text is cloned once per frame during
  the 126 ms exit; borrowing it inside the `data_mut` scope or storing an `Arc<str>` would avoid that.

## 5. Final Verdict

**VERDICT: CHANGES_REQUESTED**
