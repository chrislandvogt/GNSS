//! Prints the quality metrics for observation files:
//! `cargo run --release --example qc -- file...`. With `QC_NAV` set to
//! navigation files (comma-separated), elevation coverage too.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::BufReader;

use gnss_rinex_io::orbit::Orbits;
use gnss_rinex_io::qc::{Qc, QcOptions};

fn main() {
    let mut ephemerides = Vec::new();
    for path in std::env::var("QC_NAV").unwrap_or_default().split(',') {
        if !path.is_empty() {
            let file = std::fs::File::open(path).unwrap();
            ephemerides.extend(
                gnss_rinex_io::read_nav(BufReader::new(file))
                    .unwrap()
                    .ephemerides,
            );
        }
    }
    for path in std::env::args().skip(1) {
        let file = std::fs::File::open(&path).unwrap();
        let mut reader = match gnss_rinex_io::read_obs(BufReader::new(file)) {
            Ok(r) => r,
            Err(e) => {
                println!("{path}: {e}");
                continue;
            }
        };
        let mut options = QcOptions::default();
        let env = |name: &str| std::env::var(name).ok().and_then(|v| v.parse().ok());
        if let Some(v) = env("QC_GF") {
            options.gf_metres = v;
        }
        if let Some(v) = env("QC_GF_RATE") {
            options.gf_metres_per_second = v;
        }
        if let Some(v) = env("QC_MW_MIN") {
            options.mw_min_cycles = v;
        }
        let mut qc = Qc::new(reader.header(), options);
        if let (false, Some(position)) = (ephemerides.is_empty(), reader.header().approx_position) {
            qc = qc.with_orbits(Orbits::new(ephemerides.clone()), position);
        }
        loop {
            match reader.next_epoch() {
                Ok(Some(e)) => {
                    if !e.records.is_empty() {
                        qc.replan(reader.header());
                    }
                    qc.add(&e);
                }
                Ok(None) => break,
                Err(e) => {
                    println!("{path}: {e}");
                    break;
                }
            }
        }
        let r = qc.finish();
        println!(
            "{} {:?} interval {:?} s, {} of {:?} epochs ({:.1}%), {} gaps, {} events",
            reader.header().marker_name,
            r.first.map(|t| t.to_string()),
            r.interval,
            r.epochs,
            r.expected_epochs,
            100.0 * r.completeness().unwrap_or(0.0),
            r.gaps.len(),
            r.events
        );
        for (letter, s) in &r.systems {
            let mp: Vec<String> = s
                .multipath
                .iter()
                .map(|(b, m)| format!("MP{b} {:.3} m ({})", m.rms, m.observations))
                .collect();
            let obs_per_slip = s
                .signals
                .values()
                .map(|x| x.observations)
                .max()
                .map(|n| n.checked_div(s.slips).unwrap_or(n));
            println!(
                "  {letter}: {} sats, pair {:?}, slips {} (flagged {}, outliers {}), o/slip {:?}, {}",
                s.satellites,
                s.pair,
                s.slips,
                s.flagged,
                s.outliers,
                obs_per_slip,
                mp.join(", ")
            );
            if let Some(c) = &s.coverage {
                println!(
                    "     coverage {:?}: {} of {} expected, {} below {}°, {} unhealthy, {} no ephemeris",
                    c.ratio().map(|r| format!("{:.2}%", 100.0 * r)),
                    c.tracked,
                    c.expected,
                    c.below_cutoff,
                    c.cutoff,
                    c.unhealthy,
                    c.no_ephemeris
                );
                let signals: Vec<String> = s
                    .signals
                    .iter()
                    .filter_map(|(code, x)| Some(format!("{code} {:.1}%", 100.0 * x.coverage?)))
                    .collect();
                println!("     {}", signals.join(", "));
            }
        }
    }
}
