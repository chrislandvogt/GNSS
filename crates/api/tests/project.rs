//! Projects: `project.toml` round trips, runs from in-memory files (as the
//! web page runs them), settings and edits, the summary and the HTML report.
//! The command line's own tests cover the project folder.

#![allow(clippy::unwrap_used)]

use gnss_api::project::{
    self, AdjustmentSettings, EditRecord, ExportFormat, Format, Import, Project,
};
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
    // Without data from outside the project, no such sections.
    assert!(!html.contains("Data from outside the project") && !html.contains("Attribution"));
}

#[test]
fn the_report_lists_data_from_outside_and_its_attribution() {
    let project = sample();
    let run = project::run(&project, &mut files).unwrap();
    let mut data = gnss_model::DataFiles::new();
    data.merge(
        vec![gnss_model::DataFile {
            purpose: "SYDN observations, 2025-06-15 00:00:00 to 2025-06-15 02:00:00 GPS".into(),
            name: "SYDN00AUS_R_20251660000_01D_30S_MO.crx.gz".into(),
            source: "ga-archive".into(),
            sha256: "a".repeat(64),
            bytes: 10,
            url: "https://x.example/SYDN.crx.gz".into(),
            fetched: "2026-10-29T00:00:00Z".into(),
            licence: "CC BY 4.0, Geoscience Australia".into(),
            note: Some("rapid: finals aren't out yet".into()),
        }],
        ["CORS data from <GA>, CC BY 4.0.".to_owned()],
    );
    let html = project::report_with_data(&project, &run, Some(&data), "2026-10-20 08:00 UTC");
    assert!(html.contains("<h3>Data from outside the project</h3>"));
    assert!(html.contains("SYDN00AUS_R_20251660000_01D_30S_MO.crx.gz"));
    assert!(
        html.contains("rapid: finals aren&#39;t out yet")
            || html.contains("rapid: finals aren't out yet")
    );
    assert!(
        html.contains("<h3>Attribution</h3>\n<ul>\n<li>CORS data from &lt;GA&gt;, CC BY 4.0.</li>")
    );
    // Still self-contained: the URL is text, not something loaded or linked.
    for outside in ["<script", "src=", "<link", "href=\"https:"] {
        assert!(!html.contains(outside), "{outside}");
    }
}

/// Every export and the report of the sample network, hashed: the same
/// project gives the same bytes natively on every OS and in the browser
/// (D-024), so the web page's downloads match the CLI's. A deliberate change
/// to a format or the adjustment changes the hash: check the other tests,
/// then update it.
const EXPORTS_GOLDEN: &str = "fbd81dce4bafffdf56e8c909bef57d479fb9fb447d402adece0aab7869d7854c";

#[test]
fn exports_are_the_same_bytes_everywhere() {
    let project = sample();
    let run = project::run(&project, &mut files).unwrap();
    // LandXML and the report name the version that wrote them, and the
    // report the build; they're left out, so the hash holds across releases.
    let landxml_version = format!(
        "<Application name=\"GNSS\" version=\"{}\"/>",
        gnss_api::VERSION
    );
    let mut all = Vec::new();
    for (format, _) in ExportFormat::ALL {
        for file in project::export(&run, format, 1000.0, "2026-10-20T08:00:00.000Z").unwrap() {
            all.extend_from_slice(file.name.as_bytes());
            if format == ExportFormat::Landxml {
                let text = String::from_utf8(file.bytes).unwrap();
                assert!(text.contains(&landxml_version));
                all.extend_from_slice(text.replace(&landxml_version, "").as_bytes());
            } else {
                all.extend_from_slice(&file.bytes);
            }
        }
    }
    let report = project::report(&project, &run, "2026-10-20 08:00:00 UTC");
    let software = format!(
        "gnss {} ({})",
        gnss_api::VERSION,
        gnss_api::BuildTarget::current().as_str()
    );
    assert!(report.contains(&software));
    all.extend_from_slice(report.replace(&software, "").as_bytes());
    assert_eq!(
        gnss_store::sha256_hex(&all),
        EXPORTS_GOLDEN,
        "the exports' bytes changed: either a format or the adjustment changed on purpose \
         (check the other tests, then update EXPORTS_GOLDEN) or this platform differs (D-024)"
    );
}

#[test]
fn formats_are_guessed_from_contents_then_names() {
    let guess = project::guess_format;
    assert_eq!(guess("gnss-networkstn.xml", STATIONS), Some(Format::Dynaml));
    assert_eq!(guess("anything.txt", MEASUREMENTS), Some(Format::Dynaml));
    let rtklib = include_bytes!("../../formats/tests/data/rtklib/static-llh.pos");
    assert_eq!(guess("solution.txt", rtklib), Some(Format::Rtklib));
    let sinex = include_bytes!("../../formats/tests/data/sinex/TEST_ITRF05.SNX");
    assert_eq!(guess("auspos", sinex), Some(Format::Sinex));
    assert_eq!(guess("job.SNX", b""), Some(Format::Sinex));
    assert_eq!(
        guess("vectors.gps", b"G1 A-B 1 2 3\n"),
        Some(Format::Starnet)
    );
    assert_eq!(
        guess("vectors.csv", b"# exported\nFrom,To,dX (m),dY (m),dZ (m)\n"),
        Some(Format::CsvVectors)
    );
    assert_eq!(
        guess("control.csv", b"name;latitude;longitude;height\n"),
        Some(Format::CsvControl)
    );
    assert_eq!(guess("control.xml", b"<LandXML>"), None);
    assert_eq!(guess("photo.jpg", b"\xff\xd8\xff"), None);
}
