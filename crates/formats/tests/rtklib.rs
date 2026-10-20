//! Reading RTKLIB solutions: one real static baseline written by RTKLIB-EX in
//! each of its three forms, and what happens with other files.

#![allow(clippy::unwrap_used)]

use gnss_datum::{FrameId, GRS80};
use gnss_formats::rtklib::{Names, mark_from_file, read};
use gnss_formats::{Element, Measurement};
use nalgebra::{Matrix3, Vector3};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const XYZ: &str = include_str!("data/rtklib/static-xyz.pos");
const LLH: &str = include_str!("data/rtklib/static-llh.pos");
const ENU: &str = include_str!("data/rtklib/static-enu.pos");

fn names() -> Names {
    Names {
        base: Some("MTHR".into()),
        rover: Some("SNGO".into()),
    }
}

fn baseline(m: &Measurement) -> (Vector3<f64>, Matrix3<f64>) {
    let Element::Vector { from, to, vector } = &m.elements[0] else {
        panic!("{m:?}");
    };
    assert_eq!((from.as_str(), to.as_str()), ("MTHR", "SNGO"));
    (*vector, m.covariance.fixed_view::<3, 3>(0, 0).into_owned())
}

#[test]
fn reads_each_form_of_the_same_solution() {
    let read_one = |name: &str, text: &str| {
        let survey = read(name, text, Some(FrameId::Wgs84G2139), &names()).unwrap();
        assert!(survey.notes.is_empty(), "{:?}", survey.notes);
        assert_eq!(survey.measurements.len(), 1);
        assert_eq!(survey.stations[0].name, "MTHR");
        let m = &survey.measurements[0];
        // The middle of the two hours: 03:00 GPST on 15 June 2025.
        let expected = 2025.0 + (165.0 * 86_400.0 + 3.0 * 3600.0) / (365.0 * 86_400.0);
        assert!((m.reference.epoch.unwrap().decimal_year() - expected).abs() < 1e-12);
        assert_eq!(m.reference.frame, FrameId::Wgs84G2139);
        assert_eq!(m.origin.line, 26);
        baseline(m)
    };
    let (xyz, cxyz) = read_one("static-xyz.pos", XYZ);
    let (llh, cllh) = read_one("static-llh.pos", LLH);
    let (enu, cenu) = read_one("static-enu.pos", ENU);
    // The base from the header, and the rover's solution.
    assert_eq!(
        xyz,
        Vector3::new(-4714363.8530, 2594342.1370, -3412807.8078)
            - Vector3::new(-4707913.7460, 2599003.2450, -3418163.9530)
    );
    // Each form rounds to 0.1 mm.
    assert!((llh - xyz).amax() < 1e-4, "{}", (llh - xyz).amax());
    assert!((enu - xyz).amax() < 1e-4, "{}", (enu - xyz).amax());

    // The covariances agree to RTKLIB's rounding of standard deviations to
    // 0.1 mm, and the signed square roots are undone: in ECEF, sdxy -0.0008
    // is a covariance of -6.4e-7 m².
    assert_eq!(cxyz[(0, 1)], -0.0008 * 0.0008);
    assert_eq!(cxyz[(2, 0)], 0.0007 * 0.0007);
    assert!((cllh - cxyz).amax() < 2e-7, "{}", (cllh - cxyz).amax());
    assert!((cenu - cxyz).amax() < 2e-7, "{}", (cenu - cxyz).amax());
    // In local axes at the base, the ENU file's own numbers come back.
    let r = GRS80
        .ecef_to_geodetic(&Vector3::new(-4707913.7460, 2599003.2450, -3418163.9530))
        .unwrap()
        .enu_rotation();
    let local = r * cenu * r.transpose();
    assert!((local[(0, 0)] - 0.0004 * 0.0004).abs() < 1e-12);
    assert!((local[(1, 2)] - 0.0006 * 0.0006).abs() < 1e-12);
    assert!((local[(0, 1)] + 0.0001 * 0.0001).abs() < 1e-12);
}

