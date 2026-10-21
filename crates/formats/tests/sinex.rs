//! Reading SINEX solutions as weighted control.

#![allow(clippy::unwrap_used)]

use gnss_datum::FrameId;
use gnss_formats::sinex::read;
use gnss_formats::{CoordinateValue, Element, MeasurementKind};
use nalgebra::Vector3;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const DYNADJUST: &str = include_str!("data/sinex/TEST_ITRF05.SNX");

#[test]
fn reads_dynadjusts_sample() {
    // CATREF's comment doesn't name the frame, so the caller does.
    let err = read("t.snx", DYNADJUST, None).unwrap_err();
    assert!(err.to_string().contains("say which frame"), "{err}");
    let survey = read("t.snx", DYNADJUST, Some(FrameId::Itrf2005)).unwrap();
    assert!(survey.notes.is_empty(), "{:?}", survey.notes);
    let names: Vec<&str> = survey.stations.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["ALIC", "KARR", "YAR2"]);
    assert_eq!(
        survey.stations[0].description.as_deref(),
        Some("NT, Australia")
    );
    assert_eq!(
        survey.stations[0].coordinate.as_ref().unwrap().value,
        CoordinateValue::Cartesian(Vector3::new(
            -0.405205265910111e7,
            0.421283600689116e7,
            -0.254510472135050e7
        ))
    );
    let m = &survey.measurements[0];
    assert_eq!(m.kind, MeasurementKind::PointCluster);
    assert_eq!(m.elements.len(), 3);
    assert!(matches!(&m.elements[2], Element::Position { station, .. } if station == "YAR2"));
    // 17:298:43200 is noon UTC on 25 October 2017.
    let expected = 2017.0 + (297.0 * 86_400.0 + 43_218.0) / (365.0 * 86_400.0);
    let epoch = m.reference.epoch.unwrap().decimal_year();
    assert!((epoch - expected).abs() < 1e-12, "{epoch}");
    assert_eq!(m.reference.frame, FrameId::Itrf2005);
    // The lower triangle, mirrored: ALIC's X with KARR's Y is (5, 1).
    let c = &m.covariance;
    assert_eq!(c[(0, 0)], 0.706765843212940e-07);
    assert_eq!(c[(4, 0)], 0.101642303438650e-08);
    assert_eq!(c[(0, 4)], c[(4, 0)]);
    assert_eq!(c[(8, 6)], 0.332918060381180e-07);
    assert_eq!(c, &c.transpose());
}

/// A two-site SINEX file around a matrix block.
fn sinex(comment: &str, matrix: &str) -> String {
    format!(
        "%=SNX 2.02 TST 25:166:00000 TST 25:165:00000 25:165:86370 P 00006 2 X\n\
         +FILE/COMMENT\n{comment}\n-FILE/COMMENT\n\
         +SOLUTION/ESTIMATE\n\
         *INDEX TYPE__ CODE PT SOLN _REF_EPOCH__ UNIT S __ESTIMATED VALUE____ _STD_DEV___\n\
         \x20    1 STAX   AAAA  A    1 25:165:43200 m    2 -.400000000000000E+07 0.10000E-02\n\
         \x20    2 STAY   AAAA  A    1 25:165:43200 m    2 0.420000000000000E+07 0.10000E-02\n\
         \x20    3 STAZ   AAAA  A    1 25:165:43200 m    2 -.250000000000000E+07 0.10000E-02\n\
         \x20    4 VELX   AAAA  A    1 25:165:43200 m/y  2 -.300000000000000E-01 0.10000E-03\n\
         \x20    5 STAX   BBBB  A    1 25:165:43200 m    2 -.400100000000000E+07 0.20000E-02\n\
         \x20    6 STAY   BBBB  A    1 25:165:43200 m    2 0.420010000000000E+07 0.20000E-02\n\
         \x20    7 STAZ   BBBB  A    1 25:165:43200 m    2 -.250010000000000E+07 0.20000E-02\n\
         -SOLUTION/ESTIMATE\n{matrix}\n%ENDSNX\n"
    )
}

