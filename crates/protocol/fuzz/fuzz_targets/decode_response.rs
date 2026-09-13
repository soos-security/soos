#![no_main]

use libfuzzer_sys::fuzz_target;
use soos_protocol::{decode, Response};

fuzz_target!(|data: &[u8]| {
    if let Ok(resp) = decode::<Response>(data) {
        let _ = resp.is_allow();
    }
});
