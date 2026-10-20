//! Any bytes read as a CSV vector or control file must give a survey or an
//! error, never a panic.

#![no_main]

use gnss_datum::FrameId;
use gnss_formats::{Reference, csv, decode};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = decode(data);
    let reference = Some(Reference {
        frame: FrameId::Gda2020,
        epoch: None,
    });
    let _ = csv::read_vectors("fuzz.csv", &text, reference);
    let _ = csv::read_control("fuzz.csv", &text, reference);
    let _ = csv::read_control("fuzz.csv", &text, None);
});
