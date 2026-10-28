//! The command line end to end: DynAdjust's sample network from import to
//! exports, the outlier workflow to a passing adjustment, an RTKLIB baseline
//! with no redundancy, and the errors people will meet.

#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const DATA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../crates/formats/tests/data"
);

fn gnss(project: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gnss"))
        .arg("--project")
        .arg(project)
        .args(args)
        .output()
        .unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn json(o: &Output) -> serde_json::Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", stdout(o)))
}

/// Runs a command, checking its exit code.
fn ok(project: &Path, args: &[&str], code: i32) -> Output {
    let o = gnss(project, args);
    assert_eq!(
        o.status.code(),
        Some(code),
        "gnss {args:?}\nstdout: {}\nstderr: {}",
        stdout(&o),
        stderr(&o)
    );
    o
}

fn path(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// A project with DynAdjust's sample network imported.
fn sample(root: &Path) -> PathBuf {
    let project = root.join("sample");
    ok(root, &["init", path(&project), "--name", "Sample"], 0);
    // The format is guessed when it isn't given, and the guess is shown.
    let o = ok(
        &project,
        &[
            "import",
            "control",
            &format!("{DATA}/dynaml/gnss-networkstn.xml"),
        ],
        0,
    );
    assert!(
        stdout(&o).contains("as dynaml (guessed from the file; give --format to choose)"),
        "{}",
        stdout(&o)
    );
    let o = ok(
        &project,
        &[
            "--json",
            "import",
            "vectors",
            &format!("{DATA}/dynaml/gnss-networkmsr.xml"),
            "--format",
            "dynaml",
        ],
        0,
    );
    let imported = json(&o);
    assert_eq!(imported["imported"], "raw/gnss-networkmsr.xml");
    assert_eq!(imported["format"], "dynaml");
    assert_eq!(imported["guessed"], false);
    assert_eq!(imported["measurements"], 131);
    project
}

#[test]
fn sample_network_from_import_to_exports() {
    let root = tempfile::tempdir().unwrap();
    let project = sample(root.path());
    for entry in [
        "project.toml",
        "raw.lock",
        "raw/gnss-networkstn.xml",
        "results",
    ] {
        assert!(project.join(entry).exists(), "{entry}");
    }

    // Twelve tests fail, so the exit code says so.
    let o = ok(&project, &["--json", "adjust"], 2);
    let summary = json(&o);
    assert_eq!(summary["marks"], 43);
    assert_eq!(summary["degrees_of_freedom"], 288);
    assert_eq!(summary["global_test"]["passed"], true);
    assert_eq!(summary["passed"], false);
    assert_eq!(summary["suspects"].as_array().unwrap().len(), 12);
    assert_eq!(
        summary["suspects"][0]["observation"],
        "gnss-networkmsr.xml:435"
    );
    let text = stdout(&ok(&project, &["adjust"], 2));
    assert!(text.contains("Global test passed"), "{text}");
    assert!(
        text.contains(
            "gnss-networkmsr.xml:435 (324900360 to 222702940): vector test 28.35 > 12.63"
        ),
        "{text}"
    );

    // The report.
    ok(&project, &["report"], 2);
    let report = fs::read_to_string(project.join("results/report.html")).unwrap();
    assert!(report.contains("<h1>Sample</h1>"));
    assert!(report.contains("gnss-networkmsr.xml:435"));
    assert!(report.contains("Constrained by 6 observed positions (weighted control)."));

    // Every export.
    for format in ["gpkg", "csv", "landxml", "dynaml", "starnet"] {
        ok(&project, &["export", "--format", format], 0);
    }
    let gpkg = fs::read(project.join("results/results.gpkg")).unwrap();
    assert_eq!(&gpkg[68..72], b"GPKG");
    for file in [
        "marks.csv",
        "observations.csv",
        "control.xml",
        "adjusted-stn.xml",
        "adjusted-msr.xml",
        "adjusted.gps",
    ] {
        assert!(project.join("results").join(file).is_file(), "{file}");
    }
    let other = root.path().join("elsewhere.gpkg");
    ok(
        &project,
        &["export", "--format", "gpkg", "--output", path(&other)],
        0,
    );
    assert_eq!(fs::read(&other).unwrap().len(), gpkg.len());

    // Minimally constrained, for this run only: the six observed positions
    // drop out.
    let minimal = json(&gnss(&project, &["--json", "adjust", "--minimal"]));
    assert_eq!(minimal["degrees_of_freedom"], 288 - 18 + 3);
    assert!(
        fs::read_to_string(project.join("project.toml"))
            .unwrap()
            .contains("datum = \"constrained\"")
    );
}

#[test]
fn excluding_the_worst_one_at_a_time_passes() {
    let root = tempfile::tempdir().unwrap();
    let project = sample(root.path());
    let before = fs::read_to_string(project.join("project.toml")).unwrap();

    // Nothing to exclude, or no reason: refused, and the project's unchanged.
    let o = ok(&project, &["exclude", "nope:1", "--reason", "typo"], 1);
    assert!(
        stderr(&o).contains("there's no observation nope:1"),
        "{}",
        stderr(&o)
    );
    ok(
        &project,
        &["exclude", "gnss-networkmsr.xml:435", "--reason", " "],
        1,
    );
    assert_eq!(
        fs::read_to_string(project.join("project.toml")).unwrap(),
        before
    );

    // The outlier workflow: exclude the worst, adjust again, repeat.
    let mut excluded = Vec::new();
    loop {
        let o = gnss(&project, &["--json", "adjust"]);
        let summary = json(&o);
        if summary["passed"] == true {
            assert_eq!(o.status.code(), Some(0));
            break;
        }
        assert_eq!(o.status.code(), Some(2));
        let worst = summary["suspects"][0]["observation"]
            .as_str()
            .unwrap()
            .to_owned();
        ok(
            &project,
            &[
                "exclude",
                &worst,
                "--reason",
                "worst vector test; to re-observe",
            ],
            0,
        );
        excluded.push(worst);
        assert!(excluded.len() < 10, "{excluded:?}");
    }
    assert_eq!(
        excluded,
        [
            "gnss-networkmsr.xml:435",
            "gnss-networkmsr.xml:389",
            "gnss-networkmsr.xml:2715",
            "gnss-networkmsr.xml:2577",
            "gnss-networkmsr.xml:2508"
        ]
    );
    let o = ok(&project, &["exclude", &excluded[0], "--reason", "again"], 1);
    assert!(stderr(&o).contains("is already edited"), "{}", stderr(&o));

    // The report lists the edits with their reasons, and passes.
    ok(&project, &["report"], 0);
    let report = fs::read_to_string(project.join("results/report.html")).unwrap();
    assert!(report.contains("The global test passed and no observation is suspect."));
    assert_eq!(
        report.matches("worst vector test; to re-observe").count(),
        5
    );
}

#[test]
fn an_rtklib_baseline_without_redundancy_says_so() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("baseline");
    ok(root.path(), &["init", path(&project)], 0);
    let control = root.path().join("control.csv");
    fs::write(
        &control,
        "name,x,y,z,constraint\nMTHR,-4707913.7460,2599003.2450,-3418163.9530,fixed\n",
    )
    .unwrap();
    ok(
        &project,
        &[
            "import",
            "control",
            path(&control),
            "--format",
            "csv",
            "--frame",
            "GDA2020",
        ],
        0,
    );
    let o = ok(
        &project,
        &[
            "import",
            "vectors",
            &format!("{DATA}/rtklib/static-xyz.pos"),
            "--format",
            "rtklib",
            "--base",
            "MTHR",
            "--rover",
            "SNGO",
        ],
        0,
    );
    assert!(stdout(&o).contains("WGS 84 was assumed"), "{}", stdout(&o));

    let o = ok(&project, &["adjust"], 2);
    assert!(
        stdout(&o).contains("No redundancy, so nothing could be tested."),
        "{}",
        stdout(&o)
    );
    let summary = json(&gnss(&project, &["--json", "adjust"]));
    assert_eq!(summary["marks"], 2);
    assert_eq!(summary["degrees_of_freedom"], 0);
    ok(&project, &["report"], 2);
}

