//! Any bytes read as STAR*NET GPS vectors must give a survey or an error,
//! never a panic.

#![no_main]

use gnss_datum::FrameId;
use gnss_formats::{Reference, decode, starnet};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let reference = Reference {
        frame: FrameId::Gda2020,
        epoch: None,
    };
    let _ = starnet::read("fuzz.gps", &decode(data), reference);
});
