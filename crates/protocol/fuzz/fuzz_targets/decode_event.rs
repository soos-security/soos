#![no_main]

use libfuzzer_sys::fuzz_target;
use soos_protocol::{decode, Event};

fuzz_target!(|data: &[u8]| {
    let _ = decode::<Event>(data);
});
