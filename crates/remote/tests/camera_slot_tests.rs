//! Contract tests of the pure view slot and stream token of the live camera view of
//! `soos-remote` (GitHub #345, ADR 2026-10-07 "Live Camera View in `soos-remote` Through the
//! Daemon Preview Channel", architect spec `AI/architect_spec_remote_live_camera.md` §7,
//! tests 10–16, matrix RLC8, RLC9, RLC14).
//!
//! Pure: every time is a `tokio::time::Instant` built by adding durations to one origin, so
//! the token TTL is exact without any clock. Migration M13 (owner request 2026-10-07): there
//! is no cooldown between views; a new reservation right after any view ends succeeds.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::time::Duration;

use tokio::time::Instant;

use soos_remote::camera_slot::{SlotError, SlotPhase, ViewOwner, ViewSlot, ViewToken};
use soos_remote::identity::PathClass;
use soos_remote::{CAMERA_VIEW_TOKEN_B64_LEN, CAMERA_VIEW_TOKEN_BYTES, CAMERA_VIEW_TOKEN_TTL_MS};

const MAX_VIEW: Duration = Duration::from_secs(120);

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn token_bytes(n: u8) -> [u8; CAMERA_VIEW_TOKEN_BYTES] {
    let mut out = [0u8; CAMERA_VIEW_TOKEN_BYTES];
    for (i, b) in out.iter_mut().enumerate() {
        *b = n.wrapping_mul(31).wrapping_add(i as u8);
    }
    out
}

fn token(n: u8) -> ViewToken {
    ViewToken::from_bytes(token_bytes(n))
}

fn tailnet() -> ViewOwner {
    ViewOwner {
        class: PathClass::Tailnet,
        session: None,
    }
}

fn funnel(session: u8) -> ViewOwner {
    ViewOwner {
        class: PathClass::Funnel,
        session: Some([session; 32]),
    }
}

fn phase(slot: &ViewSlot, now: Instant) -> SlotPhase {
    slot.phase(now)
}

// ---------------------------------------------------------------------------------------
// Test 10
// ---------------------------------------------------------------------------------------

/// Test 10 (RLC9): Idle → reserve → Pending → begin → Starting → mark_streaming →
/// Streaming → end → Idle, with `ends_at = begin time + max_view`; M13: a new reservation
/// at the very instant the view ended succeeds (no cooldown).
#[test]
fn test_rlc_slot_reserve_begin_stream_end_cycle() {
    let t0 = Instant::now();
    let mut slot = ViewSlot::new();
    assert_eq!(slot.phase(t0), SlotPhase::Idle);
    assert_eq!(slot.check(t0), Ok(()));

    slot.reserve(t0, tailnet(), token(1))
        .expect("idle slot reserves");
    assert_eq!(phase(&slot, t0), SlotPhase::Pending);

    let t1 = t0 + ms(1_500);
    let ticket = slot
        .begin(t1, tailnet(), &token(1), MAX_VIEW)
        .expect("matching token and owner begin the view");
    assert_eq!(ticket.ends_at, t1 + MAX_VIEW);
    assert_eq!(phase(&slot, t1), SlotPhase::Starting);
    assert!(!slot.stop_requested(ticket.view));

    slot.mark_streaming(ticket.view);
    assert_eq!(phase(&slot, t1), SlotPhase::Streaming);
    assert!(!slot.stop_requested(ticket.view));

    let t2 = t1 + ms(30_000);
    slot.end(ticket.view);
    assert_eq!(slot.phase(t2), SlotPhase::Idle);
    assert_eq!(slot.check(t2), Ok(()), "no cooldown after a shown view");

    // A second cycle at the same instant gets a different view id.
    slot.reserve(t2, tailnet(), token(2))
        .expect("immediate reserve after a view ended");
    let second = slot.begin(t2, tailnet(), &token(2), MAX_VIEW).unwrap();
    assert_ne!(second.view, ticket.view, "view ids are never reused");
}

// ---------------------------------------------------------------------------------------
// Test 11
// ---------------------------------------------------------------------------------------

