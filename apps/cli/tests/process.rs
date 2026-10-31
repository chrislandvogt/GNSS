//! `gnss process` with the real engine: RTKLIB's own two-receiver sample
//! (rover and base, 2005-04-02, GPS) imported, its navigation file in the
//! download cache, processed through wasmtime, and recorded for the
//! adjustment. It needs the built module, so CI runs it in the job that
//! builds it (`--ignored`, with `GNSS_RTKLIB_DIST` set to the module's
//! `dist/`, which holds the sample in `testdata/`).

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

use gnss_api::fetch::gnss_fetch::{Cache, Entry, plan::ANTEX};
use gnss_model::{DataFile, DataFiles};

fn gnss(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gnss"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

#[test]
#[ignore = "needs the built module: CI runs it in the RTKLIB-EX job"]
fn process_runs_the_engine_and_records_the_fixed_baseline() {
    let dist = PathBuf::from(std::env::var("GNSS_RTKLIB_DIST").expect("GNSS_RTKLIB_DIST"));
    let data = dist.join("testdata");
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("p");
    assert!(
        gnss(root.path(), &["init", project.to_str().unwrap()])
            .status
            .success()
    );
    let rover = data.join("07590920.05o");
    let base = data.join("30400920.05o");
    let o = gnss(
        &project,
        &[
            "import",
            "rinex",
            rover.to_str().unwrap(),
            base.to_str().unwrap(),
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));

    // What gnss fetch would have left: the navigation file and ANTEX (a
    // header only: RTKLIB's sample predates igs20) in the cache, and
    // data.toml naming them.
    let cache_dir = root.path().join("cache");
    let mut cache = Cache::new(gnss_store::FsStore::new(&cache_dir));
    let antex = "     1.4            M                                       ANTEX VERSION / SYST\n\
                 A                                                           PCV TYPE / REFANT\n\
                 \x20                                                           END OF HEADER\n";
    let mut files = DataFiles::new();
    for (purpose, name, bytes) in [
        (
            "navigation data, 2005-04-02",
            "30400920.05n",
            std::fs::read(data.join("30400920.05n")).unwrap(),
        ),
        (ANTEX, ANTEX, antex.as_bytes().to_vec()),
    ] {
        let entry = Entry {
            source: "test".into(),
            name: name.into(),
            sha256: String::new(),
            bytes: 0,
            url: "https://x.example/".into(),
            fetched: "2026-10-31T00:00:00Z".into(),
            licence: "test".into(),
        };
        let e = cache.put(entry, &bytes).unwrap();
        files.files.push(DataFile {
            purpose: purpose.into(),
            name: e.name,
            source: e.source,
            sha256: e.sha256,
            bytes: e.bytes,
            url: e.url,
            fetched: e.fetched,
            licence: e.licence,
            note: None,
            site_log: None,
        });
    }
    std::fs::write(project.join("data.toml"), files.to_toml()).unwrap();
    // The sample is GPS only.
    let settings = std::fs::read_to_string(project.join("project.toml")).unwrap();
    std::fs::write(
        project.join("project.toml"),
        format!("{settings}\n[processing]\nconstellations = [\"gps\"]\n"),
    )
    .unwrap();

    let engine = dist.join("rnx2rtkp.wasm");
    let o = gnss(
        &project,
        &[
            "--json",
            "process",
            "--engine",
            engine.to_str().unwrap(),
            "--cache",
            cache_dir.to_str().unwrap(),
        ],
    );
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success(),
        "{out}\n{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let report: serde_json::Value = serde_json::from_str(&out).unwrap();
    let b = &report["baselines"][0];
    assert_eq!(
        (b["base"].as_str(), b["rover"].as_str()),
        (Some("0759"), Some("3040"))
    );
    assert_eq!(b["quality"], "fixed");
    assert_eq!(b["used"], true);
    let length = b["length"].as_f64().unwrap();
    assert!((length - 3335.0).abs() < 5.0, "{length}");
    let file = b["file"].as_str().unwrap();
    assert!(project.join(file).exists());
    assert!(
        std::fs::read_to_string(project.join("project.toml"))
            .unwrap()
            .contains(file)
    );
    // A different module is refused.
    let wrong = root.path().join("other.wasm");
    std::fs::write(&wrong, b"\0asm\x01\0\0\0").unwrap();
    let o = gnss(
        &project,
        &[
            "process",
            "--engine",
            wrong.to_str().unwrap(),
            "--cache",
            cache_dir.to_str().unwrap(),
        ],
    );
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("isn't the engine"));
}
