# Architect Spec — GitHub #412: lazy second shared-memory buffer of `soos-lock-widget`

- **Date**: 2026-10-17 — **Branch**: `fix/lock-widget-memory` (GitHub-only, not registered in
  `scripts/sync_issue.py`, like `feat/lockscreen-adapters`) — **Epic**: #352 — **Follow-up of**: PR #403
  (walkthrough 214 §9, memory item).
- **ADRs applied**: ADR 2026-10-15 "Lock-Screen Adapters for swaylock-plugin and hyprlock" items (5) and (6);
  item (6) "two buffers per surface" is amended by a dated note (§6 below). No owner decision is needed: every
  budget, every visible behaviour and every LKA1–LKA13 contract stays as is.
- **Crate touched**: `crates/lock-widget` only (`src/layout.rs`, `src/wayland.rs`); no new dependency.

## 1. Problem

`OutputSurface` allocates two full-size ARGB8888 buffers in one `SlotPool` at configure time
(`BUFFERS_PER_SURFACE = 2`). At the pixel budget (two surfaces of `MAX_SURFACE_PIXELS` =
33 554 432 px, i.e. `MAX_TOTAL_PIXELS`) that is 4 × 128 MiB = 512 MiB of mapped shared memory. The second
buffer is only used while the compositor still holds the first one; a compositor that releases an shm buffer
right after it has read it (the usual wlroots behaviour: the buffer is uploaded on commit) never needs it.

## 2. Rejected alternative (finding recorded in the ADR note)

Walkthrough 214 §9 suggested "one full buffer plus small widget-box buffers". A layer surface takes one
full-size buffer, so a smaller buffer needs either a `wl_subsurface` or `wp_viewporter` scaling:

- swaylock-plugin's nested server (owner binary `~/.local/bin/swaylock-plugin`, inspected read-only on
  2026-10-17) implements `nested_surface_*`, `nested_viewport_*`, `nested_shm_pool_*`, dmabuf, colour and
  fractional-scale handlers, and **no** `nested_subsurface_*` / `nested_subcompositor_*` handler: its only
  `wl_subcompositor` strings belong to its own client side ("Missing wl_subcompositor" for the outer compositor).
  The widget cannot rely on a subsurface.
- `wp_viewporter` would upscale a smaller buffer (blur): a visible change, out of scope.
- Lowering `MAX_TOTAL_PIXELS` would drop the widget on two 8K outputs: a visible change, out of scope.

## 3. `src/layout.rs` (pure, unit-tested)

Unchanged: `MAX_OUTPUTS`, `MAX_SURFACE_DIM`, `MAX_SURFACE_PIXELS`, `MAX_TOTAL_PIXELS`, `buffer_len`,
`WIDGET_CENTER_Y_FRACTION`, `Rect`, `Layout` (the LKA9 tests stay untouched).

```rust
/// Buffers per surface at most: one is allocated at configure time; a second only while the compositor
/// still holds every allocated one (see [`needs_extra_buffer`]).
pub const MAX_BUFFERS_PER_SURFACE: usize = 2;
/// Former name of [`MAX_BUFFERS_PER_SURFACE`], kept for the LKA9 contract (`test_lka9_memory_bounds`
/// asserts it equals 2). It is the upper bound, not the count allocated up front.
pub const BUFFERS_PER_SURFACE: usize = MAX_BUFFERS_PER_SURFACE;
/// Alignment of a buffer slot in a shared-memory pool: smithay-client-toolkit's `SlotPool::new_slot`
/// rounds every slot up to 64 bytes.
pub const SHM_SLOT_ALIGN: usize = 64;

/// Bytes of the shared-memory pool holding one buffer of `width` × `height`: [`buffer_len`] rounded up to
/// [`SHM_SLOT_ALIGN`]; `None` exactly when `buffer_len` is `None` (or on overflow).
pub fn slot_len(width: u32, height: u32) -> Option<usize>;

/// Shared-memory bytes of one surface holding `buffers` buffers; `None` when `buffers` is 0 or above
/// [`MAX_BUFFERS_PER_SURFACE`], or when [`slot_len`] is `None`.
pub fn surface_shm_bytes(width: u32, height: u32, buffers: usize) -> Option<usize>;

/// Allocation policy: whether a surface holding `allocated` buffers, `any_free` of which is not held by
/// the compositor, may allocate one more: only when none is free and fewer than
/// [`MAX_BUFFERS_PER_SURFACE`] exist.
pub const fn needs_extra_buffer(allocated: usize, any_free: bool) -> bool {
    !any_free && allocated < MAX_BUFFERS_PER_SURFACE
}
```

