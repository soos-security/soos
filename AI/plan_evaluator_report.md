# Plan Evaluation Report — Round 3
- **Date**: 2026-10-13
- **Issue**: GitHub-only #378 (PR 1 of #353) — UVC extension-unit IR emitter control and stream strobe classification
- **Branch**: `feat/ir-antispoofing-mock`
- **Base commit**: `6f32ef8`
- **Plan evaluated**: `AI/architect_spec_ir_emitter_control.md`, §1–§8 amended by §9 "Revision 2" (R1–R16) and §10 "Revision 3" (R17–R20)
- **Scope note**: only the plan is judged here, not the Phase 2 draft tests.

## 1. Coverage Matrix

| Acceptance line | Spec element | Status |
|---|---|---|
| `IrEmitter` `set` / `get` / capability | §2.2 + R1 | Covered |
| `UvcXuEmitter`, only new `unsafe`, `// SAFETY:` | §2.1, §2.2, R11, R14 | Covered |
| `MockIrEmitter` drives lit/unlit frames | §2.5 + R17 | Covered |
| Strobe classes + synthetic fixtures | §2.4 + R5, R6, R15, R18 | Covered |
| A class change is reported (later veto) | §2.4 + R7 | Covered |
| `[camera.ir_emitter]` fail-closed, default off | §2.3, §3, R10 | Covered |
| `probe` read-only; `configure` writes the device only with an explicit flag; root; Infrared node; bounded; restore incl. Ctrl-C; `--dry-run` | §2.6 + R2, R3, R8, R16, R19, R20 | Covered |
| Mock XU backend, no sudo | §2.5, §7 | Covered |
| `soos-frame` `ir` feature | OD-2 (nothing added) | Covered |
| Docs, `IRA` rows (hardware rows pending), walkthrough | §8, R10, R13, R15 | Covered |
| #353 VID:PID mismatch refuses start-up | AD-3 + R13: `⬜ Pending (split item 4)` | Deferred, tracked |

## 2. Resolution of Round 2 Findings

| Finding | Revision | Verified against code | Status |
|---|---|---|---|
| N1 MAJOR — R12 relied on `notify_activity` while frozen | R17: `step_frame()` (`mock-camera` only) produces exactly one frame through `frame_source`, even while frozen, without touching readiness, warm-up or activity, and returns it; `set_frozen`/`notify_activity` semantics unchanged; the wake frame is routed through `frame_source` (identical output for `new`) | `crates/camera-v4l/src/mock.rs`: shared atomic `sequence`, `store_frame_monotonic` keeps the highest sequence; the frozen contract used by `crates/daemon/tests/pipeline_integration_tests.rs` is untouched | Resolved |
| N2 — canonical pattern | R18: `[on, on, off, off, on, off, off, on]`, plus a parity-aligned pattern on a controllable scene gives `AmbiguousPattern` | Checked: p=2 (s0 on / s2 off), p=3 (s0 on / s3 off), p=4 (s1 on / s5 off): not a function of `sequence % p`; 4 on and 4 off ≥ `STROBE_MIN_PER_GROUP` | Resolved |
| N3 — no-thread check scope | R19: also covers `crates/admin-cli/src/main.rs` | `main.rs` spawns no thread today | Resolved |
| N4 — probe duration | R20: documented worst case ≈ 16 × 5 s ≈ 80 s; ENOENT answers are immediate | consistent with R16 | Resolved (no cap, acceptable for an interactive root diagnostic) |

## 3. Pillar Analysis (revision 3 deltas)

### Pillar 1 — Architecture & threat model
- `step_frame` exists only under `mock-camera`, which no production manifest enables. It is not referenced by `crates/admin-cli/src`, so `STREAMING_TOKENS` is unaffected. PASS.

### Pillar 2 — PAM deadline & concurrency
- Unchanged: PAM is untouched. PASS.

### Pillar 3 — Panic safety & fail-closed
- Scenario: a test calls `step_frame()` while the background thread is not frozen. Both generators draw from the same atomic sequence, and the monotonic store keeps the newest frame. Because `step_frame` returns its own frame, the test does not need to race `latest_frame`. PASS.

### Pillar 4 — Dependencies
- No change. PASS.

### Pillar 5 — Data confidentiality
- Frames are synthetic only. PASS.

### Pillar 6 — Test integrity & power
- Scenario: the implementation treats any 2-state pattern as `Controllable` without the period check. Then `test_ira_parity_aligned_pattern_is_ambiguous` fails. If the period check instead rejects every pattern, the R18 canonical pattern test fails. Both wrong implementations are caught. PASS.
- No existing test is modified, and the frozen semantics are preserved. PASS.

## 4. Findings

None blocking. Observation, no action required: the hand check of the R18 pattern belongs in the tester contract as stated. A small helper assertion, "not a function of `sequence % p` for p in 2..=4", would keep the property if the pattern is ever edited.

## 5. Verdict
VALIDATION_VERDICT: APPROVED
