//! A second opinion on a real Trimble Business Center export: 31 vectors
//! around Bright, Victoria, observed in December 2019
//! (`crates/formats/tests/data/tdef`), adjusted in GDA2020 on the five CORS
//! at the GDA2020 coordinates keyed into TBC.
//!
//! The adjustment fails its global test, and the two vectors into mark
//! 324900700 fail their tests. Leaving those two out, as a person might after
//! checking that mark's set-up, gives an adjustment that passes. So the TDEF
//! reader carries real vectors and covariances through, and the second
//! opinion finds what it should.

#![allow(clippy::unwrap_used)]

use gnss_api::project::{self, AdjustmentSettings, EditRecord, Format, Import, Project};
use gnss_datum::FrameId;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const TDEF: &[u8] = include_bytes!("../../formats/tests/data/tdef/tbc-bright-2019.asc");

/// The CORS, held at the GDA2020 coordinates keyed into TBC.
const CONTROL: &str = "\
name,latitude,longitude,height,constraint,frame
BEEC,-36.346434060,146.657743009,442.9339,fixed,GDA2020
MYRT,-36.557956214,146.722203452,227.1787,fixed,GDA2020
WNGA,-36.354541422,146.323064315,162.6992,fixed,GDA2020
HOTH,-36.982063056,147.141848352,1773.9196,fixed,GDA2020
MIMI,-36.532210239,147.373232453,290.5791,fixed,GDA2020
";

fn bright(frame: &str) -> Project {
    let mut project = Project::new("Bright", AdjustmentSettings::new(FrameId::Gda2020, None));
    project.imports = vec![
        Import {
            file: "raw/tbc-bright-2019.asc".into(),
            format: Format::Tdef,
            frame: Some(frame.into()),
            epoch: None,
            base: None,
            rover: None,
        },
        Import {
            file: "raw/cors.csv".into(),
            format: Format::CsvControl,
            frame: None,
            epoch: None,
            base: None,
            rover: None,
        },
    ];
    project
}

fn files(import: &Import) -> Result<Vec<u8>, String> {
    Ok(match import.name() {
        "tbc-bright-2019.asc" => TDEF.to_vec(),
        _ => CONTROL.as_bytes().to_vec(),
    })
}

#[test]
fn a_tbc_export_gets_a_second_opinion() {
    assert_eq!(
        project::guess_format("vectors.asc", TDEF),
        Some(Format::Tdef)
    );

    // TBC computes vectors in the global frame at the survey's epoch.
    let mut project = bright("ITRF2014");
    let run = project::run(&project, &mut files).unwrap();
    let summary = project::summary(&project, &run);
    assert_eq!(summary.marks, 11);
    assert_eq!(summary.observations, 31);
    assert_eq!(summary.degrees_of_freedom, 75);
    let global = summary.global_test.as_ref().unwrap();
    assert!(!global.passed);
    assert!((summary.variance_factor.unwrap() - 1.971).abs() < 5e-4);
    let suspects: Vec<&str> = summary
        .suspects
        .iter()
        .map(|s| s.observation.as_str())
        .collect();
    assert_eq!(
        suspects,
        [
            "tbc-bright-2019.asc:67",
            "tbc-bright-2019.asc:67",
            "tbc-bright-2019.asc:58",
            "tbc-bright-2019.asc:58",
            "tbc-bright-2019.asc:58"
        ]
    );
    assert!(
        summary
            .suspects
            .iter()
            .all(|s| s.label.ends_with("to 324900700"))
    );

    // The vectors' epoch hardly matters this close to 2020.0: taking them as
    // GDA2020 changes the global test by 0.02.
    let gda = bright("GDA2020");
    let run = project::run(&gda, &mut files).unwrap();
    let statistic = project::summary(&gda, &run).global_test.unwrap().statistic;
    assert!((statistic - global.statistic).abs() < 0.05, "{statistic}");

    // Without the two vectors into 324900700, it passes.
    for line in [67, 58] {
        project.edits.push(EditRecord {
            observation: format!("tbc-bright-2019.asc:{line}"),
            action: "exclude".into(),
            reason: "fails its tests; check 324900700's set-up".into(),
            factor: None,
        });
    }
    let run = project::run(&project, &mut files).unwrap();
    let summary = project::summary(&project, &run);
    assert!(summary.passed);
    assert_eq!(summary.degrees_of_freedom, 69);
    assert!((summary.variance_factor.unwrap() - 1.258).abs() < 5e-4);
}
