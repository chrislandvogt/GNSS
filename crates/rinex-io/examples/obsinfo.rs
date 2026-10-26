//! Reads an observation file, plain or compressed, and prints a summary:
//! `cargo run --release --example obsinfo -- file...`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::BufReader;

fn main() {
    for path in std::env::args().skip(1) {
        let start = std::time::Instant::now();
        let file = std::fs::File::open(&path).unwrap();
        let mut reader = match gnss_rinex_io::read_obs(BufReader::new(file)) {
            Ok(r) => r,
            Err(e) => {
                println!("{path}: {e}");
                continue;
            }
        };
        let (mut epochs, mut events, mut observations) = (0, 0, 0usize);
        let (mut first, mut last) = (None, None);
        loop {
            match reader.next_epoch() {
                Ok(Some(e)) => {
                    if e.flag.has_observations() {
                        epochs += 1;
                        observations += e
                            .satellites
                            .iter()
                            .map(|s| s.observations.iter().flatten().count())
                            .sum::<usize>();
                        first = first.or(e.time);
                        last = e.time;
                    } else {
                        events += 1;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    println!("{path}: {e}");
                    break;
                }
            }
        }
        let h = reader.header();
        println!(
            "{} v{}.{:02} {} | {} {} | {} {:?} | epochs {epochs} events {events} obs {observations} | {} .. {} | {:.2?}",
            h.marker_name,
            h.version.major,
            h.version.minor,
            h.time_system.code(),
            h.receiver.kind,
            h.antenna.kind,
            h.antenna.radome.as_deref().unwrap_or("-"),
            h.antenna_delta,
            first.map(|t| t.to_string()).unwrap_or_default(),
            last.map(|t| t.to_string()).unwrap_or_default(),
            start.elapsed()
        );
    }
}