#[test]
fn raw_files_are_locked() {
    let root = tempfile::tempdir().unwrap();
    let project = sample(root.path());
    let raw = project.join("raw/gnss-networkmsr.xml");
    let mut bytes = fs::read(&raw).unwrap();
    bytes.extend_from_slice(b"\n");
    fs::write(&raw, bytes).unwrap();
    let o = ok(&project, &["adjust"], 1);
    assert!(
        stderr(&o).contains("raw/gnss-networkmsr.xml: it has changed since it was imported"),
        "{}",
        stderr(&o)
    );

    // Importing a file of the same name again is refused.
    let o = ok(
        &project,
        &[
            "import",
            "vectors",
            &format!("{DATA}/dynaml/gnss-networkmsr.xml"),
            "--format",
            "dynaml",
        ],
        1,
    );
    assert!(
        stderr(&o).contains("already in the project"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn errors_are_plain_and_change_nothing() {
    let root = tempfile::tempdir().unwrap();
    let here = root.path();

    // Not a project.
    let o = ok(here, &["adjust"], 1);
    assert!(stderr(&o).contains("isn't a project"), "{}", stderr(&o));
    let o = ok(here, &["--json", "adjust"], 1);
    assert!(
        json(&o)["error"]
            .as_str()
            .unwrap()
            .contains("isn't a project")
    );

    // A frame that doesn't exist, and a folder that isn't empty.
    let o = ok(
        here,
        &["init", path(&here.join("p")), "--frame", "GDA2021"],
        1,
    );
    assert!(stderr(&o).contains("Known frames"), "{}", stderr(&o));
    let project = here.join("p");
    ok(here, &["init", path(&project)], 0);
    let o = ok(here, &["init", path(&project)], 1);
    assert!(stderr(&o).contains("isn't empty"), "{}", stderr(&o));

    // Nothing imported yet.
    let o = ok(&project, &["adjust"], 1);
    assert!(
        stderr(&o).contains("nothing is imported yet"),
        "{}",
        stderr(&o)
    );

    // A file that can't be read as it's said to be leaves the project as it
    // was.
    let before = fs::read_to_string(project.join("project.toml")).unwrap();
    let vectors = here.join("vectors.gps");
    fs::write(&vectors, "G1 A-B 1.0 2.0 3.0\n").unwrap();
    let o = ok(
        &project,
        &["import", "vectors", path(&vectors), "--format", "starnet"],
        1,
    );
    assert!(
        stderr(&o).contains("give the import a frame"),
        "{}",
        stderr(&o)
    );

    // A file that looks like something this command doesn't import, or like
    // nothing, needs its format given.
    let o = ok(&project, &["import", "control", path(&vectors)], 1);
    assert!(
        stderr(&o).contains(
            "vectors.gps looks like STAR*NET GPS vectors, which this command doesn't import"
        ),
        "{}",
        stderr(&o)
    );
    let notes = here.join("notes.docx");
    fs::write(&notes, "x").unwrap();
    let o = ok(&project, &["import", "vectors", path(&notes)], 1);
    assert!(stderr(&o).contains("give --format"), "{}", stderr(&o));
    assert_eq!(
        fs::read_to_string(project.join("project.toml")).unwrap(),
        before
    );
    assert_eq!(fs::read_dir(project.join("raw")).unwrap().count(), 0);
    assert_eq!(fs::read_to_string(project.join("raw.lock")).unwrap(), "");
}

#[test]
fn qc_reads_plain_and_compressed_rinex_alike() {
    let dir = tempfile::tempdir().unwrap();
    let rinex = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/rinex-io/tests/data/rinex/SWTC00AUS_20251660000.rnx"
    );
    let crx = format!("{rinex}.crx");
    let plain = json(&ok(dir.path(), &["qc", "--json", rinex], 0));
    let compressed = json(&ok(dir.path(), &["qc", "--json", &crx], 0));
    assert_eq!(plain[0]["qc"], compressed[0]["qc"]);
    let qc = &plain[0]["qc"];
    assert_eq!(qc["marker"], "SWTC00AUS");
    assert_eq!(qc["antenna"], "LEIAS10");
    assert_eq!(qc["epochs"], 6);
    assert_eq!(qc["interval"], 30.0);
    assert!(qc["antenna_note"].is_null(), "{qc}");
    let gps = qc["systems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["system"] == "GPS")
        .unwrap();
    assert_eq!(gps["pair"], "L1/L2");

    let human = stdout(&ok(dir.path(), &["qc", rinex], 0));
    assert!(human.contains("LEICA GR30 with LEIAS10 NONE"), "{human}");
    assert!(
        human.contains("6 of 6 epochs (100.0%) every 30 s"),
        "{human}"
    );

    // With a navigation file, elevation coverage: here everything above
    // 10° that the file's ephemerides cover was tracked.
    let nav = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/rinex-io/tests/data/nav/HOB200AUS_R_20251660000_excerpt_MN.rnx"
    );
    let with_nav = json(&ok(dir.path(), &["qc", "--json", rinex, "--nav", nav], 0));
    let systems = with_nav[0]["qc"]["systems"].as_array().unwrap();
    let gps = systems.iter().find(|s| s["system"] == "GPS").unwrap();
    assert_eq!(gps["coverage"]["cutoff"], 10.0);
    assert_eq!(gps["coverage"]["expected"], 12);
    assert_eq!(gps["coverage"]["ratio"], 1.0);
    assert!(plain[0]["qc"]["systems"][0]["coverage"].is_null());
    let human = stdout(&ok(dir.path(), &["qc", rinex, "--nav", nav], 0));
    assert!(
        human.contains("above 10°: 12 of 12 satellite-epochs tracked (100.0%)"),
        "{human}"
    );
    let o = gnss(dir.path(), &["qc", rinex, "--nav", rinex]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("not a navigation file"),
        "{}",
        stderr(&o)
    );

    let not_rinex = dir.path().join("notes.txt");
    fs::write(&not_rinex, "hello\n").unwrap();
    let o = gnss(dir.path(), &["qc", path(&not_rinex)]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("notes.txt: line 1: a RINEX file starts"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn import_rinex_makes_occupations_and_marks() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("p");
    ok(root.path(), &["init", path(&project)], 0);
    let data = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/rinex-io/tests/data/rinex"
    );
    let swtc = format!("{data}/SWTC00AUS_20251660000.rnx.crx");
    let mobs = format!("{data}/MOBS0050.20o");
    let o = json(&ok(
        &project,
        &["import", "rinex", "--json", &swtc, &mobs],
        0,
    ));
    let ids: Vec<&str> = o["occupations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["2020-01-05-MOBS-A", "2025-06-15-SWTC00AUS-A"]);
    assert_eq!(o["marks_added"], serde_json::json!(["MOBS", "SWTC00AUS"]));
    let occupation =
        fs::read_to_string(project.join("occupations/2025-06-15-SWTC00AUS-A.toml")).unwrap();
    assert!(
        occupation.contains("rinex = [\"raw/SWTC00AUS_20251660000.rnx.crx\"]"),
        "{occupation}"
    );
    assert!(
        fs::read_to_string(project.join("raw.lock"))
            .unwrap()
            .contains("raw/MOBS0050.20o")
    );

    // Again: the raw file exists, and nothing changes.
    let o = gnss(&project, &["import", "rinex", &mobs]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("raw/MOBS0050.20o"), "{}", stderr(&o));
}

#[test]
fn reconcile_fixes_headers_and_exports_corrected_rinex() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("p");
    ok(root.path(), &["init", path(&project)], 0);
    let crx = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/rinex-io/tests/data/rinex/SWTC00AUS_20251660000.rnx.crx"
    );
    ok(&project, &["import", "rinex", crx], 0);
    let agree = stdout(&ok(&project, &["reconcile"], 0));
    assert!(agree.contains("Every header agrees"), "{agree}");

    // The field log says 1.6 m, not the header's 1.931 m.
    let occupation = project.join("occupations/2025-06-15-SWTC00AUS-A.toml");
    let text = fs::read_to_string(&occupation).unwrap();
    fs::write(&occupation, text.replace("value = 1.931", "value = 1.6")).unwrap();
    let differ = stdout(&ok(&project, &["reconcile"], 2));
    assert!(
        differ.contains("antenna height: the header says 1.9310 m, the occupation 1.6000 m"),
        "{differ}"
    );
    let o = gnss(&project, &["reconcile", "--fix"]);
    assert_ne!(o.status.code(), Some(0), "--fix needs --by");
    let fixed = json(&ok(
        &project,
        &["reconcile", "--fix", "--by", "CL", "--json"],
        0,
    ));
    assert_eq!(fixed["findings"], serde_json::json!([]));
    let id = fixed["overlays_written"][0].as_str().unwrap();
    assert!(project.join(format!("overlays/{id}.toml")).is_file());

    ok(&project, &["export-rinex"], 0);
    let out = fs::read_to_string(project.join("results/rinex/SWTC00AUS_20251660000.rnx")).unwrap();
    assert!(out.contains(
        "        1.6000        0.0000        0.0000                  ANTENNA: DELTA H/E/N"
    ));
    // The overlay is named in the comments, which run over several records.
    let comments: String = out
        .lines()
        .filter(|l| l.ends_with("COMMENT"))
        .map(|l| l[..60].to_owned())
        .collect();
    assert!(
        comments.contains(&format!("Corrected by overlay {id} (CL)")),
        "{comments}"
    );
}
