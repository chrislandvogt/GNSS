//! The quality metrics on synthetic observations with known effects:
//! multipath of known size, slips of known kinds at known epochs, a
//! receiver's loss-of-lock flag, a data gap and a change of signal.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;
#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

use gnss_rinex_io::qc::{Qc, QcOptions, QcReport};
use gnss_rinex_io::{Sv, read_obs};

const C: f64 = 299_792_458.0;
const F1: f64 = 1575.42e6;
const F2: f64 = 1227.60e6;
const EPOCHS: usize = 120;
const STEP: f64 = 30.0;
/// Code multipath on G01's C1C: 0.3 m, with a 600 s period (20 epochs).
const MP_AMPLITUDE: f64 = 0.3;

fn field(value: Option<f64>, lli: Option<u8>, ssi: u8) -> String {
    match value {
        None => " ".repeat(16),
        Some(v) => format!(
            "{v:14.3}{}{ssi}",
            lli.map_or(" ".to_owned(), |l| l.to_string())
        ),
    }
}

/// Observations of one satellite at time t: C1C L1C S1C C2W L2W S2W, then
/// C1X L1X (the other L1 tracking mode).
fn satellite(
    prn: u8,
    k: usize,
    slips: &[(usize, f64, f64)],
    lli_at: Option<usize>,
    x_mode: bool,
) -> String {
    let t = k as f64 * STEP;
    let range = 2.2e7 + 1000.0 * f64::from(prn) + 120.0 * t;
    // The ionosphere at L1, in metres, drifting slowly.
    let iono = 3.0 + 0.0005 * t;
    let gamma = (F1 / F2) * (F1 / F2);
    let (l1, l2) = (C / F1, C / F2);
    let mut n1 = 1.0e6 + f64::from(prn);
    let mut n2 = 2.0e6 - f64::from(prn);
    for &(at, d1, d2) in slips {
        if k >= at {
            n1 += d1;
            n2 += d2;
        }
    }
    let multipath = if prn == 1 {
        MP_AMPLITUDE * libm::sin(2.0 * std::f64::consts::PI * t / 600.0)
    } else {
        0.0
    };
    let c1 = range + iono + multipath;
    let c2 = range + gamma * iono;
    let p1 = (range - iono) / l1 + n1;
    let p2 = (range - gamma * iono) / l2 + n2;
    let lli = (lli_at == Some(k)).then_some(1);
    let (c1c, l1c, c1x, l1x) = if x_mode {
        (None, None, Some(c1), Some(p1))
    } else {
        (Some(c1), Some(p1), None, None)
    };
    [
        field(c1c, None, 7),
        field(l1c, lli, 7),
        field(Some(45.0 - f64::from(prn)), None, 7),
        field(Some(c2), None, 6),
        field(Some(p2), None, 6),
        field(Some(38.0), None, 6),
        field(c1x, None, 7),
        field(l1x, None, 7),
    ]
    .concat()
}

/// The file: G01 throughout; G02 with slips and a flag; G03 switching
/// tracking mode halfway. Epochs 60 to 79 are missing.
fn rinex() -> String {
    let mut text = String::from(
        "     3.04           OBSERVATION DATA    G                   RINEX VERSION / TYPE
G    8 C1C L1C S1C C2W L2W S2W C1X L1X                      SYS / # / OBS TYPES
    30.000                                                  INTERVAL
                                                            END OF HEADER
",
    );
    // G02: 5 cycles on L1 at epoch 40; one cycle on both bands at 100 (the
    // Melbourne-Wübbena combination can't see that one); a loss-of-lock
    // flag at 110.
    let g02_slips = [(40, 5.0, 0.0), (100, 1.0, 1.0)];
    for k in (0..EPOCHS).filter(|k| !(60..80).contains(k)) {
        let t = k as f64 * STEP;
        let (minute, second) = ((t / 60.0) as u32, t % 60.0);
        text.push_str(&format!(
            "> 2026 01 01 {:02} {:02} {second:10.7}  0  3\n",
            minute / 60,
            minute % 60
        ));
        for (prn, slips, lli, x) in [
            (1, &[][..], None, false),
            (2, &g02_slips[..], Some(110), false),
            (3, &[][..], None, k >= 30),
        ] {
            text.push_str(&format!(
                "G{prn:02}{}\n",
                satellite(prn, k, slips, lli, x).trim_end()
            ));
        }
    }
    text
}

fn qc(text: &str, options: QcOptions) -> QcReport {
    let mut reader = read_obs(text.as_bytes()).unwrap();
    let mut qc = Qc::new(reader.header(), options);
    while let Some(epoch) = reader.next_epoch().unwrap() {
        qc.add(&epoch);
    }
    qc.finish()
}

