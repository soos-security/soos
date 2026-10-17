## Audit Constraints — GitHub #412 (lazy second shared-memory buffer of `soos-lock-widget`)

Branch: `fix/lock-widget-memory`. Inputs: `AI/architect_spec_lock_widget_memory.md`,
`AI/plan_evaluator_report.md` (APPROVED), `AI/tester_contract_lock_widget_memory.md`, red run of
`lock_widget_buffer_tests` (4 of 7 failed on assertions).

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| 1 | No `.unwrap()`, `.expect(`, panic/todo/unreachable/print macro, `dbg!` in production code; no slice indexing (use `get_mut`). | `crates/lock-widget/src/{layout,wayland}.rs` | `test_lka12_no_panic_or_print_macro_in_production_code`; clippy `-D warnings` |
| 2 | Size arithmetic is checked: `checked_next_multiple_of`, `checked_mul`, `try_from`; an overflow is `None`, never a wrap. | `layout.rs::{slot_len, surface_shm_bytes}` | `test_lka18_slot_len_*`, `test_lka18_shared_memory_bytes_per_surface` |
| 3 | At most `MAX_BUFFERS_PER_SURFACE` buffers per surface: the only push into the buffer list is gated by `needs_extra_buffer`. | `wayland.rs::draw_all` | review; `test_lka18_*` |
| 4 | A failed allocation leaves no partial state: a new buffer lives in its own `SlotPool` of exactly `slot_len` bytes; `SlotPool::resize` (which corrupts the pool length on failure in sctk 0.20.0) is never called and no pool ever grows. | `wayland.rs::new_shm_buffer` | review; `grep -n "resize" crates/lock-widget/src/wayland.rs` empty |
| 5 | A failed allocation is a busy buffer: `retry = true`, no stderr write, nothing about the lock state logged; the only stderr lines stay the existing once-per-run output and skip lines. | `wayland.rs::draw_all` | `test_lka12_stderr_is_only_written_by_the_binaries`; review |
| 6 | A newly allocated buffer starts with `painted = false`, so its first use is `paint_full` with full-surface damage (identical pixels to the eager code). | `wayland.rs::draw_all` | review |
| 7 | Budgets, `buffer_len`, damage, commit, timer, retry, seat-less binding, layer and exit codes unchanged. | `layout.rs`, `wayland.rs` | `test_lka9_*`, `test_lka12_widget_binds_no_seat_and_draws_a_background_layer`, `test_lka13_*` |
| 8 | Buffers are dropped before their pool (field order), as the eager code did (`buffers.clear()` before `pool = None`). | `wayland.rs::ShmBuffer` | review |
| 9 | `#![forbid(unsafe_code)]` kept; no new dependency; `Cargo.lock` unchanged. | crate | `test_lka12_both_crate_roots_forbid_unsafe`; `git diff --exit-code origin/main -- Cargo.lock crates/lock-widget/Cargo.toml` |
| 10 | Test integrity: no existing test modified; the stubs' "TDD Red stub" doc lines are replaced by real docs. | all tests | `git diff --name-only origin/main -- 'crates/*/tests' tests/` lists only the new file |

VERDICT: CLEARED
