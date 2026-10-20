//! Projects: `project.toml` round trips, runs from in-memory files (as the
//! web page runs them), settings and edits, the summary and the HTML report.
//! The command line's own tests cover the project folder.

#![allow(clippy::unwrap_used)]

use gnss_api::project::{self, AdjustmentSettings, EditRecord, Format, Import, Project};
use gnss_datum::FrameId;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const STATIONS: &[u8] = include_bytes!("../../formats/tests/data/dynaml/gnss-networkstn.xml");
const MEASUREMENTS: &[u8] = include_bytes!("../../formats/tests/data/dynaml/gnss-networkmsr.xml");

fn import(file: &str, format: Format) -> Import {
    Import {
        file: format!("raw/{file}"),
        format,
        frame: None,
        epoch: None,
        base: None,
        rover: None,
    }
}

fn sample() -> Project {
    let mut project = Project::new(
        "Sample <network> & co",
        AdjustmentSettings::new(FrameId::Gda2020, None),
    );
    project
        .imports
        .push(import("gnss-networkstn.xml", Format::Dynaml));
    project
        .imports
        .push(import("gnss-networkmsr.xml", Format::Dynaml));
    project
}

fn files(import: &Import) -> Result<Vec<u8>, String> {
    match import.name() {
        "gnss-networkstn.xml" => Ok(STATIONS.to_vec()),
        "gnss-networkmsr.xml" => Ok(MEASUREMENTS.to_vec()),
        other => Err(format!("no file {other}")),
    }
}

#[test]
fn project_files_round_trip() {
    let mut project = sample();
    project.edits.push(EditRecord {
        observation: "gnss-networkmsr.xml:435".into(),
        action: "reweight".into(),
        factor: Some(4.0),
        reason: "multipath at the base".into(),
    });
    let text = project.to_toml().unwrap();
    assert!(text.contains("[[import]]\nfile = \"raw/gnss-networkstn.xml\"\nformat = \"dynaml\""));
    assert_eq!(Project::from_toml(&text).unwrap(), project);

    // Settings left out take their defaults; unknown ones are refused, as
    // are schemas this version doesn't know.
    let minimal = "schema = 1\nname = \"x\"\n[adjustment]\nframe = \"ITRF2014\"\nepoch = 2026.5\n";
    let read = Project::from_toml(minimal).unwrap();
    assert_eq!(read.adjustment.datum, "constrained");
    assert_eq!(read.adjustment.alpha0, 0.001);
    assert!(read.imports.is_empty());
    let err = Project::from_toml(&minimal.replace("2026.5", "2026.5\nalpha = 0.01")).unwrap_err();
    assert!(err.to_string().contains("unknown field `alpha`"), "{err}");
    let err = Project::from_toml(&minimal.replace("schema = 1", "schema = 2")).unwrap_err();
    assert!(err.to_string().contains("schema 2"), "{err}");

    // A dynamic frame needs its epoch.
    let err = Project::from_toml(&minimal.replace("epoch = 2026.5\n", ""))
        .unwrap()
        .reference()
        .unwrap_err();
    assert!(err.to_string().contains("the adjustment's frame"), "{err}");
    for name in [
        "dynaml",
        "starnet",
        "rtklib",
        "csv-vectors",
        "csv-control",
        "sinex",
    ] {
        assert_eq!(Format::parse(name).unwrap().name(), name);
    }
}

#[test]
fn runs_from_bytes_and_summarises() {
    let project = sample();
    let run = project::run(&project, &mut files).unwrap();
    let summary = project::summary(&project, &run);
    assert_eq!(summary.marks, 43);
    assert_eq!(summary.observations, 131);
    assert_eq!(summary.degrees_of_freedom, 288);
    // DynAdjust's own result for its sample: χ² 336.64 on 288 degrees of
    // freedom.
    assert!((summary.global_test.as_ref().unwrap().statistic - 336.64).abs() < 0.005);
    assert_eq!(summary.suspects.len(), 12);
    assert!(!summary.passed);
    assert_eq!(run.files.len(), 2);
    assert_eq!(run.files[1].bytes, MEASUREMENTS.len());
    assert_eq!(run.files[1].sha256, gnss_store::sha256_hex(MEASUREMENTS));

    // A file that can't be had names itself.
    let err = project::run(&project, &mut |_| Err("gone".into())).unwrap_err();
    assert_eq!(err.to_string(), "raw/gnss-networkstn.xml: gone");
}

