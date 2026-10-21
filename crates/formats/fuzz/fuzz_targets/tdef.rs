//! Any bytes read as a Trimble Data Exchange Format file must give a survey
//! or an error, never a panic, in a static frame and in a dynamic one.

#![no_main]

use gnss_datum::FrameId;
use gnss_formats::{decode, tdef};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = decode(data);
    let _ = tdef::read("fuzz.asc", &text, FrameId::Gda2020, None);
    let _ = tdef::read("fuzz.asc", &text, FrameId::Itrf2014, None);
});
