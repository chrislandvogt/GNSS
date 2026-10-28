//! Navigation files read in RINEX 2, 3 and 4, and the satellite positions
//! from them, against RTKLIB's and against precise orbits. The files and
//! expected values are made by `tests/data/nav/make_fixtures.py`.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;
#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

use std::collections::BTreeMap;

use gnss_rinex_io::orbit::{Ephemeris, Orbits};
use gnss_rinex_io::{GnssTime, Sv, TICKS_PER_SECOND, read_nav};

const HOB2: &str = include_str!("data/nav/HOB200AUS_R_20251660000_excerpt_MN.rnx");
const HOB2_V4: &str = include_str!("data/nav/HOB200AUS_R_20251660000_excerpt_MN4.rnx");
const MOBS_N: &str = include_str!("data/nav/mobs0050_excerpt.20n");
const MOBS_G: &str = include_str!("data/nav/mobs0050_excerpt.20g");
const RTKLIB: &str = include_str!("data/nav/rtklib.txt");
const GINAN: &str = include_str!("data/nav/ginan.txt");

fn orbits(text: &str) -> Orbits {
    Orbits::new(read_nav(text.as_bytes()).unwrap().ephemerides)
}

fn sv(id: &str) -> Sv {
    Sv {
        system: id.chars().next().unwrap(),
        prn: id[1..].parse().unwrap(),
    }
}

/// A GPS time from its seconds of week, in the week of `date`.
fn at(date: (i64, i64, i64), seconds_of_week: f64) -> GnssTime {
    let day = GnssTime::from_fields(date.0, date.1, date.2, 0, 0, "0").unwrap();
    let week = 604_800 * TICKS_PER_SECOND;
    let start = day.ticks() - day.ticks().rem_euclid(week);
    GnssTime::from_ticks(start + (seconds_of_week * TICKS_PER_SECOND as f64).round() as i64)
}

fn distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f64>()
        .sqrt()
}

/// RTKLIB's positions and clocks for each file, by satellite and time.
fn rtklib() -> BTreeMap<&'static str, BTreeMap<(String, String), [f64; 4]>> {
    let mut out: BTreeMap<_, BTreeMap<_, _>> = BTreeMap::new();
    for line in RTKLIB.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let v = |i: usize| f[i].parse::<f64>().unwrap();
        out.entry(f[0])
            .or_default()
            .insert((f[1].to_owned(), f[2].to_owned()), [v(3), v(4), v(5), v(6)]);
    }
    out
}

/// Every satellite at every test time: the same positions as RTKLIB's to
/// 0.1 mm, the same clocks to 1e-14 s, and none where RTKLIB has none.
fn matches_rtklib(file: &str, text: &str, date: (i64, i64, i64)) {
    let expected = &rtklib()[file];
    let orbits = orbits(text);
    let times: Vec<String> = {
        let mut t: Vec<String> = expected.keys().map(|k| k.1.clone()).collect();
        t.sort_by(|a, b| a.parse::<f64>().unwrap().total_cmp(&b.parse().unwrap()));
        t.dedup();
        t
    };
    assert_eq!(times.len(), 12, "{file}");
    let mut seen = 0;
    for s in orbits.satellites().collect::<Vec<_>>() {
        let id = format!("{}{:02}", s.system, s.prn);
        for time in &times {
            let t = at(date, time.parse().unwrap());
            let ours = orbits.position(s, t);
            match (expected.get(&(id.clone(), time.clone())), ours) {
                (Some(e), Some(p)) => {
                    let d = distance(&p.position, &e[..3]);
                    assert!(d < 1e-4, "{file} {id} at {time}: {d} m from RTKLIB");
                    assert!(
                        (p.clock - e[3]).abs() < 1e-14,
                        "{file} {id} at {time}: clock"
                    );
                    seen += 1;
                }
                (None, None) => {}
                // RTKLIB takes an ephemeris up to a second past its
                // validity; we don't.
                (Some(_), None)
                    if orbits
                        .position(s, GnssTime::from_ticks(t.ticks() - TICKS_PER_SECOND))
                        .is_some()
                        || orbits
                            .position(s, GnssTime::from_ticks(t.ticks() + TICKS_PER_SECOND))
                            .is_some() =>
                {
                    seen += 1;
                }
                (e, p) => panic!("{file} {id} at {time}: RTKLIB {e:?}, ours {p:?}"),
            }
        }
    }
    assert_eq!(seen, expected.len(), "{file}");
}

#[test]
fn rinex3_every_system_matches_rtklib() {
    matches_rtklib(
        "HOB200AUS_R_20251660000_excerpt_MN.rnx",
        HOB2,
        (2025, 6, 15),
    );
}

#[test]
fn rinex2_gps_and_glonass_match_rtklib() {
    matches_rtklib("mobs0050_excerpt.20n", MOBS_N, (2020, 1, 5));
    matches_rtklib("mobs0050_excerpt.20g", MOBS_G, (2020, 1, 5));
}

#[test]
fn rinex4_reads_as_rinex3_does() {
    let v3 = read_nav(HOB2.as_bytes()).unwrap();
    let v4 = read_nav(HOB2_V4.as_bytes()).unwrap();
    assert_eq!((v4.version.major, v4.version.minor), (4, 1));
    // The CNAV, STO and ION records are skipped, the rest the same.
    assert_eq!(v3.ephemerides, v4.ephemerides);
}

