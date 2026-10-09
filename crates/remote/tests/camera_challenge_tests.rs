//! Contract test of the `CameraView` challenge pools of `soos-remote` (GitHub #345, ADR
//! 2026-10-07 "Live Camera View in `soos-remote` Through the Daemon Preview Channel",
//! architect spec `AI/architect_spec_remote_live_camera.md` §1.1, S-5, test 40, matrix RLC6).
//!
//! The spec allows a new `challenge_tests.rs`-style file for test 40; this file keeps the
//! existing `auth_store_tests.rs` compiling while `ChallengePurpose::CameraView` does not
//! exist yet. Pure: every time is a `tokio::time::Instant` offset from one origin.

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

use soos_remote::challenge::{
    ChallengeBinding, ChallengeError, ChallengePurpose, ChallengeStore, Taken,
};
use soos_remote::identity::{client_hint, PathClass};
use soos_remote::{CHALLENGE_BYTES, CHALLENGE_TTL_MS, MAX_PENDING_CHALLENGES};

use ChallengePurpose::{CameraView, Unlock};
use PathClass::{Funnel, Tailnet};

fn bytes(n: u32) -> [u8; CHALLENGE_BYTES] {
    let mut out = [0u8; CHALLENGE_BYTES];
    out[..4].copy_from_slice(&n.to_be_bytes());
    out[31] = 0x3c;
    out
}

fn session(n: u8) -> ChallengeBinding {
    ChallengeBinding::WebSession([n; 32])
}

/// Test 40 (RLC6, RLC-S3): the pools `(Tailnet, CameraView, None)` and `(Funnel,
/// CameraView, WebSession)` hold `MAX_PENDING_CHALLENGES` (4) each and refuse the 5th,
/// separately from the unlock pools; any other binding variant is `Mismatch`; a
/// `CameraView` challenge taken as `Unlock` (or the reverse) is `Mismatch` and removed;
/// the Funnel session hash must match; expiry applies.
#[test]
fn test_rlc_camera_view_challenge_pool() {
    assert_eq!(MAX_PENDING_CHALLENGES, 4);
    let t0 = Instant::now();
    let mut store = ChallengeStore::new();

    // Tailnet camera pool: 4 accepted, 5th refused.
    for i in 0..4 {
        assert_eq!(
            store.issue(
                t0,
                Tailnet,
                CameraView,
                ChallengeBinding::None,
                None,
                bytes(i)
            ),
            Ok(0),
            "tailnet camera {i}"
        );
    }
    assert_eq!(
        store.issue(
            t0,
            Tailnet,
            CameraView,
            ChallengeBinding::None,
            None,
            bytes(4)
        ),
        Err(ChallengeError::PoolFull)
    );
    assert_eq!(store.pending(Tailnet, CameraView), 4);

    // Funnel camera pool: 4 accepted, 5th refused.
    for i in 10..14 {
        assert_eq!(
            store.issue(t0, Funnel, CameraView, session(1), None, bytes(i)),
            Ok(0),
            "funnel camera {i}"
        );
    }
    assert_eq!(
        store.issue(t0, Funnel, CameraView, session(2), None, bytes(14)),
        Err(ChallengeError::PoolFull)
    );
    assert_eq!(store.pending(Funnel, CameraView), 4);

    // The unlock pools are separate and still have room.
    for i in 20..24 {
        store
            .issue(t0, Tailnet, Unlock, ChallengeBinding::None, None, bytes(i))
            .expect("tailnet unlock pool is not shared with the camera pool");
    }
    for i in 30..34 {
        store
            .issue(t0, Funnel, Unlock, session(1), None, bytes(i))
            .expect("funnel unlock pool is not shared with the camera pool");
    }

    // Wrong binding variants for the camera purpose.
    let mut fresh = ChallengeStore::new();
    for (class, binding) in [
        (Tailnet, session(1)),
        (Tailnet, ChallengeBinding::EnrollCode([3; 32])),
        (
            Tailnet,
            ChallengeBinding::Login(client_hint(&[("x-forwarded-for", &b"198.51.100.7"[..])])),
        ),
        (Funnel, ChallengeBinding::None),
        (Funnel, ChallengeBinding::EnrollCode([3; 32])),
        (
            Funnel,
            ChallengeBinding::Login(client_hint(&[("x-forwarded-for", &b"198.51.100.7"[..])])),
        ),
    ] {
        assert_eq!(
            fresh.issue(t0, class, CameraView, binding.clone(), None, bytes(40)),
            Err(ChallengeError::Mismatch),
            "{class:?} {binding:?}"
        );
    }
    assert_eq!(fresh.pending(Tailnet, CameraView), 0);
    assert_eq!(fresh.pending(Funnel, CameraView), 0);

    // Purpose confusion: a CameraView challenge taken as Unlock is Mismatch and removed.
    assert_eq!(
        store.take(t0, Tailnet, Unlock, &ChallengeBinding::None, &bytes(0)),
        Err(ChallengeError::Mismatch)
    );
    assert_eq!(store.pending(Tailnet, CameraView), 3, "entry removed");
    assert_eq!(
        store.take(t0, Tailnet, CameraView, &ChallengeBinding::None, &bytes(0)),
        Err(ChallengeError::Unknown),
        "single use even after a mismatch"
    );
    // The reverse: an Unlock challenge taken as CameraView.
    assert_eq!(
        store.take(t0, Tailnet, CameraView, &ChallengeBinding::None, &bytes(20)),
        Err(ChallengeError::Mismatch)
    );
    assert_eq!(store.pending(Tailnet, Unlock), 3);

    // Nominal take.
    assert_eq!(
        store.take(t0, Tailnet, CameraView, &ChallengeBinding::None, &bytes(1)),
        Ok(Taken::Plain)
    );
    // Funnel: another session's hash is Mismatch (and removes the entry); the right one works.
    assert_eq!(
        store.take(t0, Funnel, CameraView, &session(2), &bytes(10)),
        Err(ChallengeError::Mismatch)
    );
    assert_eq!(
        store.take(t0, Funnel, CameraView, &session(1), &bytes(11)),
        Ok(Taken::Plain)
    );
    // Class confusion: a Funnel camera challenge presented on the tailnet.
    assert_eq!(
        store.take(t0, Tailnet, CameraView, &ChallengeBinding::None, &bytes(12)),
        Err(ChallengeError::Mismatch)
    );
    // A pool slot freed by a take can be reused.
    store
        .issue(
            t0,
            Tailnet,
            CameraView,
            ChallengeBinding::None,
            None,
            bytes(50),
        )
        .expect("room after takes");

    // Expiry: a camera challenge is valid strictly less than CHALLENGE_TTL_MS.
    let mut timed = ChallengeStore::new();
    timed
        .issue(
            t0,
            Tailnet,
            CameraView,
            ChallengeBinding::None,
            None,
            bytes(60),
        )
        .unwrap();
    assert_eq!(
        timed.take(
            t0 + Duration::from_millis(CHALLENGE_TTL_MS),
            Tailnet,
            CameraView,
            &ChallengeBinding::None,
            &bytes(60)
        ),
        Err(ChallengeError::Expired)
    );
    // A CameraView challenge never carries a pending registration.
    assert_eq!(
        timed.issue(
            t0,
            Tailnet,
            CameraView,
            ChallengeBinding::None,
            Some(soos_remote::challenge::PendingRegistration {
                user_handle: [0; soos_remote::USER_HANDLE_BYTES]
            }),
            bytes(61)
        ),
        Err(ChallengeError::Mismatch)
    );
}