/// Test 11 (RLC9, RLC-S5): one global view: `check` and `reserve` answer `Busy` while a view
/// is Pending, Starting or Streaming, whatever the caller.
#[test]
fn test_rlc_slot_single_global_view() {
    let t0 = Instant::now();
    let mut slot = ViewSlot::new();
    slot.reserve(t0, tailnet(), token(1)).unwrap();
    for owner in [tailnet(), funnel(7)] {
        assert_eq!(slot.check(t0), Err(SlotError::Busy), "pending");
        assert_eq!(slot.reserve(t0, owner, token(9)), Err(SlotError::Busy));
    }
    let ticket = slot.begin(t0, tailnet(), &token(1), MAX_VIEW).unwrap();
    assert_eq!(slot.check(t0), Err(SlotError::Busy), "starting");
    assert_eq!(slot.reserve(t0, funnel(7), token(9)), Err(SlotError::Busy));
    slot.mark_streaming(ticket.view);
    assert_eq!(
        slot.check(t0 + ms(60_000)),
        Err(SlotError::Busy),
        "streaming"
    );
    assert_eq!(
        slot.reserve(t0 + ms(60_000), tailnet(), token(9)),
        Err(SlotError::Busy)
    );
    // The busy reservation never replaced the running view.
    assert_eq!(phase(&slot, t0), SlotPhase::Streaming);
    assert!(!slot.stop_requested(ticket.view));
}

// ---------------------------------------------------------------------------------------
// Test 12
// ---------------------------------------------------------------------------------------

/// Test 12 (RLC8, RLC-S4): a token is single use and valid strictly less than
/// `CAMERA_VIEW_TOKEN_TTL_MS`; an expired Pending reservation returns the slot to Idle.
#[test]
fn test_rlc_slot_token_single_use_and_ttl() {
    let t0 = Instant::now();
    let mut slot = ViewSlot::new();
    slot.reserve(t0, tailnet(), token(1)).unwrap();
    let ticket = slot
        .begin(
            t0 + ms(CAMERA_VIEW_TOKEN_TTL_MS - 1),
            tailnet(),
            &token(1),
            MAX_VIEW,
        )
        .expect("just before expiry");
    // Second use of the same token: rejected, the running view is untouched.
    assert_eq!(
        slot.begin(t0 + ms(2), tailnet(), &token(1), MAX_VIEW),
        Err(SlotError::TokenRejected)
    );
    assert_eq!(phase(&slot, t0 + ms(2)), SlotPhase::Starting);
    slot.end(ticket.view);

    // Expiry: `begin` at exactly `expires` is rejected and the slot is Idle.
    let t1 = t0 + ms(100_000);
    slot.reserve(t1, tailnet(), token(2)).unwrap();
    let at_expiry = t1 + ms(CAMERA_VIEW_TOKEN_TTL_MS);
    assert_eq!(
        slot.begin(at_expiry, tailnet(), &token(2), MAX_VIEW),
        Err(SlotError::TokenRejected)
    );
    assert_eq!(slot.phase(at_expiry), SlotPhase::Idle);
    assert_eq!(slot.check(at_expiry), Ok(()));
    // The expired reservation also frees the slot for `check`/`reserve` without a `begin`.
    let t2 = t1 + ms(200_000);
    slot.reserve(t2, tailnet(), token(3)).unwrap();
    let later = t2 + ms(CAMERA_VIEW_TOKEN_TTL_MS);
    assert_eq!(slot.check(later), Ok(()));
    slot.reserve(later, tailnet(), token(4))
        .expect("expired pending is purged");
    assert_eq!(
        slot.begin(later, tailnet(), &token(3), MAX_VIEW),
        Err(SlotError::TokenRejected),
        "the purged token is gone"
    );
    // `begin` on an Idle slot is rejected.
    let mut idle = ViewSlot::new();
    assert_eq!(
        idle.begin(t0, tailnet(), &token(1), MAX_VIEW),
        Err(SlotError::TokenRejected)
    );
}

// ---------------------------------------------------------------------------------------
// Test 13
// ---------------------------------------------------------------------------------------

/// Test 13 (RLC8): a token is bound to its owner: a different path class, a different
/// Funnel session hash or a different token is rejected and the Pending reservation is
/// kept for its owner, who then succeeds.
#[test]
fn test_rlc_slot_token_bound_to_owner() {
    let t0 = Instant::now();

    // Funnel owner, session A.
    let mut slot = ViewSlot::new();
    slot.reserve(t0, funnel(0xAA), token(1)).unwrap();
    for (owner, presented) in [
        (tailnet(), token(1)),
        (funnel(0xBB), token(1)),
        (
            ViewOwner {
                class: PathClass::Funnel,
                session: None,
            },
            token(1),
        ),
        (funnel(0xAA), token(2)),
    ] {
        assert_eq!(
            slot.begin(t0, owner, &presented, MAX_VIEW),
            Err(SlotError::TokenRejected)
        );
        assert_eq!(
            phase(&slot, t0),
            SlotPhase::Pending,
            "a stray request never cancels the owner's reservation"
        );
    }
    slot.begin(t0 + ms(1), funnel(0xAA), &token(1), MAX_VIEW)
        .expect("the owner still succeeds");

    // Tailnet owner: a Funnel request with the right token is rejected.
    let mut slot = ViewSlot::new();
    slot.reserve(t0, tailnet(), token(3)).unwrap();
    assert_eq!(
        slot.begin(t0, funnel(0xAA), &token(3), MAX_VIEW),
        Err(SlotError::TokenRejected)
    );
    assert_eq!(phase(&slot, t0), SlotPhase::Pending);
    slot.begin(t0, tailnet(), &token(3), MAX_VIEW).unwrap();
}

