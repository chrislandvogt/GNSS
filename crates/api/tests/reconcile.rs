//! Headers against occupations: differences found, fixed with overlays,
//! and corrected files written.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use gnss_api::folder::load;
use gnss_api::ingest::{IngestOptions, import_rinex};
use gnss_api::project::{AdjustmentSettings, Project};
use gnss_api::reconcile::{corrected_rinex, fix, reconcile};
use gnss_datum::FrameId;
use gnss_model::Timestamp;
use gnss_rinex_io::read_obs;
use gnss_store::{MemoryStore, ProjectPath, Store};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;
#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

fn record(content: &str, label: &str) -> String {
    format!("{content:<60}{label}\n")
}

/// PM1 from 00:00 to 00:59:30 GPS time on 2026-01-01, as the receiver
/// wrote it: the default antenna and no height.
fn rinex() -> Vec<u8> {
    let mut text = [
        record(
            "     3.04           OBSERVATION DATA    G",
            "RINEX VERSION / TYPE",
        ),
        record("PM1", "MARKER NAME"),
        record(
            "1234                LEICA GR30          4.82",
            "REC # / TYPE / VERS",
        ),
        record("5678                LEIAS10         NONE", "ANT # / TYPE"),
        record(
            &format!("{:14.4}{:14.4}{:14.4}", 0.0, 0.0, 0.0),
            "ANTENNA: DELTA H/E/N",
        ),
        record("G    1 C1C", "SYS / # / OBS TYPES"),
        record(
            "  2026     1     1     0     0    0.0000000     GPS",
            "TIME OF FIRST OBS",
        ),
        record(
            "  2026     1     1     0    59   30.0000000     GPS",
            "TIME OF LAST OBS",
        ),
        record("", "END OF HEADER"),
    ]
    .concat();
    for k in 0..120 {
        let t = 30 * k;
        text.push_str(&format!(
            "> 2026 01 01 00 {:02} {:10.7}  0  1\nG01  20000000.000\n",
            t / 60,
            f64::from(t % 60)
        ));
    }
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
    let files = vec![("pm1.rnx".to_owned(), rinex())];
    import_rinex(&mut store, &files, &IngestOptions::default()).unwrap();
    store
}

fn occupation_path() -> ProjectPath {
    ProjectPath::new("occupations/2026-01-01-PM1-A.toml").unwrap()
}

#[test]
fn an_imported_file_agrees_with_its_occupation_until_the_field_log_says_otherwise() {
    let mut store = project();
    // As imported, the occupation is what the header says.
    assert!(reconcile(&store).unwrap().is_empty());

    // The field log: an R12i with a TRM59800.00 under a SCIS radome, 1.532 m
    // to the ARP, a different antenna serial, and observed until 01:10.
    let text = String::from_utf8(store.read(&occupation_path()).unwrap().unwrap()).unwrap();
    let edited = text
        .replace("igs = \"LEIAS10\"", "igs = \"TRM59800.00\"")
        .replace("radome = \"NONE\"", "radome = \"SCIS\"")
        .replace("serial = \"5678\"", "serial = \"9999\"")
        .replace("value = 0.0", "value = 1.532")
        .replace(
            "end = \"2026-01-01T00:59:12Z\"",
            "end = \"2026-01-01T01:10:00Z\"",
        );
    store.write(&occupation_path(), edited.as_bytes()).unwrap();
    let findings = reconcile(&store).unwrap();
    let got: Vec<(&str, bool)> = findings
        .iter()
        .map(|f| (f.what.as_str(), f.fixable))
        .collect();
    assert_eq!(
        got,
        [
            ("antenna", true),
            ("radome", true),
            ("antenna serial", false),
            ("antenna height", true),
            ("span", false),
        ]
    );
    assert_eq!(findings[3].header, "0.0000 m");
    assert_eq!(findings[3].occupation_value, "1.5320 m");

    // Overlays fix what they can, and the project still loads cleanly.
    let now = Timestamp::parse("2026-01-02T03:00:00Z").unwrap();
    let ids = fix(&mut store, "CL", now).unwrap();
    assert_eq!(ids, ["2026-01-02-pm1-rnx-header"]);
    let loaded = load(&store).unwrap();
    assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
    assert!(
        loaded.overlays[0]
            .reason
            .contains("antenna TRM59800.00 (header LEIAS10)")
    );
    let left: Vec<String> = reconcile(&store)
        .unwrap()
        .into_iter()
        .map(|f| f.what)
        .collect();
    assert_eq!(left, ["antenna serial", "span"]);
    // Fixing again finds nothing more to fix.
    assert!(fix(&mut store, "CL", now).unwrap().is_empty());

    // The corrected file: the header as the overlay says, with a comment.
    let out = corrected_rinex(&store, "raw/pm1.rnx").unwrap();
    let r = read_obs(&out[..]).unwrap();
    let h = r.header();
    assert_eq!(h.antenna.kind, "TRM59800.00");
    assert_eq!(h.antenna.radome.as_deref(), Some("SCIS"));
    assert_eq!(h.antenna_delta, Some([1.532, 0.0, 0.0]));
    assert!(h.comments[0].starts_with("Corrected by overlay 2026-01-02-pm1-rnx-header (CL)"));
    // The raw file is as it was.
    assert_eq!(
        store
            .read_raw(&ProjectPath::new("raw/pm1.rnx").unwrap())
            .unwrap(),
        rinex()
    );
}
