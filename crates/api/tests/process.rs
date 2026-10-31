//! Processing baselines (`gnss process`): what the engine is given for each,
//! and what's kept and recorded from what it gives back. The engine here is
//! a stand-in that checks its inputs and answers with a solution file; the
//! real one runs in the CLI's tests and CI.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use gnss_api::ingest::{IngestOptions, import_rinex};
use gnss_api::process::{Engine, EngineRun, process};
use gnss_api::project::{AdjustmentSettings, Format, Project};
use gnss_datum::FrameId;
use gnss_fetch::{Cache, Entry};
use gnss_model::{DataFile, DataFiles};
use gnss_store::{MemoryStore, ProjectPath, Store};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;
#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const NAV: &[u8] =
    include_bytes!("../../rinex-io/tests/data/nav/HOB200AUS_R_20251660000_excerpt_MN.rnx");
const OLD_NAV: &[u8] = include_bytes!("../../rinex-io/tests/data/nav/mobs0050_excerpt.20n");

fn record(content: &str, label: &str) -> String {
    format!("{content:<60}{label}\n")
}

/// A mark observed from 00:00 to 00:29:30 GPS time on 2025-06-15, the day
/// of the navigation excerpt.
fn rinex(mark: &str, x: f64) -> Vec<u8> {
    let mut text = [
        record(
            "     3.04           OBSERVATION DATA    M",
            "RINEX VERSION / TYPE",
        ),
        record(mark, "MARKER NAME"),
        record(
            "1234                LEICA GR30          4.82",
            "REC # / TYPE / VERS",
        ),
        record("5678                LEIAS10         NONE", "ANT # / TYPE"),
        record(
            &format!("{x:14.4}{:14.4}{:14.4}", 4212836.0, -2545105.0),
            "APPROX POSITION XYZ",
        ),
        record(
            &format!("{:14.4}{:14.4}{:14.4}", 1.5, 0.0, 0.0),
            "ANTENNA: DELTA H/E/N",
        ),
        record("G    1 C1C", "SYS / # / OBS TYPES"),
        record(
            "  2025     6    15     0     0    0.0000000     GPS",
            "TIME OF FIRST OBS",
        ),
        record("", "END OF HEADER"),
    ]
    .concat();
    for k in 0..60 {
        let t = 30 * k;
        text.push_str(&format!(
            "> 2025 06 15 00 {:02} {:10.7}  0  1\nG01  20000000.000\n",
            t / 60,
            f64::from(t % 60)
        ));
    }
    text.into_bytes()
}

/// Two field occupations, and data.toml with a CORS station's file, a
/// navigation file and ANTEX, in the cache.
fn project(nav: &[u8]) -> (MemoryStore, Cache<MemoryStore>) {
    let mut store = MemoryStore::new();
    let settings = Project::new("Test", AdjustmentSettings::new(FrameId::Gda2020, None));
    store
        .write(
            &ProjectPath::new("project.toml").unwrap(),
            settings.to_toml().unwrap().as_bytes(),
        )
        .unwrap();
    let files = vec![
        ("pm1.rnx".to_owned(), rinex("PM1", -4052052.0)),
        ("pm2.rnx".to_owned(), rinex("PM2", -4053052.0)),
    ];
    import_rinex(&mut store, &files, &IngestOptions::default()).unwrap();

    let mut cache = Cache::new(MemoryStore::new());
    let mut data = DataFiles::new();
    for (purpose, name, bytes) in [
        (
            "CORS observations, 2025-06-14 23:55:00 to 2025-06-15 00:35:00 GPS"
                .replace("CORS", "HOB2"),
            "HOB2.rnx",
            rinex("HOB2", -4062052.0),
        ),
        (
            "navigation data, 2025-06-15".into(),
            "nav.rnx",
            nav.to_vec(),
        ),
        (
            gnss_fetch::plan::ANTEX.into(),
            gnss_fetch::plan::ANTEX,
            b"ANTEX".to_vec(),
        ),
    ] {
        let entry = cache
            .put(
                Entry {
                    source: "test".into(),
                    name: name.into(),
                    sha256: String::new(),
                    bytes: 0,
                    url: format!("https://x.example/{name}"),
                    fetched: "2026-10-31T00:00:00Z".into(),
                    licence: "test".into(),
                },
                &bytes,
            )
            .unwrap();
        data.files.push(DataFile {
            purpose,
            name: entry.name,
            source: entry.source,
            sha256: entry.sha256,
            bytes: entry.bytes,
            url: entry.url,
            fetched: entry.fetched,
            licence: entry.licence,
            note: None,
            site_log: None,
        });
    }
    store
        .write(
            &ProjectPath::new("data.toml").unwrap(),
            data.to_toml().as_bytes(),
        )
        .unwrap();
    (store, cache)
}

/// Answers each run with a solution of the given quality, and keeps what
/// it was given.
struct Stand {
    q: u32,
    runs: Vec<(Vec<String>, Vec<String>, String)>,
}