#[test]
fn reads_correlation_and_information_matrices() {
    // Correlations with standard deviations on the diagonal, upper triangle.
    let corr = "+SOLUTION/MATRIX_ESTIMATE U CORR\n\
                \x20    1     1 0.10000E-02 0.50000E+00 0.00000E+00\n\
                \x20    2     2 0.10000E-02\n\
                \x20    3     3 0.10000E-02\n\
                \x20    4     4 0.10000E-03\n\
                \x20    5     5 0.20000E-02 0.25000E+00\n\
                \x20    6     6 0.20000E-02\n\
                \x20    7     7 0.20000E-02\n\
                -SOLUTION/MATRIX_ESTIMATE U CORR";
    let survey = read("c.snx", &sinex("* Coordinates in IGS20", corr), None).unwrap();
    let m = &survey.measurements[0];
    // The frame named in the comment, noted; the velocity skipped, noted.
    assert_eq!(m.reference.frame, FrameId::Igs20);
    let notes: Vec<&str> = survey.notes.iter().map(|n| n.message.as_str()).collect();
    assert_eq!(
        notes,
        [
            "the frame was taken as IGS20, which the file's comments name",
            "skipped 1 VELX parameter: only station positions are read",
        ]
    );
    let c = &m.covariance;
    assert!((c[(0, 0)] - 1e-6).abs() < 1e-20);
    assert!((c[(0, 1)] - 0.5e-6).abs() < 1e-20);
    // BBBB's X (parameter 5) with its Y (6): 0.25 × 2 mm × 2 mm.
    assert!((c[(3, 4)] - 1e-6).abs() < 1e-20);
    assert!((c[(3, 3)] - 4e-6).abs() < 1e-20);

    // An information matrix is inverted: a diagonal one, simply.
    let info = "+SOLUTION/MATRIX_ESTIMATE L INFO\n\
                \x20    1     1 0.10000E+07\n     2     2 0.10000E+07\n     3     3 0.10000E+07\n\
                \x20    4     4 0.10000E+09\n     5     5 0.25000E+06\n     6     6 0.25000E+06\n\
                \x20    7     7 0.25000E+06\n\
                -SOLUTION/MATRIX_ESTIMATE L INFO";
    let survey = read("i.snx", &sinex("", info), Some(FrameId::Itrf2020)).unwrap();
    let c = &survey.measurements[0].covariance;
    assert!((c[(0, 0)] - 1e-6).abs() < 1e-18);
    assert!((c[(5, 5)] - 4e-6).abs() < 1e-18);
}

#[test]
fn explains_what_it_cant_read() {
    let cova = "+SOLUTION/MATRIX_ESTIMATE L COVA\n\
                \x20    1     1 0.1E-05\n     2     2 0.1E-05\n     3     3 0.1E-05\n\
                \x20    5     5 0.4E-05\n     6     6 0.4E-05\n     7     7 0.4E-05\n\
                -SOLUTION/MATRIX_ESTIMATE L COVA";
    let err = |text: &str| read("e.snx", text, None).unwrap_err().to_string();
    assert_eq!(
        err("hello"),
        "e.snx, line 1: it isn't a SINEX file: the first line should start %=SNX"
    );
    assert_eq!(
        err(&sinex("* ITRF2014 and IGS20", cova)),
        "e.snx: the file's comments name IGS20 and ITRF2014: say which frame the solution is in"
    );
    assert!(
        err(&sinex("ITRF2014", "")).contains("no SOLUTION/MATRIX_ESTIMATE"),
        "{}",
        err(&sinex("ITRF2014", ""))
    );
    let other_epoch = sinex("ITRF2014", cova).replace(
        "5 STAX   BBBB  A    1 25:165:43200",
        "5 STAX   BBBB  A    1 25:166:00000",
    );
    assert_eq!(
        err(&other_epoch),
        "e.snx, line 12: site BBBB's X, Y and Z have different reference epochs"
    );
    let missing = sinex("ITRF2014", cova).replace("STAZ   BBBB", "VELZ   BBBB");
    assert_eq!(
        err(&missing),
        "e.snx, line 11: site BBBB doesn't have all of STAX, STAY and STAZ"
    );
    let shape = sinex("ITRF2014", &cova.replace("L COVA", "D COVA"));
    assert!(
        err(&shape).contains("isn't a matrix form SINEX defines"),
        "{}",
        err(&shape)
    );
    let no_variance = sinex("ITRF2014", &cova.replace("     7     7 0.4E-05\n", ""));
    assert!(
        err(&no_variance).contains("has no variance"),
        "{}",
        err(&no_variance)
    );
}