`BUFFERS_PER_SURFACE` is **kept** (as an alias) rather than removed: removing or renaming it would break
`test_lka9_memory_bounds`, an existing contract that must not be modified (AGENTS.md Zero Test Weakening).
It is no longer used by production code.

`slot_len` uses `usize::checked_next_multiple_of(SHM_SLOT_ALIGN)` (no arithmetic side effects).

## 4. `src/wayland.rs`

```rust
/// One shared-memory buffer and its own pool.
struct ShmBuffer {
    buffer: Buffer,      // declared first: dropped before its pool
    pool: SlotPool,
    painted: bool,       // painted fully once
}
struct OutputSurface { output, layer, buffers: Vec<ShmBuffer>, layout: Option<Layout>, pixels: u64 }

/// One buffer of `width` × `height` in a pool of exactly `slot_len` bytes; `None` on any failure
/// (no partial state survives).
fn new_shm_buffer(shm: &Shm, width: u32, height: u32) -> Option<ShmBuffer>;
```

- **One pool per buffer, not a growing pool.** The issue text suggests growing the same `SlotPool`.
  Verified in smithay-client-toolkit 0.20.0 `src/shm/raw.rs::RawPool::resize`: it stores the new length
  *before* `set_len` and the remap, so a failed growth leaves a pool whose recorded length exceeds its file and
  mapping (a later slot there would be out of the mapping: a panic inside the toolkit or a protocol error).
  `SlotPool::new` fails cleanly instead (the file and mapping are dropped). A separate exactly-sized pool for
  the second buffer has the same memory cost, keeps "failed allocation = busy" side-effect free, and needs no
  `resize`. swaylock-plugin's nested server implements `shm_create_pool` (the widget already creates one pool
  per surface).
- **Exact pool size.** The pool length is `slot_len`, not `buffer_len`: `SlotPool::alloc` grows to at least
  twice the pool when a 64-byte-rounded slot does not fit, so an unaligned `buffer_len` pool would double on
  its first buffer (the eager code had the same latent over-allocation: a `2 × buffer_len` pool with two
  rounded slots grew to `4 × buffer_len` when `width × height` is not a multiple of 16).
- `configure_surface`: unchanged budget checks and early returns; on a new size it drops the old buffers and
  creates **one** buffer with `new_shm_buffer`; failure → `false` (same stderr skip line as today).
- `draw_all`, per surface with a layout:
  1. `free` = first buffer whose `canvas` is available (unchanged reuse branch);
  2. if `needs_extra_buffer(buffers.len(), free.is_some())` and `new_shm_buffer` succeeds, push it
     (`painted = false`, so its first use is `paint_full` with full damage) and use it;
  3. no buffer usable → `retry = true; continue` (a failed allocation is a busy buffer: no stderr line, the
     state is never logged; the frame timer retries every `AMBIENT_REPAINT_MS`).
  Damage, attach, commit, timer, retry, seat-less binding, exit codes and stderr rules are unchanged.
- Module docs: "draws with one shared-memory buffer per surface, and a second only while the compositor still
  holds the first".

## 5. Memory outcome

| Case | Before | After |
|---|---|---|
| Two budget-size outputs (2 × 33 554 432 px), compositor releases promptly | 512 MiB | 256 MiB |
| Same, compositor keeps holding the committed buffer | 512 MiB | 512 MiB (unchanged worst case) |
| One 7680 × 4320 output | 2 × 132 710 400 B | 132 710 400 B (≈ 126.6 MiB) |

Frames drawn and pixels shown are identical: a newly allocated buffer is painted fully, exactly like the
first use of the second eager buffer before.

## 6. Traceability

- Matrix (component `lockscreen-adapters`): new rows LKA18 (policy and byte helper, `test_lka18_*`, ✅ once
  green) and LKA19 (owner hardware: the widget still animates under swaylock-plugin on driftwm with the lazy
  buffer; ⬜ pending). LKA9 unchanged. Prefixes LKA18/LKA19 are unused on origin/main.
- `AI/DECISIONS.md`: new dated ADR "[2026-10-17] Lazy Second Shared-Memory Buffer of soos-lock-widget"
  amending item (6) and recording the §2 finding.
- `Docs/LOCKSCREEN_WIDGET.md` budgets paragraph; walkthrough `AI/walkthroughs/218_lock_widget_memory.md`
  closing walkthrough 214 §9's memory item.

## 7. Invariants touched

`#![forbid(unsafe_code)]` kept; no `unwrap`/`expect`/panic macro/print macro in production (LKA12 scans);
no seat; no new crate in `Cargo.lock`; nothing about the lock state is written to stderr.
