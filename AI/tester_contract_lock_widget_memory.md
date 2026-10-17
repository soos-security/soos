# Tester Contract — GitHub #412: lazy second shared-memory buffer of `soos-lock-widget`

- **Date**: 2026-10-17 — **Branch**: `fix/lock-widget-memory`
- **Spec**: `AI/architect_spec_lock_widget_memory.md` (plan evaluation APPROVED, `AI/plan_evaluator_report.md`).
- All tests are new, in a new file; red was proven against signature stubs in `crates/lock-widget/src/layout.rs`
  (`slot_len` and `surface_shm_bytes` returned `None`, `needs_extra_buffer` returned `false`); the stubs compile,
  so every failure is an assertion failure, not a missing item.

| Test file | Tests | Matrix | Red evidence |
|---|---|---|---|
| `crates/lock-widget/tests/lock_widget_buffer_tests.rs` | `test_lka18_at_most_two_buffers_per_surface`, `test_lka18_no_extra_buffer_while_one_is_free`, `test_lka18_extra_buffer_only_when_every_buffer_is_busy`, `test_lka18_replay_keeps_one_buffer_when_the_compositor_releases_promptly`, `test_lka18_replay_allocates_the_second_buffer_only_for_a_holding_compositor`, `test_lka18_slot_len_is_buffer_len_rounded_to_the_slot_alignment`, `test_lka18_shared_memory_bytes_per_surface` | LKA18 | 4 of 7 failed (`cargo test -p soos-lock-widget --locked --test lock_widget_buffer_tests`); the 3 passing ones are guards that an eager implementation (`needs_extra_buffer` always `true`) would break |

### What the tests pin

- Never more than `MAX_BUFFERS_PER_SURFACE` (2) buffers; `BUFFERS_PER_SURFACE` stays equal to it (the LKA9
  contract still asserts 2).
- No second buffer while one is free; a second one only when every allocated one is busy.
- A replay of the widget's selection loop against three compositor models: prompt release keeps one buffer and
  draws every frame; a compositor holding the committed buffer until it is replaced settles at two buffers and
  draws every frame; a compositor that never releases stops at two buffers and the rest of the ticks retry.
- `slot_len` is `buffer_len` rounded up to 64 bytes (smithay-client-toolkit slot alignment), `None` exactly when
  `buffer_len` is.
- Sizes quoted by the documentation: 128 MiB per budget-size buffer (8192 × 4096 = `MAX_SURFACE_PIXELS`),
  256 MiB for two budget-size outputs with one buffer each (= `MAX_TOTAL_PIXELS` × 4), 512 MiB for the former
  eager allocation, exact bytes for 7680 × 4320 and 1920 × 1080; `None` for 0 or 3 buffers and invalid sizes.

### Migrated existing tests

None. No existing test file is modified. `test_lka9_memory_bounds` (which imports and asserts
`BUFFERS_PER_SURFACE == 2`) is the reason the constant is kept as an alias.

### Not testable in CI

The Wayland loop (`src/wayland.rs`) has no CI compositor; matrix LKA19 is the owner hardware check.
