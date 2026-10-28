//! Any bytes read as a navigation file must give ephemerides or an error,
//! never a panic; and every ephemerides' positions, near their reference
//! time and far from it, must come out without one.

#![no_main]

use gnss_rinex_io::orbit::Orbits;
use gnss_rinex_io::{GnssTime, TICKS_PER_SECOND};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(nav) = gnss_rinex_io::read_nav(data) {
        for e in &nav.ephemerides {
            for dt in [0, 1, -1_000, 7_200, 1_000_000] {
                let t = GnssTime::from_ticks(e.toe().ticks().saturating_add(dt * TICKS_PER_SECOND));
                let _ = e.position(t);
            }
        }
        let orbits = Orbits::new(nav.ephemerides.clone());
        let _ = orbits.glonass_channels();
        for e in &nav.ephemerides {
            let _ = orbits.position(e.sv(), e.toe());
        }
    }
});