#[test]
fn settings_and_edits_take_effect() {
    // Holding one mark, minimally.
    let mut project = sample();
    project.adjustment.datum = "fixed:BEEC".into();
    let run = project::run(&project, &mut files).unwrap();
    assert_eq!(
        project::summary(&project, &run).degrees_of_freedom,
        288 - 18 + 3
    );
    project.adjustment.datum = "fixed:".into();
    assert!(project::run(&project, &mut files).is_err());

    // A re-weighting takes effect; an edit without a reason or to nothing
    // is refused.
    let mut project = sample();
    project.edits.push(EditRecord {
        observation: "gnss-networkmsr.xml:435".into(),
        action: "reweight".into(),
        factor: Some(9.0),
        reason: "multipath".into(),
    });
    let run = project::run(&project, &mut files).unwrap();
    let summary = project::summary(&project, &run);
    assert!(
        summary
            .suspects
            .iter()
            .all(|s| s.observation != "gnss-networkmsr.xml:435")
    );
    project.edits[0].reason = " ".into();
    let err = project::run(&project, &mut files).unwrap_err();
    assert!(err.to_string().contains("needs a reason"), "{err}");
    project.edits[0].reason = "x".into();
    project.edits[0].factor = None;
    assert!(project::run(&project, &mut files).is_err());
    project.edits[0] = EditRecord {
        observation: "nope:1".into(),
        action: "exclude".into(),
        factor: None,
        reason: "x".into(),
    };
    let err = project::run(&project, &mut files).unwrap_err();
    assert!(err.to_string().contains("nope:1"), "{err}");

    // Uncertainties scaled by the a posteriori variance factor grow with
    // it (σ̂₀² is 1.17 here).
    let mut project = sample();
    let a_priori = project::run(&project, &mut files).unwrap();
    project.adjustment.uncertainty = "a-posteriori".into();
    let a_posteriori = project::run(&project, &mut files).unwrap();
    let mark = a_priori.built.network.mark("BEEC").unwrap();
    let pu = |run: &project::Run| {
        run.adjustment
            .positional_uncertainty(mark, run.scaling)
            .unwrap()
            .horizontal_95
    };
    let ratio = pu(&a_posteriori) / pu(&a_priori);
    assert!((ratio * ratio - 1.1689).abs() < 1e-3, "{ratio}");
}

#[test]
fn the_report_is_one_page_with_everything_escaped() {
    let project = sample();
    let run = project::run(&project, &mut files).unwrap();
    let html = project::report(&project, &run, "2026-10-20 08:00 UTC");
    assert_eq!(
        html,
        project::report(&project, &run, "2026-10-20 08:00 UTC")
    );
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("<h1>Sample &lt;network&gt; &amp; co</h1>"));
    assert!(!html.contains("<network>"));
    // Self-contained: no scripts, styles or images from anywhere else.
    for outside in ["<script", "src=", "<link", "http://", "https://"] {
        assert!(!html.contains(outside), "{outside}");
    }
    for section in [
        "summary",
        "tests",
        "suspects",
        "coordinates",
        "relative",
        "closures",
        "observations",
        "provenance",
    ] {
        assert!(
            html.contains(&format!("<section id=\"{section}\">")),
            "{section}"
        );
    }
    assert_eq!(
        html.matches("<table>").count(),
        html.matches("</table>").count()
    );
    assert!(html.contains("Constrained by 6 observed positions (weighted control)."));
    assert!(html.contains("The global test passed; 12 tests of observations failed."));
    assert!(html.contains(&gnss_store::sha256_hex(MEASUREMENTS)));
    // BEEC, with its MGA zone 55 coordinates.
    assert!(html.contains("<tr><td>BEEC</td><td>weighted</td>"));
    assert!(html.contains("<td class=\"n\">55</td><td class=\"n\">469288.6177</td>"));
}
