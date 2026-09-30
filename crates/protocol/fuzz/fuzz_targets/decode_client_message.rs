#![no_main]

//! Fuzz target for the client message discriminator (GitHub #204 / DMN-15).
//!
//! Arbitrary payloads must never panic, a tagged classification always comes from a
//! reserved trailer byte, and a legacy classification never does. Exact single-type
//! matching of legacy payloads is asserted by the proptest suite
//! (`crates/protocol/tests/client_message_tests.rs`).

use libfuzzer_sys::fuzz_target;
use soos_protocol::message::{decode_client_message, FrameFormat, MESSAGE_TAG_MIN};

fuzz_target!(|data: &[u8]| {
    if let Ok((_message, format)) = decode_client_message(data) {
        let last = data.last().copied().unwrap_or(0);
        match format {
            FrameFormat::Tagged => assert!(last >= MESSAGE_TAG_MIN),
            FrameFormat::Legacy => assert!(last < MESSAGE_TAG_MIN),
        }
    }
});
