//! Any bytes restored as a Hatanaka-compressed file must give RINEX lines
//! and then an end or an error, never a panic.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = gnss_rinex_io::decompress(data, std::io::sink());
});
