//! Any bytes read as SINEX must give a survey or an error, never a panic,
//! whether the frame is given or looked for in the file.

#![no_main]

use gnss_datum::FrameId;
use gnss_formats::{decode, sinex};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = decode(data);
    let _ = sinex::read("fuzz.snx", &text, None);
    let _ = sinex::read("fuzz.snx", &text, Some(FrameId::Itrf2014));
});