#[test]
fn names_marks_from_the_files_or_the_caller() {
    let survey = read("static-xyz.pos", XYZ, None, &Names::default()).unwrap();
    let Element::Vector { from, to, .. } = &survey.measurements[0].elements[0] else {
        panic!();
    };
    // rnx2rtkp was given rover.obs, base.obs and nav.rnx.
    assert_eq!((from.as_str(), to.as_str()), ("base", "rover"));
    let notes: Vec<&str> = survey.notes.iter().map(|n| n.message.as_str()).collect();
    assert_eq!(
        notes,
        [
            "the base is taken as base and the rover as rover, from the file names",
            "RTKLIB doesn't record the orbits' frame; WGS 84 was assumed, as for broadcast orbits",
        ]
    );
    assert_eq!(survey.measurements[0].reference.frame, FrameId::Wgs84);

    for (file, mark) in [
        ("07590920.05o", Some("0759")),
        ("30400920.05n", None),
        (
            "/data/ALIC00AUS_R_20251660000_01D_30S_MO.crx.gz",
            Some("ALIC"),
        ),
        ("ALIC00AUS_R_20251660000_01D_MN.rnx", None),
        ("C:\\jobs\\PM12345A.obs", Some("PM12345A")),
        ("brdc1660.25n", None),
        ("nav.rnx", None),
        ("igs14.atx", None),
        ("rover.obs", Some("rover")),
        ("sngo1660.25d.Z", Some("SNGO")),
    ] {
        assert_eq!(mark_from_file(file).as_deref(), mark, "{file}");
    }
}

#[test]
fn notes_and_refuses_what_isnt_a_static_baseline() {
    let heading_and = |extra: &str, line: &str| {
        let header = XYZ.lines().take(25).collect::<Vec<_>>().join("\n");
        format!("{header}\n{extra}{line}\n")
    };
    let solution = XYZ.lines().nth(25).unwrap();
    // Two solutions: the last is used.
    let survey = read(
        "two.pos",
        &heading_and(&format!("{solution}\n"), solution),
        None,
        &names(),
    )
    .unwrap();
    assert!(
        survey.notes[0]
            .message
            .starts_with("it has 2 solutions; the last was used")
    );
    // A float solution is noted, and a single-point one refused.
    let float = solution.replacen("   1  22", "   2  22", 1);
    let survey = read("float.pos", &heading_and("", &float), None, &names()).unwrap();
    assert_eq!(
        survey.notes[0].message,
        "the solution is float (Q=2), not fixed"
    );
    let single = solution.replacen("   1  22", "   5  22", 1);
    let err = read("single.pos", &heading_and("", &single), None, &names()).unwrap_err();
    assert_eq!(
        err.to_string(),
        "single.pos, line 26: the solution is single point (Q=5), not a baseline from the base: \
         process it in a relative mode"
    );
    // Heights above a geoid aren't ellipsoidal.
    let geodetic = LLH.replace("WGS84/ellipsoidal", "WGS84/geodetic");
    let err = read("g.pos", &geodetic, None, &names()).unwrap_err();
    assert!(err.to_string().contains("heights are geodetic"), "{err}");
    // Without the base position there's no vector.
    let no_base: String = XYZ
        .lines()
        .filter(|l| !l.contains("ref pos"))
        .collect::<Vec<_>>()
        .join("\n");
    let err = read("n.pos", &no_base, None, &names()).unwrap_err();
    assert!(err.to_string().contains("no ref pos line"), "{err}");
    // Nor without the column heading.
    let err = read("h.pos", solution, None, &names()).unwrap_err();
    assert!(
        err.to_string().contains("no RTKLIB column heading"),
        "{err}"
    );
    let err = read(
        "b.pos",
        &heading_and("", "2025/06/15 02:00:00.000 x"),
        None,
        &names(),
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "b.pos, line 26: a solution needs at least 13 values, but this has 3"
    );
}
