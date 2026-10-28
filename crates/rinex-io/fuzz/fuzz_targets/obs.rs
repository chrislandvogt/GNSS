//! Any bytes read as an observation file, plain or compressed, must give
//! epochs and then an end or an error, never a panic; and the same for
//! finding its occupations and writing a corrected copy.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(mut reader) = gnss_rinex_io::read_obs(data) {
        while let Ok(Some(_)) = reader.next_epoch() {}
    }
    if let Ok(segments) = gnss_rinex_io::occupation::segments(data) {
        let _ = gnss_rinex_io::occupation::merge(&[segments.clone(), segments], 300.0);
    }
    let patch = gnss_rinex_io::HeaderPatch {
        marker_name: Some("PM1".into()),
        antenna: Some("LEIAS10".into()),
        antenna_delta: Some([1.5, 0.0, 0.0]),
        ..Default::default()
    };
    let _ = gnss_rinex_io::rewrite_obs(data, std::io::sink(), &patch, &["fuzz".into()]);
});