fn sv(prn: u8) -> Sv {
    Sv { system: 'G', prn }
}

#[test]
fn completeness_and_gaps() {
    let r = qc(&rinex(), QcOptions::default());
    assert_eq!(r.interval, Some(30.0));
    assert_eq!(r.epochs, 100);
    assert_eq!(r.expected_epochs, Some(120));
    assert!((r.completeness().unwrap() - 100.0 / 120.0).abs() < 1e-12);
    assert_eq!(r.gaps.len(), 1);
    assert_eq!(r.gaps[0].seconds(), 630.0);
    assert_eq!(r.gaps[0].after.to_string(), "2026-01-01T00:29:30.0000000");
    let g = &r.systems[&'G'];
    assert_eq!((g.satellites, g.satellite_epochs), (3, 300));
    // L1C on G03 only for its first 30 epochs; L1X after.
    assert_eq!(g.signals["L1C"].observations, 200 + 30);
    assert_eq!(g.signals["L1X"].observations, 70);
    assert!((g.signals["L2W"].completeness - 1.0).abs() < 1e-12);
    let s1 = g.signals["S1C"].strength.unwrap();
    assert_eq!((s1.min, s1.max, s1.count), (42.0, 44.0, 300));
    assert!((s1.mean - 43.0).abs() < 1e-12);
    assert!(g.signals["C1C"].strength.is_none());
}

#[test]
fn slips_are_found_where_they_are() {
    let r = qc(&rinex(), QcOptions::default());
    // G01 is clean through the gap; G03's change of signal isn't a slip.
    assert_eq!(r.satellites[&sv(1)].slips, 0);
    assert_eq!(r.satellites[&sv(3)].slips, 0);
    // G02: the 5-cycle slip and the 1-1 slip, and the flag counted apart.
    assert_eq!(r.satellites[&sv(2)].slips, 2);
    assert_eq!(r.satellites[&sv(2)].flagged, 1);
    let g = &r.systems[&'G'];
    assert_eq!((g.pair, g.slips, g.flagged), (Some((1, 2)), 2, 1));

    // Without the geometry-free test, the 1-1 slip goes unseen: only the
    // Melbourne-Wübbena test finds the other.
    let blind = QcOptions {
        gf_metres: 10.0,
        ..QcOptions::default()
    };
    assert_eq!(qc(&rinex(), blind).satellites[&sv(2)].slips, 1);
}

#[test]
fn multipath_is_measured() {
    // G01's 0.3 m sinusoid over whole periods has an RMS of 0.3/√2. G02 and
    // G03 have none, so their arcs pull the system's RMS down by the share
    // of their observations; measure G01 alone by dropping them.
    let text: String = rinex()
        .lines()
        .filter(|l| !l.starts_with("G02") && !l.starts_with("G03"))
        .map(|l| format!("{}\n", l.replace("  0  3", "  0  1")))
        .collect();
    let r = qc(&text, QcOptions::default());
    let g = &r.systems[&'G'];
    let mp1 = g.multipath[&1];
    assert_eq!(mp1.observations, 100);
    let expected = MP_AMPLITUDE / 2.0_f64.sqrt();
    assert!(
        (mp1.rms - expected).abs() < 0.002,
        "MP1 {} against {expected}",
        mp1.rms
    );
    // L2's code has no multipath: only the values' rounding to 1 mm.
    assert!(g.multipath[&2].rms < 0.002, "MP2 {}", g.multipath[&2].rms);

    // Arcs shorter than the minimum don't count.
    let long = QcOptions {
        min_arc_epochs: 61,
        ..QcOptions::default()
    };
    assert!(qc(&text, long).systems[&'G'].multipath.is_empty());
}

#[test]
fn glonass_needs_its_channels() {
    let text = "     3.04           OBSERVATION DATA    R                   RINEX VERSION / TYPE
R    4 C1C L1C C2C L2C                                      SYS / # / OBS TYPES
                                                            END OF HEADER
> 2026 01 01 00 00  0.0000000  0  1
R01  22000000.000   117000000.000   22000000.000    91000000.000
";
    let r = qc(text, QcOptions::default());
    let s = &r.systems[&'R'];
    assert_eq!(s.pair, Some((1, 2)));
    assert_eq!(s.satellite_epochs, 1);
    // No channel for R01, so no slips or multipath can be computed.
    assert!(s.multipath.is_empty());
}

/// Hobart (HOB2), ECEF metres, near enough.
const HOB2: [f64; 3] = [-3_950_072.0, 2_522_415.0, -4_311_638.0];