impl Engine for Stand {
    fn run(
        &mut self,
        inputs: &[(String, Vec<u8>)],
        args: &[String],
        output: &str,
    ) -> Result<EngineRun, String> {
        let names = inputs.iter().map(|(n, _)| n.clone()).collect();
        let config = inputs
            .iter()
            .find(|(n, _)| n == "run.conf")
            .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();
        self.runs.push((names, args.to_vec(), config));
        // Every observation file given is the overlap's: the occupations
        // were imported from 00:00 GPS (23:59:42 UTC).
        for (name, bytes) in inputs.iter().filter(|(n, _)| n.ends_with(".obs")) {
            let text = String::from_utf8_lossy(bytes);
            let first = text
                .lines()
                .find(|l| l.starts_with('>'))
                .unwrap_or_default();
            assert!(
                first.starts_with("> 2025 06 15 00 00  0.0000000"),
                "{name}: {first}"
            );
        }
        assert_eq!(output, "out.pos");
        let pos = format!(
            "% ref pos   :  -4062052.0000   4212836.0000  -2545105.0000\n\
             2025/06/15 00:00:30.000  -4052052.0010   4212836.0020  -2545105.0030   {}   8   0.0010   0.0010   0.0010   0.0000   0.0000   0.0000   0.00    5.2\n",
            self.q
        );
        Ok(EngineRun {
            status: 0,
            stderr: String::new(),
            output: Some(pos.into_bytes()),
        })
    }
}

#[test]
fn every_overlapping_pair_is_processed_and_fixed_solutions_are_recorded() {
    let (mut store, cache) = project(NAV);
    let mut engine = Stand {
        q: 1,
        runs: Vec::new(),
    };
    let report = process(&mut store, &cache, &mut engine, 600.0).unwrap();
    // PM1 and PM2 with each other, and each with HOB2.
    let pairs: Vec<(&str, &str)> = report
        .baselines
        .iter()
        .map(|b| (b.base.as_str(), b.rover.as_str()))
        .collect();
    assert_eq!(pairs, [("PM1", "PM2"), ("HOB2", "PM1"), ("HOB2", "PM2")]);
    assert!(
        report.baselines.iter().all(|b| b.used),
        "{:?}",
        report.baselines
    );
    let first = &report.baselines[0];
    assert_eq!(first.quality.as_deref(), Some("fixed"));
    assert_eq!((first.satellites, first.ratio), (Some(8), Some(5.2)));
    assert!((first.length.unwrap() - 10_000.0).abs() < 0.01);

    // What the engine was given: the configuration, the observations, the
    // navigation file and ANTEX, uncompressed under its own name.
    let (names, args, config) = &engine.runs[0];
    assert_eq!(
        names,
        &[
            "run.conf",
            "rover.obs",
            "base.obs",
            "igs20_2434.atx",
            "nav0.rnx"
        ]
    );
    assert_eq!(
        args[..7],
        [
            "rnx2rtkp",
            "-k",
            "run.conf",
            "-o",
            "out.pos",
            "rover.obs",
            "base.obs"
        ]
    );
    assert_eq!(args.last().map(String::as_str), Some("nav0.rnx"));
    assert!(config.contains("file-satantfile    =igs20_2434.atx"));
    assert!(config.contains("pos1-posmode       =static"));

    // Kept, and recorded for the adjustment with the marks' names.
    let project = Project::from_toml(
        &String::from_utf8(
            store
                .read(&ProjectPath::new("project.toml").unwrap())
                .unwrap()
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let imports: Vec<_> = project
        .imports
        .iter()
        .filter(|i| i.format == Format::Rtklib)
        .collect();
    assert_eq!(imports.len(), 3);
    assert_eq!(imports[1].base.as_deref(), Some("HOB2"));
    assert_eq!(imports[1].frame.as_deref(), Some("WGS 84"));
    assert!(
        store
            .read(&ProjectPath::new(imports[0].file.as_str()).unwrap())
            .unwrap()
            .is_some()
    );

    // Processed again with float solutions: kept, reported, not recorded,
    // and the last processing's records go.
    let mut engine = Stand {
        q: 2,
        runs: Vec::new(),
    };
    let report = process(&mut store, &cache, &mut engine, 600.0).unwrap();
    assert!(report.baselines.iter().all(|b| !b.used && b.file.is_some()));
    assert!(
        report.baselines[0]
            .problem
            .as_deref()
            .unwrap()
            .contains("float")
    );
    let project = Project::from_toml(
        &String::from_utf8(
            store
                .read(&ProjectPath::new("project.toml").unwrap())
                .unwrap()
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(project.imports.iter().all(|i| i.format != Format::Rtklib));
}

#[test]
fn navigation_data_that_misses_a_constellation_stops_a_baseline() {
    // Navigation data from 2020: nothing covers 2025.
    let (mut store, cache) = project(OLD_NAV);
    let mut engine = Stand {
        q: 1,
        runs: Vec::new(),
    };
    let report = process(&mut store, &cache, &mut engine, 600.0).unwrap();
    assert!(engine.runs.is_empty());
    let problem = report.baselines[0].problem.as_deref().unwrap();
    assert!(problem.contains("doesn't cover G, E, C"), "{problem}");
}

#[test]
fn without_navigation_data_it_says_to_fetch() {
    let (mut store, cache) = project(NAV);
    store
        .write(
            &ProjectPath::new("data.toml").unwrap(),
            DataFiles::new().to_toml().as_bytes(),
        )
        .unwrap();
    let err = process(
        &mut store,
        &cache,
        &mut Stand {
            q: 1,
            runs: Vec::new(),
        },
        600.0,
    )
    .unwrap_err();
    assert!(err.to_string().contains("run gnss fetch"), "{err}");
}
