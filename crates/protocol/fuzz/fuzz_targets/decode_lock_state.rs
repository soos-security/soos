#![no_main]

use libfuzzer_sys::fuzz_target;
use soos_protocol::types::{
    LockStateResponse, LockWidgetState, Response, LOCK_STATE_RESPONSE_MAX_PAYLOAD,
};
use soos_protocol::{decode, encode};

// GitHub #380 (PR 1 of #352): the lock-state decoder must never panic, a decoded answer is
// one of the three collapsed states within the 12-byte bound, it round-trips, and no frame
// is ever both a `LockStateResponse` and a `Response` (answer vs refusal are unambiguous).
fuzz_target!(|data: &[u8]| {
    if let Ok(resp) = decode::<LockStateResponse>(data) {
        assert!(matches!(
            resp.state,
            LockWidgetState::Idle | LockWidgetState::Scanning | LockWidgetState::NotRecognized
        ));
        assert!(decode::<Response>(data).is_err());
        if let Ok(frame) = encode(&resp) {
            assert!(frame.len() - 4 <= LOCK_STATE_RESPONSE_MAX_PAYLOAD);
            assert_eq!(decode::<LockStateResponse>(&frame).ok(), Some(resp));
        }
    }
});
