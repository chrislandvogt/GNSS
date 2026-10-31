//! Trimming observation files to a window and interval before processing
//! (ARCHITECTURE.md section 9.3), and checking navigation data covers the
//! constellations processed.

#![allow(clippy::unwrap_used)]

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

use gnss_rinex_io::{
    Epoch, GnssTime, HeaderPatch, Trim, read_nav, read_obs, rewrite_obs, trim_obs,
};

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const V3: &[u8] = include_bytes!("data/rinex/synthetic3.rnx");
const V3_CRX: &[u8] = include_bytes!("data/rinex/synthetic3.rnx.crx");
/// What CRX2RNX restores from it: the same data, a number or two written
/// differently from the plain file.
const V3_RESTORED: &[u8] = include_bytes!("data/rinex/synthetic3.rnx.crx2rnx");
const V2: &[u8] = include_bytes!("data/rinex/synthetic2.11o");
const NAV: &[u8] = include_bytes!("data/nav/HOB200AUS_R_20251660000_excerpt_MN.rnx");

fn t(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> GnssTime {
    GnssTime::from_fields(y, mo, d, h, mi, &s.to_string()).unwrap()
}

fn epochs(file: &[u8]) -> Vec<Epoch> {
    let mut reader = read_obs(file).unwrap();
    let mut all = Vec::new();
    while let Some(e) = reader.next_epoch().unwrap() {
        all.push(e);
    }
    all
}

fn trimmed(file: &[u8], trim: Trim) -> (Vec<u8>, gnss_rinex_io::Trimmed) {
    let mut out = Vec::new();
    let summary = trim_obs(file, &mut out, &HeaderPatch::default(), &[], trim).unwrap();
    (out, summary)
}

#[test]
fn epochs_outside_the_window_go_and_the_rest_are_copied_as_they_are() {
    let window = (t(2026, 1, 1, 0, 1, 0), t(2026, 1, 1, 0, 4, 0));
    let (out, summary) = trimmed(
        V3,
        Trim {
            window: Some(window),
            interval: None,
        },
    );
    let kept = epochs(&out);
    let all = epochs(V3);
    // Observations from 00:01:00 to 00:04:00 inclusive, as RTKLIB's -ts
    // and -te, and every event, whatever its time.
    let in_window =
        |e: &&Epoch| e.flag.is_event() || e.time.is_some_and(|t| window.0 <= t && t <= window.1);
    let expected: Vec<&Epoch> = all.iter().filter(in_window).collect();
    assert_eq!(kept.iter().collect::<Vec<_>>(), expected);
    assert_eq!(summary.kept, 7);
    assert_eq!(
        summary.read,
        all.iter().filter(|e| !e.flag.is_event()).count() as u64
    );
    assert!(kept.iter().any(|e| e.flag.is_event() && e.time.is_none()));
    // Every line written is one of the file's own, in order.
    let text = String::from_utf8(out).unwrap();
    let original: Vec<&str> = std::str::from_utf8(V3).unwrap().lines().collect();
    let mut at = 0;
    for line in text.lines() {
        at += original[at..].iter().position(|l| l == &line).unwrap() + 1;
    }
    // The header reads the same.
    assert_eq!(
        read_obs(text.as_bytes()).unwrap().header(),
        read_obs(V3).unwrap().header()
    );
}

#[test]
fn a_compressed_file_trims_to_the_same_plain_rinex() {
    let trim = Trim {
        window: Some((t(2026, 1, 1, 0, 2, 0), t(2026, 1, 1, 0, 6, 0))),
        interval: Some(60.0),
    };
    assert_eq!(trimmed(V3_CRX, trim).0, trimmed(V3_RESTORED, trim).0);
}

#[test]
fn decimating_keeps_whole_intervals() {
    let (out, summary) = trimmed(
        V3,
        Trim {
            window: None,
            interval: Some(60.0),
        },
    );
    let times: Vec<i64> = epochs(&out)
        .iter()
        .filter(|e| !e.flag.is_event())
        .map(|e| e.time.unwrap().fields().5)
        .collect();
    assert!(
        !times.is_empty() && times.iter().all(|s| *s == 0),
        "{times:?}"
    );
    assert_eq!(summary.kept, times.len() as u64);
    // No trim at all: the same as the plain rewrite.
    let mut plain = Vec::new();
    rewrite_obs(V3, &mut plain, &HeaderPatch::default(), &[]).unwrap();
    assert_eq!(trimmed(V3, Trim::default()).0, plain);
}

#[test]
fn rinex_2_epochs_trim_whole_with_their_continuation_lines() {
    let all = epochs(V2);
    let times: Vec<GnssTime> = all.iter().filter_map(|e| e.time).collect();
    let window = (times[1], times[times.len() - 2]);
    let (out, _) = trimmed(
        V2,
        Trim {
            window: Some(window),
            interval: None,
        },
    );
    let expected: Vec<&Epoch> = all
        .iter()
        .filter(|e| e.flag.is_event() || e.time.is_some_and(|t| window.0 <= t && t <= window.1))
        .collect();
    assert_eq!(epochs(&out).iter().collect::<Vec<_>>(), expected);
}

#[test]
fn navigation_data_must_cover_each_constellation_processed() {
    let nav = read_nav(NAV).unwrap();
    let window = (t(2025, 6, 15, 1, 0, 0), t(2025, 6, 15, 3, 30, 0));
    let coverage = nav.coverage(&['G', 'R', 'I'], window);
    // GPS: ephemerides near 02:00, valid for two hours either side.
    assert!(coverage[0].complete(), "{:?}", coverage[0]);
    assert!(coverage[0].satellites > 0);
    // GLONASS: the last is at 02:45, valid for 30 minutes, so the end of
    // the window isn't covered.
    let glonass = &coverage[1];
    assert!(!glonass.complete());
    assert_eq!(glonass.gaps.len(), 1);
    assert_eq!(glonass.gaps[0].1, window.1);
    assert!(glonass.gaps[0].0 > t(2025, 6, 15, 3, 0, 0), "{glonass:?}");
    // NavIC: none at all.
    assert_eq!(coverage[2].satellites, 0);
    assert_eq!(coverage[2].gaps, [window]);
}
