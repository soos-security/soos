# Plan Evaluation Report
- **Date**: 2026-10-17
- **Issue**: GitHub #412 (part of epic #352; follow-up of PR #403, walkthrough 214 §9) — lazy second
  shared-memory buffer of `soos-lock-widget`
- **Branch**: `fix/lock-widget-memory`
- **Base commit**: `9e82795`
- **Plan evaluated**: `AI/architect_spec_lock_widget_memory.md` §1–§7

## 1. Coverage Matrix

| Requirement (issue #412) | Spec element | Status |
|---|---|---|
| Budgets and `buffer_len` unchanged, LKA9 tests untouched | §3 (constants kept, `BUFFERS_PER_SURFACE` kept as an alias) | Covered |
| `MAX_BUFFERS_PER_SURFACE = 2` replaces the eager count | §3 | Covered |
| Pure allocation policy | §3 `needs_extra_buffer` | Covered |
| Byte helper asserted in tests | §3 `slot_len`, `surface_shm_bytes`; §5 | Covered |
| One buffer at configure time | §4 `configure_surface` | Covered |
| Reuse a released buffer; second buffer only when all are busy; first use is `paint_full` | §4 `draw_all` 1–2 | Covered |
| Failed allocation = busy: `retry`, no stderr line, no state logged | §4 `draw_all` 3 | Covered |
| Damage, commit, timer, retry, seat-less binding, exit codes, stderr unchanged | §4, §7 | Covered |
| Module docs, `Docs/LOCKSCREEN_WIDGET.md`, ADR note, matrix, walkthrough | §4, §6 | Covered |
| Owner hardware row | §6 LKA19 (pending) | Covered |

## 2. Checks Against the Reference Documents

- **AI/ARCHITECTURE.md**: `crates/lock-widget` is a user-level swaylock-plugin layer-shell program (ADR
  2026-10-15); nothing in PAM, the daemon or the IPC changes. No Tokio, no seat. Pass.
- **AI/DECISIONS.md**: ADR 2026-10-15 item (5) (no new crate) holds: `SlotPool` and `Shm` are already used.
  Item (6) "two buffers per surface" changes in allocation timing only; the spec records an amending ADR
  note rather than silently diverging. The pixel budgets of item (6) are unchanged. Pass.
- **AI/BACKLOG.md**: no backlog id (GitHub-only branch, like `feat/lockscreen-adapters`); no backlog edit.
- **AI/VERIFICATION_MATRIX.md**: LKA18/LKA19 are unused on origin/main. LKA9 evidence is untouched. The new
  evidence names `test_lka18_*`, which `matrix_citations` resolves once the tests exist.
- **Code**: verified in smithay-client-toolkit 0.20.0 that (a) `SlotPool::new_slot` rounds slots to 64 bytes and
  `alloc` grows to at least twice the pool when a slot does not fit, hence the exact `slot_len` pool size; (b)
  `RawPool::resize` records the new length before `set_len` and the remap, so a failed growth corrupts the
  pool, hence one exactly-sized pool per buffer instead of growing (a deviation from the issue's "the
  SlotPool grows" wording, with identical memory and behaviour, justified in spec §4).

## 3. Risks

| Risk | Mitigation | Verdict |
|---|---|---|
| Removing `BUFFERS_PER_SURFACE` breaks `test_lka9_memory_bounds` (immutable contract) | Kept as an alias equal to `MAX_BUFFERS_PER_SURFACE` | Resolved |
| The Wayland loop has no CI compositor | Policy and sizes are pure and unit-tested; LKA19 owner hardware row | Accepted (pending row) |
| A compositor that never releases | At most 2 buffers (policy), retry timer as before | Unchanged worst case |
| Static LKA12 scans (no `unwrap`, no print macro, no seat) | Constraint list for the developer | Covered |

VALIDATION_VERDICT: APPROVED
