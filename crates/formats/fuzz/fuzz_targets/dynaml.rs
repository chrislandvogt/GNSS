//! Any bytes read as DynaML must give a survey or an error, never a panic.
//! What reads is written back out, and must read again.

#![no_main]

use gnss_datum::FrameId;
use gnss_formats::{Reference, decode, dynaml};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(survey) = dynaml::read("fuzz.xml", &decode(data)) else {
        return;
    };
    let reference = Reference {
        frame: FrameId::Gda2020,
        epoch: None,
    };
    let written = dynaml::write_measurements(&survey.measurements, reference);
    let again = dynaml::read("again.xml", &written);
    assert!(again.is_ok(), "{again:?}\n{written}");
});
