//! Any bytes read as an observation file, plain or compressed, must give
//! epochs and then an end or an error, never a panic.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(mut reader) = gnss_rinex_io::read_obs(data) {
        while let Ok(Some(_)) = reader.next_epoch() {}
    }
});