/// Elevation in degrees, worked independently of the crate: geodetic
/// latitude by Bowring's formula, then the up component.
fn elevation(station: [f64; 3], sat: [f64; 3]) -> f64 {
    let (a, f) = (6_378_137.0_f64, 1.0 / 298.257_223_563);
    let b = a * (1.0 - f);
    let e2 = f * (2.0 - f);
    let ep2 = e2 / (1.0 - e2);
    let [x, y, z] = station;
    let p = (x * x + y * y).sqrt();
    let theta = libm::atan2(z * a, p * b);
    let (st, ct) = (libm::sin(theta), libm::cos(theta));
    let lat = libm::atan2(z + ep2 * b * st * st * st, p - e2 * a * ct * ct * ct);
    let lon = libm::atan2(y, x);
    let up = [
        libm::cos(lat) * libm::cos(lon),
        libm::cos(lat) * libm::sin(lon),
        libm::sin(lat),
    ];
    let d = [sat[0] - x, sat[1] - y, sat[2] - z];
    let r = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    libm::asin((up[0] * d[0] + up[1] * d[1] + up[2] * d[2]) / r).to_degrees()
}

#[test]
fn elevation_coverage_counts_what_was_in_the_sky() {
    use gnss_rinex_io::orbit::Orbits;

    // RTKLIB's GPS positions at 00:00:01 and every 15 minutes, from the
    // navigation test file; the day is a Sunday, so seconds of week are
    // seconds of day.
    let mut sky: Vec<(usize, u8, f64)> = Vec::new();
    for line in include_str!("data/nav/rtklib.txt").lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if !f[0].starts_with("HOB2") || !f[1].starts_with('G') {
            continue;
        }
        let k = (f[2].parse::<f64>().unwrap() as usize - 1) / 900;
        let p = [3, 4, 5].map(|i| f[i].parse::<f64>().unwrap());
        sky.push((k, f[1][1..].parse().unwrap(), elevation(HOB2, p)));
    }
    assert!(sky.iter().all(|s| (s.2 - 10.0).abs() > 0.1), "{sky:?}");
    let above: Vec<_> = sky.iter().filter(|s| s.2 >= 10.0).collect();
    let below = sky.len() - above.len();
    assert!(!above.is_empty() && below > 0, "{sky:?}");

    // Observations of every satellite in the sky (above the horizon or
    // not) at every epoch but one, and the first above the cutoff missing.
    let skip = (above[0].0, above[0].1);
    let mut text = format!(
        "     3.04           OBSERVATION DATA    G                   RINEX VERSION / TYPE
{:14.4}{:14.4}{:14.4}                  APPROX POSITION XYZ
G    2 C1C L1C                                              SYS / # / OBS TYPES
                                                            END OF HEADER
",
        HOB2[0], HOB2[1], HOB2[2]
    );
    let missing_epoch = 5;
    let mut written_above = 0;
    let mut written_below = 0;
    for k in (0..12).filter(|&k| k != missing_epoch) {
        let sats: Vec<_> = sky
            .iter()
            .filter(|s| s.0 == k && (s.0, s.1) != skip)
            .collect();
        let t = 1 + 900 * k;
        text.push_str(&format!(
            "> 2025 06 15 {:02} {:02} {:10.7}  0 {:2}\n",
            t / 3600,
            t % 3600 / 60,
            (t % 60) as f64,
            sats.len()
        ));
        for s in sats {
            text.push_str(&format!("G{:02}  20000000.000   100000000.000\n", s.1));
            if s.2 >= 10.0 {
                written_above += 1;
            } else {
                written_below += 1;
            }
        }
    }
    let mut reader = read_obs(text.as_bytes()).unwrap();
    let nav = gnss_rinex_io::read_nav(
        &include_bytes!("data/nav/HOB200AUS_R_20251660000_excerpt_MN.rnx")[..],
    )
    .unwrap();
    let mut qc = Qc::new(reader.header(), QcOptions::default())
        .with_orbits(Orbits::new(nav.ephemerides), HOB2);
    while let Some(epoch) = reader.next_epoch().unwrap() {
        qc.add(&epoch);
    }
    let r = qc.finish();
    assert_eq!(r.expected_epochs, Some(12));
    let g = &r.systems[&'G'];
    let c = g.coverage.unwrap();
    // Expected: every satellite above the cutoff at every expected epoch,
    // the missing one included.
    assert_eq!(c.expected, above.len());
    assert_eq!(c.tracked, written_above);
    assert_eq!(c.below_cutoff, written_below);
    assert_eq!((c.unhealthy, c.no_ephemeris), (0, 0));
    assert!((c.ratio().unwrap() - written_above as f64 / above.len() as f64).abs() < 1e-12);
    assert!(
        (g.signals["L1C"].coverage.unwrap() - written_above as f64 / above.len() as f64).abs()
            < 1e-12
    );
    // No orbits for GLONASS in a GPS-only file: nothing to say.
    assert!(!r.systems.contains_key(&'R'));
}
