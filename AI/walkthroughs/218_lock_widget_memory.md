# Walkthrough 218 — Lock Widget: Lazy Second Shared-Memory Buffer

- **Date**: 2026-10-17
- **Issue**: GitHub #412, part of the epic #352; follow-up of PR #403 (walkthrough 214 §9, memory item).
  GitHub-only: no `AI/BACKLOG.md` entry, not registered in `scripts/sync_issue.py` (like
  `feat/lockscreen-adapters`). **Branch**: `fix/lock-widget-memory`.
- **ADR**: "[2026-10-17] Lazy Second Shared-Memory Buffer of soos-lock-widget" (`AI/DECISIONS.md`), amending
  item (6) of "[2026-10-15] Lock-Screen Adapters for swaylock-plugin and hyprlock".
- **Documents**: spec `AI/architect_spec_lock_widget_memory.md`; plan evaluation `AI/plan_evaluator_report.md`
  (APPROVED); tester contract `AI/tester_contract_lock_widget_memory.md`; auditor constraints
  `AI/auditor_constraints_lock_widget_memory.md` (CLEARED); user documentation `Docs/LOCKSCREEN_WIDGET.md` §4.
- **Matrix criteria**: LKA18 (verified), LKA19 (pending: owner hardware).

## 1. Context & Objectives

`soos-lock-widget` (PR #403) allocated two full-size ARGB8888 buffers per output when an output was configured.
At the pixel budget (two outputs of 33 554 432 px) that mapped 512 MiB of shared memory, although the second
buffer is only needed while the compositor still holds the first. Walkthrough 214 §9 recorded the follow-up. The
goal is to cut the common case without changing a budget, a frame or a pixel.

## 2. Architect Design

- `layout.rs`: `MAX_BUFFERS_PER_SURFACE = 2` (the old `BUFFERS_PER_SURFACE` stays as an alias of the upper
  bound because `test_lka9_memory_bounds` asserts it), `SHM_SLOT_ALIGN = 64`, `slot_len` (`buffer_len` rounded
  up to the toolkit's slot alignment), `surface_shm_bytes` (bytes for 1 or 2 buffers) and the pure policy
  `needs_extra_buffer(allocated, any_free)`: one more buffer only when none is free and fewer than two exist.
- `wayland.rs`: `ShmBuffer { buffer, pool, painted }`, one exactly-sized `SlotPool` per buffer.
  `configure_surface` creates one buffer; `draw_all` reuses a released buffer, otherwise allocates the second
  one when the policy allows (painted fully on its first use), otherwise retries on the next tick.
- **Rejected idea** (recorded in the ADR): the walkthrough's "small widget-box buffers". A layer surface takes one
  full-size buffer; a smaller one needs a `wl_subsurface`, which swaylock-plugin's nested server does not offer
  (the owner binary has nested surface, viewport, shm-pool, dmabuf, colour and fractional-scale handlers and no
  subsurface or subcompositor handler), or `wp_viewporter` scaling, which would blur. Lowering
  `MAX_TOTAL_PIXELS` would drop the widget on two 8K outputs. Both are visible changes.

## 3. Plan Evaluation

Self-check against `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/BACKLOG.md` and `AI/VERIFICATION_MATRIX.md`:
APPROVED. Two code facts verified in smithay-client-toolkit 0.20.0 drove the design:

1. `RawPool::resize` stores the new length before `set_len` and the remap, so a failed growth of a shared pool
   would leave an inconsistent pool. The issue suggested growing the surface's pool; one pool per buffer gives
   the same memory and makes a failed allocation side-effect free.
2. `SlotPool::new_slot` rounds a slot up to 64 bytes and `alloc` grows to at least twice the pool when a slot
   does not fit. A pool of exactly `slot_len` bytes never grows. (The eager code's `2 × buffer_len` pool grew to
   `4 × buffer_len` mapped bytes whenever `width × height` is not a multiple of 16.)

## 4. Tester Contract

New file `crates/lock-widget/tests/lock_widget_buffer_tests.rs`, seven `test_lka18_*` tests: the two-buffer
cap, no second buffer while one is free, a second only when all are busy, a replay of the selection loop against
three compositor models (prompt release keeps one buffer; hold-until-replaced settles at two; never-release stops
at two and retries), slot alignment, and the documented sizes (128 MiB per budget-size buffer, 256 MiB for two
budget-size outputs with one buffer each, 512 MiB eager worst case, exact bytes for 7680 × 4320 and 1920 × 1080).
Red against signature stubs: 4 of 7 failed on assertions; the 3 passing ones are guards that an eager policy
would break. **Existing tests migrated: none**; no existing test file was modified.

## 5. Auditor Constraints

Ten constraints, CLEARED: no panic or print path, checked size arithmetic, the only push into the buffer list is
gated by the policy, no pool growth (`resize` never called), a failed allocation is a silent retry, a new buffer
is painted fully first, budgets/damage/timer/exit codes unchanged, buffers dropped before their pool, `unsafe`
still forbidden, no new dependency.

## 6. Implementation

- `crates/lock-widget/src/layout.rs`: constants and the three helpers (`checked_next_multiple_of`,
  `checked_mul`).
- `crates/lock-widget/src/wayland.rs`: `ShmBuffer::new` (pool of `slot_len` bytes, one buffer, `None` on any
  failure), `ShmBuffer::is_free`, `OutputSurface.buffers: Vec<ShmBuffer>` (the `pool` field is gone),
  `configure_surface` creates one buffer, `draw_all` applies the policy; module docs updated.
- Docs: `Docs/LOCKSCREEN_WIDGET.md` §4 budgets paragraph; ADR note and a pointer in item (6); matrix LKA18, LKA19.

## 7. Memory Outcome

| Case | Before | After |
|---|---|---|
| Two budget-size outputs, compositor releases promptly | 512 MiB | 256 MiB |
| Two budget-size outputs, compositor holds the last buffer | 512 MiB | 512 MiB |
| One 7680 × 4320 output, prompt release | 265 420 800 B | 132 710 400 B |

## 8. Verification Results

Scoped runs with `CARGO_TARGET_DIR=~/.cache/soos-targets/lock-widget-memory`, `--test-threads=3`:
`cargo fmt -p soos-lock-widget -- --check`; `cargo clippy -p soos-lock-widget --locked --all-targets
[--all-features] -- -D warnings`; `cargo test -p soos-lock-widget --locked --all-features` (every suite green,
including LKA9–LKA13); `cargo test -p soos-invariants --locked` for `lockscreen_adapters_contract` and
`matrix_citations`. `Cargo.lock` unchanged. The full gates run through `./save.sh`.

## 9. Known Limitations / Follow-ups

- LKA19 (owner hardware): confirm the widget still animates under swaylock-plugin on driftwm with the lazy
  buffer. Whether driftwm releases the shm buffer promptly (one buffer) or holds it (two) is not observable in CI.
- This closes the memory item of walkthrough 214 §9; the other 214 §9 items are unchanged.
