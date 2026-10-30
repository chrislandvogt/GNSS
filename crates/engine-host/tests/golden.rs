//! The real `rnx2rtkp.wasm` under this host, against RTKLIB's own test data
//! and the result the module is checked against everywhere else
//! (`engine-modules/rtklib-ex/golden.sha256`). It needs the built module, so
//! CI runs it in the job that builds it (`--ignored`, with
//! `GNSS_RTKLIB_DIST` set to the module's `dist/` and its `testdata/`).

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use gnss_engine_host::{Host, Limits};
use gnss_store::sha256_hex as sha256;

fn golden(name: &str) -> String {
    include_str!("../../../engine-modules/rtklib-ex/golden.sha256")
        .lines()
        .find_map(|l| {
            let (hash, file) = l.split_once("  ")?;
            (file == name).then(|| hash.to_owned())
        })
        .unwrap()
}

#[test]
#[ignore = "needs the built module: CI runs it in the RTKLIB-EX job"]
fn rnx2rtkp_gives_the_golden_solution_under_wasmtime() {
    let dist = PathBuf::from(std::env::var("GNSS_RTKLIB_DIST").expect("GNSS_RTKLIB_DIST"));
    let wasm = std::fs::read(dist.join("rnx2rtkp.wasm")).unwrap();
    assert_eq!(
        sha256(&wasm),
        golden("rnx2rtkp.wasm"),
        "not the module CI built"
    );
    // The inputs, copied: the module gets only its own folder.
    let folder = tempfile::tempdir().unwrap();
    for file in ["07590920.05o", "30400920.05n", "30400920.05o"] {
        std::fs::copy(dist.join("testdata").join(file), folder.path().join(file)).unwrap();
    }
    let host = Host::new(&wasm).unwrap();
    let args = [
        "rnx2rtkp",
        "-t",
        "-p",
        "3",
        "-e",
        "-r",
        "-3978241.958",
        "3382840.234",
        "3649900.853",
        "07590920.05o",
        "30400920.05n",
        "30400920.05o",
    ];
    let out = host.run(&args, folder.path(), Limits::default()).unwrap();
    assert_eq!(out.status, 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(sha256(&out.stdout), golden("test4.pos"));
}