// ---------------------------------------------------------------------------------------
// Test 14
// ---------------------------------------------------------------------------------------

/// Test 14 (RLC9, migration M13): there is no cooldown: after a view that showed pixels,
/// and after one that never did, the slot is Idle at the very instant it ended and accepts a
/// new reservation immediately; `end` with a stale view id is a no-op.
#[test]
fn test_rlc_slot_no_cooldown_after_any_view() {
    let t0 = Instant::now();
    let mut slot = ViewSlot::new();
    slot.reserve(t0, tailnet(), token(1)).unwrap();
    let ticket = slot.begin(t0, tailnet(), &token(1), MAX_VIEW).unwrap();
    slot.mark_streaming(ticket.view);

    // Stale id: no-op.
    slot.end(ticket.view.wrapping_add(1));
    assert_eq!(phase(&slot, t0), SlotPhase::Streaming);

    // A view that showed pixels (it reached Streaming): no cooldown.
    let t1 = t0 + ms(5_000);
    slot.end(ticket.view);
    assert_eq!(slot.phase(t1), SlotPhase::Idle);
    assert_eq!(slot.check(t1), Ok(()));
    slot.reserve(t1, tailnet(), token(2))
        .expect("immediate reserve after a shown view");
    let again = slot.begin(t1, tailnet(), &token(2), MAX_VIEW).unwrap();
    assert_ne!(again.view, ticket.view);
    slot.mark_streaming(again.view);
    assert!(slot.stop());
    slot.end(again.view);
    assert_eq!(slot.check(t1), Ok(()), "a stopped view leaves no cooldown");

    // A view that never showed pixels (first frame failed): no cooldown either.
    slot.reserve(t1, tailnet(), token(3)).unwrap();
    let failed = slot.begin(t1, tailnet(), &token(3), MAX_VIEW).unwrap();
    slot.end(failed.view);
    assert_eq!(slot.phase(t1), SlotPhase::Idle);
    slot.reserve(t1, tailnet(), token(4))
        .expect("immediate reserve after an unshown view");

    // A second `end` of an already ended view is a no-op as well.
    let pending_phase = phase(&slot, t1);
    slot.end(failed.view);
    assert_eq!(phase(&slot, t1), pending_phase);
    assert_eq!(pending_phase, SlotPhase::Pending);

    // The phase set has no cooldown variant: this exhaustive match only compiles while
    // `SlotPhase` is exactly Idle, Pending, Starting and Streaming.
    for later in [
        t1 + ms(1),
        t1 + ms(CAMERA_VIEW_TOKEN_TTL_MS),
        t1 + ms(1_000_000),
    ] {
        match slot.phase(later) {
            SlotPhase::Idle | SlotPhase::Pending | SlotPhase::Starting | SlotPhase::Streaming => {}
        }
        // Likewise `SlotError` is exactly Busy and TokenRejected (no cooldown refusal).
        match slot.check(later) {
            Ok(()) | Err(SlotError::Busy | SlotError::TokenRejected) => {}
        }
    }
}

// ---------------------------------------------------------------------------------------
// Test 15
// ---------------------------------------------------------------------------------------

