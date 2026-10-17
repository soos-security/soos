//! Contract tests of the lazy shared-memory buffer policy of the widget (GitHub #412, follow-up of
//! #403; matrix LKA18): one buffer per surface, a second only while the compositor still holds
//! every allocated one, never more than two, and the shared-memory sizes the documentation quotes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_lock_widget::layout::{
    buffer_len, needs_extra_buffer, slot_len, surface_shm_bytes, BUFFERS_PER_SURFACE,
    MAX_BUFFERS_PER_SURFACE, MAX_SURFACE_PIXELS, MAX_TOTAL_PIXELS, SHM_SLOT_ALIGN,
};

const MIB: usize = 1024 * 1024;

/// A surface at the per-surface pixel budget (`MAX_SURFACE_PIXELS`).
const BUDGET_W: u32 = 8192;
const BUDGET_H: u32 = 4096;

#[test]
fn test_lka18_at_most_two_buffers_per_surface() {
    assert_eq!(MAX_BUFFERS_PER_SURFACE, 2);
    assert_eq!(BUFFERS_PER_SURFACE, MAX_BUFFERS_PER_SURFACE);
    for any_free in [false, true] {
        assert!(!needs_extra_buffer(2, any_free));
        assert!(!needs_extra_buffer(3, any_free));
        assert!(!needs_extra_buffer(usize::MAX, any_free));
    }
}

#[test]
fn test_lka18_no_extra_buffer_while_one_is_free() {
    assert!(!needs_extra_buffer(1, true));
    assert!(!needs_extra_buffer(0, true));
}

#[test]
fn test_lka18_extra_buffer_only_when_every_buffer_is_busy() {
    assert!(needs_extra_buffer(1, false));
    assert!(needs_extra_buffer(0, false));
    assert!(!needs_extra_buffer(MAX_BUFFERS_PER_SURFACE, false));
}

/// How the compositor treats a committed shared-memory buffer in [`replay`].
#[derive(Clone, Copy)]
enum Compositor {
    /// Releases the buffer before the next frame (it read the pixels on commit).
    ReleasesPromptly,
    /// Holds the committed buffer until another one is committed.
    HoldsUntilReplaced,
    /// Never releases anything.
    NeverReleases,
}

/// Replays `frames` frame ticks of the widget's buffer selection (reuse a free buffer, otherwise
/// allocate when the policy allows, otherwise retry) and returns the buffers allocated and the
/// frames that drew.
fn replay(compositor: Compositor, frames: usize) -> (usize, usize) {
    // `true` = held by the compositor. One buffer exists after configure.
    let mut held = vec![false];
    let mut drawn = 0;
    for _ in 0..frames {
        let mut free = held.iter().position(|busy| !busy);
        if needs_extra_buffer(held.len(), free.is_some()) {
            held.push(false);
            free = Some(held.len() - 1);
        }
        let Some(index) = free else {
            continue; // retry on the next tick
        };
        drawn += 1;
        match compositor {
            Compositor::ReleasesPromptly => {}
            Compositor::HoldsUntilReplaced => {
                held.fill(false);
                held[index] = true;
            }
            Compositor::NeverReleases => held[index] = true,
        }
        assert!(held.len() <= MAX_BUFFERS_PER_SURFACE);
    }
    (held.len(), drawn)
}

#[test]
fn test_lka18_replay_keeps_one_buffer_when_the_compositor_releases_promptly() {
    assert_eq!(replay(Compositor::ReleasesPromptly, 300), (1, 300));
}

#[test]
fn test_lka18_replay_allocates_the_second_buffer_only_for_a_holding_compositor() {
    assert_eq!(replay(Compositor::HoldsUntilReplaced, 300), (2, 300));
    // Two frames draw, then every tick retries without a third buffer.
    assert_eq!(replay(Compositor::NeverReleases, 300), (2, 2));
}

#[test]
fn test_lka18_slot_len_is_buffer_len_rounded_to_the_slot_alignment() {
    assert_eq!(SHM_SLOT_ALIGN, 64);
    assert_eq!(slot_len(1, 1), Some(64));
    assert_eq!(slot_len(4, 4), Some(64));
    assert_eq!(slot_len(5, 4), Some(128));
    for (w, h) in [
        (1, 1),
        (3, 1),
        (1365, 767),
        (1366, 768),
        (1920, 1080),
        (7680, 4320),
        (BUDGET_W, BUDGET_H),
    ] {
        let buffer = buffer_len(w, h).unwrap();
        let slot = slot_len(w, h).unwrap();
        assert_eq!(slot % SHM_SLOT_ALIGN, 0, "{w}x{h}");
        assert!(slot >= buffer && slot - buffer < SHM_SLOT_ALIGN, "{w}x{h}");
    }
    for (w, h) in [
        (0, 1),
        (1, 0),
        (8193, 1),
        (8192, 8192),
        (u32::MAX, u32::MAX),
    ] {
        assert_eq!(slot_len(w, h), None, "{w}x{h}");
    }
}

#[test]
fn test_lka18_shared_memory_bytes_per_surface() {
    assert_eq!(
        u64::from(BUDGET_W) * u64::from(BUDGET_H),
        MAX_SURFACE_PIXELS
    );
    // 128 MiB per budget-size (8K-class) buffer; 256 MiB for one surface holding two.
    assert_eq!(surface_shm_bytes(BUDGET_W, BUDGET_H, 1), Some(128 * MIB));
    assert_eq!(surface_shm_bytes(BUDGET_W, BUDGET_H, 2), Some(256 * MIB));
    // Two budget-size outputs (the total budget) with one buffer each: 256 MiB.
    let lazy = 2 * surface_shm_bytes(BUDGET_W, BUDGET_H, 1).unwrap();
    assert_eq!(lazy, 256 * MIB);
    assert_eq!(u64::try_from(lazy).unwrap(), MAX_TOTAL_PIXELS * 4);
    // The former eager allocation (two buffers on each): 512 MiB, now only on holding compositors.
    assert_eq!(
        2 * surface_shm_bytes(BUDGET_W, BUDGET_H, 2).unwrap(),
        512 * MIB
    );
    // 8K UHD and full HD, exact bytes.
    assert_eq!(surface_shm_bytes(7680, 4320, 1), Some(132_710_400));
    assert_eq!(surface_shm_bytes(7680, 4320, 2), Some(265_420_800));
    assert_eq!(surface_shm_bytes(1920, 1080, 1), Some(8_294_400));
    // Out of the policy or the budgets.
    assert_eq!(surface_shm_bytes(1920, 1080, 0), None);
    assert_eq!(surface_shm_bytes(1920, 1080, 3), None);
    assert_eq!(surface_shm_bytes(0, 1080, 1), None);
    assert_eq!(surface_shm_bytes(8192, 8192, 1), None);
}
