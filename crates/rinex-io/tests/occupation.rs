//! Occupations found in files: split at new sites, moving antennas and
//! equipment changes, and joined across files that follow on.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;
#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

use gnss_rinex_io::occupation::{merge, segments};

/// A header line: content padded to 60 columns, then the label.
fn record(content: &str, label: &str) -> String {
    format!("{content:<60}{label}\n")
}

fn header(marker: &str, antenna: &str, height: f64) -> String {
    [
        record(
            "     3.04           OBSERVATION DATA    G",
            "RINEX VERSION / TYPE",
        ),
        record(marker, "MARKER NAME"),
        record(
            "1234                LEICA GR30          4.82",
            "REC # / TYPE / VERS",
        ),
        record(
            &format!("5678                {antenna:<16}NONE"),
            "ANT # / TYPE",
        ),
        record(
            &format!("{height:14.4}{:14.4}{:14.4}", 0.0, 0.0),
            "ANTENNA: DELTA H/E/N",
        ),
        record("G    1 C1C", "SYS / # / OBS TYPES"),
        record("", "END OF HEADER"),
    ]
    .concat()
}

/// An epoch at `t` seconds after 2026-01-01 00:00 with one satellite.
fn epoch(t: u32) -> String {
    format!(
        "> 2026 01 01 {:02} {:02} {:10.7}  0  1\nG01  20000000.000\n",
        t / 3600,
        t % 3600 / 60,
        f64::from(t % 60)
    )
}

/// An event at `t` with header records.
fn event(t: u32, flag: u8, records: &[String]) -> String {
    let mut out = format!(
        "> 2026 01 01 {:02} {:02} {:10.7}  {flag} {:2}\n",
        t / 3600,
        t % 3600 / 60,
        f64::from(t % 60),
        records.len()
    );
    for r in records {
        out.push_str(r);
    }
    out
}

#[test]
fn a_file_splits_where_the_setup_changes() {
    let mut text = header("PM1", "LEIAS10", 1.5);
    // PM1 static, 0 to 300 s; the antenna moves; a new site, PM2, at
    // 1.6 m; its height changes at 1200 s; an external event and a gap
    // don't split it.
    text.extend((0..=10).map(|k| epoch(30 * k)));
    text.push_str(&event(310, 2, &[]));
    text.extend((11..=15).map(|k| epoch(30 * k)));
    text.push_str(&event(
        460,
        3,
        &[
            record("PM2", "MARKER NAME"),
            record(
                &format!("{:14.4}{:14.4}{:14.4}", 1.6, 0.0, 0.0),
                "ANTENNA: DELTA H/E/N",
            ),
        ],
    ));
    text.extend((16..=40).map(|k| epoch(30 * k)));
    text.push_str(&event(
        1210,
        4,
        &[record(
            &format!("{:14.4}{:14.4}{:14.4}", 1.7, 0.0, 0.0),
            "ANTENNA: DELTA H/E/N",
        )],
    ));
    text.push_str(&event(1220, 5, &[]));
    text.extend((41..=50).chain(60..=70).map(|k| epoch(30 * k)));

    let s = segments(text.as_bytes()).unwrap();
    let summary: Vec<_> = s
        .iter()
        .map(|g| {
            (
                g.setup.marker.as_str(),
                g.setup.antenna_delta.unwrap()[0],
                g.kinematic,
                g.epochs,
                g.gaps.len(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("PM1", 1.5, false, 11, 0),
            ("PM1", 1.5, true, 5, 0),
            ("PM2", 1.6, false, 25, 0),
            ("PM2", 1.7, false, 21, 1),
        ]
    );
    assert_eq!(s[0].interval, Some(30.0));
    assert_eq!(s[0].last.to_string(), "2026-01-01T00:05:00.0000000");
    assert_eq!(s[3].gaps[0].seconds(), 300.0);

    // A header record that changes nothing doesn't split.
    let mut same = header("PM1", "LEIAS10", 1.5);
    same.extend((0..5).map(|k| epoch(30 * k)));
    same.push_str(&event(150, 4, &[record("a comment", "COMMENT")]));
    same.extend((5..10).map(|k| epoch(30 * k)));
    assert_eq!(segments(same.as_bytes()).unwrap().len(), 1);
}

#[test]
fn files_that_follow_on_are_one_occupation() {
    let file = |marker: &str, height: f64, from: u32, to: u32| {
        let mut text = header(marker, "LEIAS10", height);
        text.extend((from..to).map(|k| epoch(30 * k)));
        segments(text.as_bytes()).unwrap()
    };
    let files = vec![
        // Hourly files of PM1 from 00:00 to 02:00, given out of order...
        file("PM1", 1.5, 120, 240),
        file("PM1", 1.5, 0, 120),
        // ...PM1 again at 03:00, after a gap of an hour: set up again?
        file("PM1", 1.5, 360, 480),
        // ...and PM2 alongside, at another height.
        file("PM2", 1.6, 0, 240),
    ];
    let o = merge(&files, 300.0);
    let summary: Vec<_> = o
        .iter()
        .map(|o| {
            (
                o.setup.marker.as_str(),
                o.parts.clone(),
                o.epochs,
                o.gaps.len(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("PM1", vec![(1, 0), (0, 0)], 240, 0),
            ("PM2", vec![(3, 0)], 240, 0),
            ("PM1", vec![(2, 0)], 120, 0),
        ]
    );
    assert_eq!(o[0].duration_s(), 239.0 * 30.0);

    // Joining across a short gap records it; the same data twice
    // overlaps.
    let gappy = vec![file("PM1", 1.5, 0, 100), file("PM1", 1.5, 105, 200)];
    let o = merge(&gappy, 300.0);
    assert_eq!((o.len(), o[0].gaps.len()), (1, 1));
    assert_eq!(o[0].gaps[0].seconds(), 180.0);
    let twice = vec![file("PM1", 1.5, 0, 100), file("PM1", 1.5, 0, 100)];
    let o = merge(&twice, 300.0);
    assert_eq!((o.len(), o[0].overlap_s), (1, 99.0 * 30.0));
}