/// Test 15 (RLC14, RLC-S10): `stop` cancels a Pending reservation (Idle), sets
/// a durable flag on a Starting/Streaming view (kept until `end`), and is `false` on an Idle
/// slot; `stop_requested` is also true for a view the slot no longer holds (fail closed).
#[test]
fn test_rlc_slot_stop() {
    let t0 = Instant::now();
    let mut slot = ViewSlot::new();
    assert!(!slot.stop(), "nothing to stop");

    slot.reserve(t0, tailnet(), token(1)).unwrap();
    assert!(slot.stop());
    assert_eq!(slot.phase(t0), SlotPhase::Idle);
    assert_eq!(slot.check(t0), Ok(()));
    assert_eq!(
        slot.begin(t0, tailnet(), &token(1), MAX_VIEW),
        Err(SlotError::TokenRejected),
        "a stopped reservation's token is dead"
    );

    // Stop during Starting: durable across mark_streaming.
    slot.reserve(t0, tailnet(), token(2)).unwrap();
    let ticket = slot.begin(t0, tailnet(), &token(2), MAX_VIEW).unwrap();
    assert!(!slot.stop_requested(ticket.view));
    assert!(slot.stop());
    assert!(slot.stop_requested(ticket.view));
    slot.mark_streaming(ticket.view);
    assert!(
        slot.stop_requested(ticket.view),
        "the stop flag is never cleared before end"
    );
    assert!(slot.stop(), "a second stop is still acknowledged");
    assert!(slot.stop_requested(ticket.view));
    assert_eq!(
        phase(&slot, t0),
        SlotPhase::Streaming,
        "stop never frees the slot itself"
    );

    // A view id the slot does not hold reads as stopped (fail closed).
    assert!(slot.stop_requested(ticket.view.wrapping_add(1)));
    slot.end(ticket.view);
    assert!(
        slot.stop_requested(ticket.view),
        "ended view reads as stopped"
    );
    assert!(!slot.stop(), "Idle has nothing to stop");

    // Stop during Streaming without an earlier stop, right after the previous view (M13).
    let t1 = t0;
    slot.reserve(t1, tailnet(), token(3)).unwrap();
    let ticket = slot.begin(t1, tailnet(), &token(3), MAX_VIEW).unwrap();
    slot.mark_streaming(ticket.view);
    assert!(!slot.stop_requested(ticket.view));
    assert!(slot.stop());
    assert!(slot.stop_requested(ticket.view));
}

// ---------------------------------------------------------------------------------------
// Test 16
// ---------------------------------------------------------------------------------------

fn is_b64url(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// Test 16 (RLC8, RLC-S4): `encode` is exactly 43 unpadded base64url characters; `parse` is
/// strict (length, alphabet, canonical last character) and round-trips; `Debug` of a token
/// and of an owner is redacted.
#[test]
fn test_rlc_view_token_format_and_redaction() {
    assert_eq!(CAMERA_VIEW_TOKEN_BYTES, 32);
    assert_eq!(CAMERA_VIEW_TOKEN_B64_LEN, 43);
    for n in [0u8, 1, 2, 77, 255] {
        let t = token(n);
        let text = t.encode();
        assert_eq!(text.len(), CAMERA_VIEW_TOKEN_B64_LEN);
        assert!(text.chars().all(is_b64url), "{}", text.as_str());
        let back = ViewToken::parse(&text).expect("own encoding parses");
        assert_eq!(back.encode().as_str(), text.as_str(), "round trip");
    }
    // All-zero and all-0xFF tokens encode to the expected text.
    let zero = ViewToken::from_bytes([0u8; 32]);
    assert_eq!(zero.encode().as_str(), "A".repeat(43));
    let ones = ViewToken::from_bytes([0xFF; 32]);
    assert_eq!(ones.encode().as_str(), format!("{}8", "_".repeat(42)));

    let good = token(5).encode();
    let good = good.as_str();
    // Wrong lengths.
    assert!(ViewToken::parse(&good[..42]).is_none(), "42 chars");
    assert!(ViewToken::parse(&format!("{good}A")).is_none(), "44 chars");
    assert!(ViewToken::parse("").is_none());
    assert!(ViewToken::parse(&format!("{good}=")).is_none(), "padding");
    // Characters outside the base64url alphabet.
    for bad in ['=', '+', '/', '.', ' ', '%', 'é'] {
        let mut text: String = good.chars().take(20).collect();
        text.push(bad);
        text.extend(good.chars().skip(21));
        assert!(ViewToken::parse(&text).is_none(), "{bad:?}");
    }
    // Non-canonical last character: 43 chars carry 258 bits, the last 2 must be zero.
    let zero_text = "A".repeat(43);
    assert!(ViewToken::parse(&zero_text).is_some());
    for last in ['B', 'C', 'D'] {
        let text = format!("{}{last}", "A".repeat(42));
        assert!(
            ViewToken::parse(&text).is_none(),
            "non-canonical trailing bits {last}"
        );
    }
    assert!(
        ViewToken::parse(&format!("{}E", "A".repeat(42))).is_some(),
        "E is canonical"
    );

    // Debug redaction: no run of the encoded token appears.
    let t = token(9);
    let text = t.encode();
    let debug = format!("{t:?}");
    assert!(debug.contains("redacted"), "{debug}");
    for start in 0..=(text.len() - 8) {
        assert!(
            !debug.contains(&text[start..start + 8]),
            "Debug leaks token characters: {debug}"
        );
    }
    let owner = funnel(0x5A);
    let debug = format!("{owner:?}");
    assert!(debug.contains("redacted"), "{debug}");
    assert!(
        !debug.contains("90"),
        "no session byte (0x5A = 90) in {debug}"
    );
    assert!(!debug.to_ascii_lowercase().contains("5a"), "{debug}");
}
