//! Corrected copies: the header patched, everything else as it was.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;
#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

use gnss_rinex_io::{HeaderPatch, read_obs, rewrite_obs};

const SWTC: &[u8] = include_bytes!("data/rinex/SWTC00AUS_20251660000.rnx");
const SWTC_CRX: &[u8] = include_bytes!("data/rinex/SWTC00AUS_20251660000.rnx.crx");
const MOBS: &[u8] = include_bytes!("data/rinex/MOBS0050.20o");

fn epochs(bytes: &[u8]) -> Vec<gnss_rinex_io::Epoch> {
    let mut r = read_obs(bytes).unwrap();
    let mut out = Vec::new();
    while let Some(e) = r.next_epoch().unwrap() {
        out.push(e);
    }
    out
}

fn patch() -> HeaderPatch {
    HeaderPatch {
        marker_name: Some("PM12345".into()),
        receiver: Some("TRIMBLE R12i".into()),
        antenna: Some("TRM59800.00".into()),
        radome: Some("SCIS".into()),
        antenna_delta: Some([1.532, 0.0, 0.0]),
    }
}

#[test]
fn the_header_is_patched_and_the_epochs_kept() {
    for (name, bytes) in [("SWTC", SWTC), ("SWTC crx", SWTC_CRX), ("MOBS", MOBS)] {
        let mut out = Vec::new();
        let comments = ["Corrected from overlay 2026-10-15-x-header: the field log".to_owned()];
        rewrite_obs(bytes, &mut out, &patch(), &comments).unwrap();
        let h = read_obs(&out[..]).unwrap().header().clone();
        let mut expected = read_obs(bytes).unwrap().header().clone();
        assert_ne!(h, expected, "{name}");
        expected.patch(&patch());
        expected.comments.push(comments[0].clone());
        assert_eq!(h, expected, "{name}");
        assert_eq!(h.antenna.kind, "TRM59800.00");
        assert_eq!(h.antenna.radome.as_deref(), Some("SCIS"));
        // Serial numbers and firmware are kept.
        assert_eq!(
            h.receiver.number,
            read_obs(bytes).unwrap().header().receiver.number
        );
        assert_eq!(epochs(&out), epochs(bytes), "{name}");
        // Every line is a RINEX line: none longer than 80 in the header.
        let text = String::from_utf8(out).unwrap();
        let header = text.split("END OF HEADER").next().unwrap();
        assert!(header.lines().all(|l| l.len() <= 80), "{name}");
    }
}

#[test]
fn records_the_header_lacks_are_added() {
    let text = "     3.04           OBSERVATION DATA    G                   RINEX VERSION / TYPE
G    1 C1C                                                  SYS / # / OBS TYPES
                                                            END OF HEADER
> 2026 01 01 00 00  0.0000000  0  1
G01  20000000.000
";
    let mut out = Vec::new();
    let long = "x".repeat(70);
    rewrite_obs(text.as_bytes(), &mut out, &patch(), &[long]).unwrap();
    let r = read_obs(&out[..]).unwrap();
    let h = r.header();
    assert_eq!(h.marker_name, "PM12345");
    assert_eq!(h.receiver.kind, "TRIMBLE R12i");
    assert_eq!(h.antenna_delta, Some([1.532, 0.0, 0.0]));
    // A long comment goes over two records.
    assert_eq!(h.comments, ["x".repeat(60), "x".repeat(10)]);
    // An empty patch copies the file as it was.
    let mut same = Vec::new();
    rewrite_obs(text.as_bytes(), &mut same, &HeaderPatch::default(), &[]).unwrap();
    assert_eq!(same, text.as_bytes());
    assert!(rewrite_obs(&b"     3.04"[..], &mut Vec::new(), &patch(), &[]).is_err());
}
