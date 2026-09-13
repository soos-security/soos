#![no_main]

use libfuzzer_sys::fuzz_target;
use soos_protocol::{decode, Request};

fuzz_target!(|data: &[u8]| {
    if let Ok(req) = decode::<Request>(data) {
        let _ = req.validate();
    }
});