#[test]
fn records_are_read_into_their_fields() {
    let nav = read_nav(MOBS_G.as_bytes()).unwrap();
    let Ephemeris::Glonass(r) = nav.ephemerides[0] else {
        panic!("{:?}", nav.ephemerides[0])
    };
    // R06 at 2020-01-04 22:15 UTC: 18 leap seconds later in GPS time; the
    // state in metres, its channel, and −τn.
    assert_eq!(r.sv, sv("R06"));
    assert_eq!(r.toe.to_string(), "2020-01-04T22:15:18.0000000");
    assert!((r.position[0] - 8_186_292.480_469).abs() < 1e-6);
    assert!((r.velocity[2] - 3_536.410_331_726).abs() < 1e-9);
    assert_eq!(r.channel, Some(-4));
    assert_eq!(r.clock, [1.688_161_864_877e-4, 9.094_947_017_729e-13]);

    let nav = read_nav(HOB2.as_bytes()).unwrap();
    let c01 = nav
        .ephemerides
        .iter()
        .find_map(|e| match e {
            Ephemeris::Kepler(k) if k.sv == sv("C01") => Some(*k),
            _ => None,
        })
        .unwrap();
    // BeiDou time is 14 s behind GPS time.
    assert_eq!(
        c01.toc.ticks() % (60 * TICKS_PER_SECOND),
        14 * TICKS_PER_SECOND
    );
    assert_eq!(c01.toe, c01.toc);
}

#[test]
fn broadcast_gps_orbits_are_within_metres_of_precise_ones() {
    // Broadcast orbits are for the antenna's phase centre, precise ones the
    // centre of mass: a metre or so apart for GPS. Together, under 5 m.
    let orbits = orbits(HOB2);
    let mut n = 0;
    for line in GINAN.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let t = at((2025, 6, 15), f[1].parse().unwrap());
        let precise: Vec<f64> = f[2..5].iter().map(|x| x.parse().unwrap()).collect();
        let Some(p) = orbits.position(sv(f[0]), t) else {
            continue;
        };
        let d = distance(&p.position, &precise);
        assert!(d < 5.0, "{} at {}: {d} m", f[0], f[1]);
        n += 1;
    }
    assert!(n >= 30, "{n}");
}

#[test]
fn ephemerides_are_chosen_by_time() {
    let orbits = orbits(HOB2);
    let g04 = sv("G04");
    // Nearest reference time; the later of two equally near.
    let t = at((2025, 6, 15), 3600.0);
    assert_eq!(
        orbits.ephemeris(g04, t).unwrap().toe(),
        at((2025, 6, 15), 7200.0)
    );
    // Galileo: never one from after the time.
    let e07 = sv("E07");
    let t = at((2025, 6, 15), 1799.0);
    assert!(orbits.ephemeris(e07, t).unwrap().toe() <= t);
    // Out of validity: none.
    let late = at((2025, 6, 15), 86_000.0);
    assert!(orbits.position(g04, late).is_none());
    assert!(orbits.position(sv("G01"), t).is_none());
}

#[test]
fn malformed_files_are_errors_naming_their_line() {
    let err = |text: &str| read_nav(text.as_bytes()).unwrap_err().to_string();
    let header = "     3.04           N: GNSS NAV DATA    M                   RINEX VERSION / TYPE\n                                                            END OF HEADER\n";
    assert!(err("").contains("empty"));
    assert!(
        err("     3.04           OBSERVATION DATA    M                   RINEX VERSION / TYPE\n")
            .contains("not a navigation file")
    );
    let mut short = header.to_owned();
    short.push_str("G01 2025 06 15 00 00 00 0.0 0.0 0.0\n     1.0 2.0\n");
    assert!(err(&short).starts_with("line 3:"), "{}", err(&short));
    let mut bad = header.to_owned();
    bad.push_str("R01 2025 06 15 00 00 00 .1D+01 x.5 .0\n");
    assert!(err(&bad).contains("isn't a number"), "{}", err(&bad));
    let mut bad_epoch = header.to_owned();
    bad_epoch.push_str("R01 2025 13 15 00 00 00\n");
    assert!(
        err(&bad_epoch).contains("isn't an epoch"),
        "{}",
        err(&bad_epoch)
    );
    assert!(err(&header[..81]).contains("END OF HEADER"));
}

#[test]
fn rinex2_sbas_satellites_are_numbered_as_in_rinex3() {
    let text = "     2.11           H: GEO NAV MSG DATA                     RINEX VERSION / TYPE
                                                            END OF HEADER
28 25  6 15  0  0  0.0 6.286427378654D-08 5.002220859751D-11 6.047990000000D+05
    5.078396000000D+03-4.362500000000D-04 5.000000000000D-08 3.100000000000D+01
    4.185647536000D+04 1.987500000000D-04 3.750000000000D-08 4.096000000000D+03
   -7.022800000000D+00 7.480000000000D-04 6.250000000000D-08 1.330000000000D+02
";
    let nav = read_nav(text.as_bytes()).unwrap();
    // PRN 128 in RINEX 2's numbering (PRN - 100), S28 in RINEX 3's.
    assert_eq!(nav.ephemerides[0].sv(), sv("S28"));
}
