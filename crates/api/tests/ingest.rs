//! Importing RINEX into a project: occupations found and named, files
//! joined, marks added, and nothing written when something's wrong.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use gnss_api::folder::load;
use gnss_api::ingest::{IngestError, IngestOptions, import_rinex};
use gnss_api::project::{AdjustmentSettings, Project};
use gnss_datum::FrameId;
use gnss_store::{MemoryStore, ProjectPath, Store};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;
#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

fn record(content: &str, label: &str) -> String {
    format!("{content:<60}{label}\n")
}

/// An epoch every 30 s for epochs `from..to` of 2026-01-01.
fn epochs(from: u32, to: u32) -> String {
    let mut text = String::new();
    for k in from..to {
        let t = 30 * k;
        text.push_str(&format!(
            "> 2026 01 01 {:02} {:02} {:10.7}  0  1\nG01  20000000.000\n",
            t / 3600,
            t % 3600 / 60,
            f64::from(t % 60)
        ));
    }
    text
}

/// A RINEX 3 file of `marker` with `antenna`, at MOBS (Melbourne), with
/// epochs `from..to` and then `tail`.
fn rinex(marker: &str, antenna: &str, from: u32, to: u32, tail: &str) -> Vec<u8> {
    let mut text = [
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
            &format!(
                "{:14.4}{:14.4}{:14.4}",
                -4_130_636.0, 2_894_953.0, -3_890_530.0
            ),
            "APPROX POSITION XYZ",
        ),
        record(
            &format!("{:14.4}{:14.4}{:14.4}", 1.5, 0.0, 0.0),
            "ANTENNA: DELTA H/E/N",
        ),
        record("G    1 C1C", "SYS / # / OBS TYPES"),
        record("", "END OF HEADER"),
    ]
    .concat();
    text.push_str(&epochs(from, to));
    text.push_str(tail);
    text.into_bytes()
}

fn project() -> MemoryStore {
    let mut store = MemoryStore::new();
    let settings = Project::new("Test", AdjustmentSettings::new(FrameId::Gda2020, None));
    store
        .write(
            &ProjectPath::new("project.toml").unwrap(),
            settings.to_toml().unwrap().as_bytes(),
        )
        .unwrap();
    store
}

fn options() -> IngestOptions {
    IngestOptions::default()
}

#[test]
fn files_become_occupations_and_marks() {
    let mut store = project();
    // PM1 for an hour, then the receiver taken to PM2 (a new site); the
    // next hour's file continues PM2.
    let new_site = format!(
        "> 2026 01 01 01 00  0.0000000  3  1\n{}{}",
        record("PM2", "MARKER NAME"),
        epochs(121, 240)
    );
    let first = rinex("PM1", "LEIAS10", 0, 120, &new_site);
    let files = vec![
        ("day1a.rnx".to_owned(), first),
        (
            "day1b.rnx".to_owned(),
            rinex("PM2", "LEIAS10", 240, 360, ""),
        ),
    ];
    let report = import_rinex(&mut store, &files, &options()).unwrap();
    let got: Vec<_> = report
        .occupations
        .iter()
        .map(|o| (o.id.as_str(), o.files.len(), o.epochs))
        .collect();
    assert_eq!(
        got,
        [("2026-01-01-PM1-A", 1, 120), ("2026-01-01-PM2-A", 2, 239)]
    );
    // 00:00 GPS time is 18 s earlier in UTC.
    assert_eq!(report.occupations[0].start, "2025-12-31T23:59:42Z");
    assert_eq!(report.marks_added, ["PM1", "PM2"]);

    let loaded = load(&store).unwrap();
    assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
    assert_eq!(loaded.occupations.len(), 2);
    let pm2 = &loaded.occupations[1];
    assert_eq!(pm2.rinex, ["raw/day1a.rnx", "raw/day1b.rnx"]);
    assert_eq!(
        (pm2.height.value, pm2.antenna.igs.as_str()),
        (1.5, "LEIAS10")
    );
    assert_eq!(
        (pm2.receiver.make.as_str(), pm2.receiver.model.as_str()),
        ("LEICA", "GR30")
    );
    let [lat, lon, _] = loaded.marks.marks[0].approx.unwrap();
    assert!(
        (lat + 37.8294).abs() < 1e-4 && (lon - 144.9753).abs() < 1e-4,
        "{lat} {lon}"
    );

    // PM1 again that afternoon: a second occupation, with the next letter.
    let later = vec![(
        "day1c.rnx".to_owned(),
        rinex("PM1", "LEIAS10", 1440, 1560, ""),
    )];
    let report = import_rinex(&mut store, &later, &options()).unwrap();
    assert_eq!(report.occupations[0].id, "2026-01-01-PM1-B");
    assert!(report.marks_added.is_empty());
}

#[test]
fn nothing_is_written_when_something_is_wrong() {
    let mut store = project();
    let good = ("a.rnx".to_owned(), rinex("PM1", "LEIAS10", 0, 10, ""));
    import_rinex(&mut store, std::slice::from_ref(&good), &options()).unwrap();
    let before = store.clone();

    // The same file again.
    let err = import_rinex(&mut store, std::slice::from_ref(&good), &options()).unwrap_err();
    assert!(matches!(err, IngestError::Store(_)), "{err}");
    // A file without an antenna, beside a good one.
    let files = vec![
        ("b.rnx".to_owned(), rinex("PM2", "LEIAS10", 0, 10, "")),
        ("c.rnx".to_owned(), rinex("PM3", "", 0, 10, "")),
    ];
    let err = import_rinex(&mut store, &files, &options()).unwrap_err();
    assert!(
        err.to_string().contains("doesn't name the antenna"),
        "{err}"
    );
    // A marker name that can't be a mark id, unless one is given.
    let odd = vec![("d.rnx".to_owned(), rinex("PM 4", "LEIAS10", 0, 10, ""))];
    let err = import_rinex(&mut store, &odd, &options()).unwrap_err();
    assert!(
        err.to_string().contains("give the mark with --mark"),
        "{err}"
    );
    // Not RINEX.
    let junk = vec![("e.rnx".to_owned(), b"hello\n".to_vec())];
    let err = import_rinex(&mut store, &junk, &options()).unwrap_err();
    assert!(err.to_string().starts_with("e.rnx: line 1"), "{err}");
    assert_eq!(store, before);

    let named = IngestOptions {
        mark: Some("PM4".into()),
        ..options()
    };
    let report = import_rinex(&mut store, &odd, &named).unwrap();
    assert_eq!(report.occupations[0].mark, "PM4");
}
