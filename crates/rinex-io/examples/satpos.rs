//! Broadcast satellite positions every 300 s for a day, from navigation
//! files: `satpos YYYY-MM-DD NAV...`. Prints satellite, GPS seconds of
//! week, ECEF x y z (m), clock (s), health and the ephemeris's age (s,
//! from its reference time), for comparing with another implementation.

#![allow(clippy::print_stdout, clippy::unwrap_used)]

use std::io::BufReader;

use gnss_rinex_io::orbit::Orbits;
use gnss_rinex_io::{GnssTime, TICKS_PER_SECOND, read_nav};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let date: Vec<i64> = args[0].split('-').map(|x| x.parse().unwrap()).collect();
    let mut ephemerides = Vec::new();
    for path in &args[1..] {
        let file = std::fs::File::open(path).unwrap();
        ephemerides.extend(read_nav(BufReader::new(file)).unwrap().ephemerides);
    }
    let orbits = Orbits::new(ephemerides);
    let offset: i64 = std::env::var("OFFSET").map_or(0, |o| o.parse().unwrap());
    let start = GnssTime::from_fields(date[0], date[1], date[2], 0, 0, "0").unwrap();
    let week = 604_800 * TICKS_PER_SECOND;
    for k in 0..288 {
        let t = GnssTime::from_ticks(start.ticks() + (k * 300 + offset) * TICKS_PER_SECOND);
        let sow = t.ticks().rem_euclid(week) as f64 / TICKS_PER_SECOND as f64;
        for sv in orbits.satellites().collect::<Vec<_>>() {
            if let (Some(p), Some(e)) = (orbits.position(sv, t), orbits.ephemeris(sv, t)) {
                println!(
                    "{}{:02} {sow:.1} {:.4} {:.4} {:.4} {:.12e} {} {:.0}",
                    sv.system,
                    sv.prn,
                    p.position[0],
                    p.position[1],
                    p.position[2],
                    p.clock,
                    u8::from(!p.healthy),
                    t.seconds_since(e.toe())
                );
            }
        }
    }
}
