//! Any bytes read as an RTKLIB solution must give a survey or an error,
//! never a panic.

#![no_main]

use gnss_formats::rtklib::{Names, read};
use gnss_formats::decode;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = read("fuzz.pos", &decode(data), None, &Names::default());
});
