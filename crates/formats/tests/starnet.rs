//! Reading and writing STAR*NET GPS vector files.

#![allow(clippy::unwrap_used)]

use gnss_datum::FrameId;
use gnss_formats::starnet::{read, write};
use gnss_formats::{Element, Reference, Scaling};
use nalgebra::Vector3;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const WGS84_2025: Reference = Reference {
    frame: FrameId::Wgs84,
    epoch: None,
};

fn reference() -> Reference {
    Reference {
        epoch: gnss_datum::CoordinateEpoch::from_decimal_year(2025.45).ok(),
        ..WGS84_2025
    }
}

const COVARIANCES: &str = "\
# Exported vectors
.GPS WEIGHT COVARIANCE
G0 'V1  Session 1
G1 MTHR-SNGO   -6450.1070   -4661.1080   5356.1452
G2 1.44E-06   4.90E-07   3.60E-07
G3 -6.40E-07   4.90E-07   -2.50E-07
G0 'V2
G1 SNGO-PM123   1000.0  -2000.0  500.0  # a comment
G2 1.0E-06 1.0E-06 4.0E-06
G3 0 0 0
C PM123 6000000.000 300000.000 10.000 ! ! !
DB MTHR
";

#[test]
fn reads_covariance_weighting() {
    let survey = read("v.gps", COVARIANCES, reference()).unwrap();
    assert_eq!(survey.measurements.len(), 2);
    let m = &survey.measurements[0];
    assert_eq!(m.id.as_deref(), Some("V1  Session 1"));
    assert_eq!(m.origin.line, 4);
    assert_eq!(
        m.elements[0],
        Element::Vector {
            from: "MTHR".into(),
            to: "SNGO".into(),
            vector: Vector3::new(-6450.1070, -4661.1080, 5356.1452),
        }
    );
    let c = &m.covariance;
    assert_eq!((c[(0, 0)], c[(1, 1)], c[(2, 2)]), (1.44e-6, 4.9e-7, 3.6e-7));
    assert_eq!(
        (c[(0, 1)], c[(0, 2)], c[(1, 2)]),
        (-6.4e-7, 4.9e-7, -2.5e-7)
    );
    assert_eq!(c, &c.transpose());
    assert_eq!(m.reference, reference());
    let notes: Vec<String> = survey
        .notes
        .iter()
        .map(|n| format!("{}: {}", n.origin, n.message))
        .collect();
    assert_eq!(
        notes,
        [
            "v.gps, line 11: skipped 1 C record: only GPS vectors (G records) are read",
            "v.gps, line 12: skipped 1 DB record: only GPS vectors (G records) are read",
        ]
    );
}

#[test]
fn reads_standard_errors_and_correlations_with_factors() {
    let text = "\
.GPS WEIGHT STDERRCORR
.GPS FACTOR 2.0 VERT 3.0
G1 A-B 1 2 3 'descriptor
G2 0.001 0.002 0.003
G3 0.5 -0.25 0
";
    let survey = read("s.gps", text, reference()).unwrap();
    let m = &survey.measurements[0];
    assert_eq!(m.id.as_deref(), Some("descriptor"));
    let c = &m.covariance;
    assert!((c[(0, 0)] - 1e-6).abs() < 1e-20);
    assert!((c[(0, 1)] - 0.5 * 0.001 * 0.002).abs() < 1e-20);
    assert!((c[(0, 2)] + 0.25 * 0.001 * 0.003).abs() < 1e-20);
    // The factors scale standard errors, so variances by their squares.
    assert_eq!(
        m.scaling,
        Scaling {
            v: 1.0,
            p: 4.0,
            l: 4.0,
            h: 9.0
        }
    );
}

#[test]
fn refuses_correlations_taken_for_covariances() {
    let text = "G1 A-B 1 2 3\nG2 0.001 0.002 0.003\nG3 0.5 -0.25 0\n";
    let err = read("s.gps", text, reference()).unwrap_err().to_string();
    assert!(
        err.starts_with(
            "s.gps, line 1: vector A-B: its G3 values [0.5, -0.25, 0.0] look like correlations"
        ),
        "{err}"
    );
    let bad = |text: &str| read("b.gps", text, reference()).unwrap_err().to_string();
    assert_eq!(
        bad("G2 1 2 3\n"),
        "b.gps, line 1: G2 comes before any G1 record"
    );
    assert_eq!(
        bad("G1 A-B 1 2 3\n"),
        "b.gps, line 1: vector A-B has no G2 record to weight it"
    );
    assert_eq!(
        bad("G1 A-B 1 2 3 1.5 1.6\nG2 1 1 1\n"),
        "b.gps, line 1: G1 has 5 values after the stations; antenna heights on G1 aren't read \
         yet, so give mark-to-mark vectors"
    );
    assert_eq!(
        bad("G1 AB 1 2 3\n"),
        "b.gps, line 1: G1 should name the stations FROM-TO, not \"AB\""
    );
    assert_eq!(
        bad("G1 A-A 1 2 3\n"),
        "b.gps, line 1: the vector from A ends at A too"
    );
    assert_eq!(
        bad("G1 A-B 1 2 3\nG2 1e-6 1e-6 1e-6\nG4 0 0 1\n"),
        "b.gps, line 3: G4 offsets aren't read yet, so give vectors without them"
    );
    assert_eq!(
        bad(".GPS WEIGHT SOMETIMES\n"),
        "b.gps, line 1: .GPS WEIGHT should be COVARIANCE or STDERRCORR, not SOMETIMES"
    );
    assert_eq!(
        bad("# nothing\n"),
        "b.gps: it has no GPS vectors (G1 records)"
    );
}

#[test]
fn round_trips() {
    let original = read("v.gps", COVARIANCES, reference()).unwrap();
    let written = write(&original.measurements).unwrap();
    let again = read("w.gps", &written, reference()).unwrap();
    for (a, b) in original.measurements.iter().zip(&again.measurements) {
        assert_eq!(a.elements, b.elements);
        assert_eq!(a.covariance, b.covariance);
        assert_eq!(a.id, b.id);
    }
    let mut renamed = original.measurements.clone();
    renamed[0].elements[0] = Element::Vector {
        from: "PM 12".into(),
        to: "B".into(),
        vector: Vector3::zeros(),
    };
    assert_eq!(
        write(&renamed).unwrap_err().to_string(),
        "station name \"PM 12\" can't be written for STAR*NET, which separates FROM-TO with a \
         hyphen: rename it"
    );
}
