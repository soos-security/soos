#![no_main]

use libfuzzer_sys::fuzz_target;
use soos_protocol::codec::decode_preview;
use soos_protocol::types::PreviewResponse;

// GitHub #227 (review PAM-15): the 2 MiB preview decoder must never panic, and a decoded
// frame never carries more pixel bytes than the preview limit allows.
fuzz_target!(|data: &[u8]| {
    if let Ok(frame) = decode_preview::<PreviewResponse>(data) {
        assert!(frame.data.len() <= soos_protocol::types::MAX_PREVIEW_MESSAGE_SIZE);
    }
});
