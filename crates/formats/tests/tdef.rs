//! Reading Trimble Data Exchange Format files, on two real Trimble Business
//! Center exports (`tests/data/tdef`).

#![allow(clippy::unwrap_used)]

use gnss_datum::{CoordinateEpoch, FrameId};
use gnss_formats::tdef::read;
use gnss_formats::{Constraint, CoordinateValue, Element, HeightType, Scaling};
use nalgebra::Vector3;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const BRIGHT: &str = include_str!("data/tdef/tbc-bright-2019.asc");
const GEELONG: &str = include_str!("data/tdef/tbc-geelong-2020.asc");

#[test]
fn reads_a_tbc_export() {
    let survey = read("bright.asc", BRIGHT, FrameId::Gda2020, None).unwrap();
    assert_eq!(survey.measurements.len(), 31);

    // The first vector, exactly as written, line 51.
    let first = &survey.measurements[0];
    assert_eq!(first.origin.line, 51);
    let Element::Vector { from, to, vector } = &first.elements[0] else {
        panic!()
    };
    assert_eq!((from.as_str(), to.as_str()), ("324900360", "MIMI"));
    assert_eq!(*vector, Vector3::new(-33180.9965, -47900.0482, 2293.9375));
    let c = &first.covariance;
    assert_eq!(c[(0, 0)], 2.8657057555e-004);
    assert_eq!(c[(0, 1)], -1.6807920474e-004);
    assert_eq!(c[(1, 0)], c[(0, 1)]);
    assert_eq!(c[(0, 2)], 2.4558452424e-004);
    assert_eq!(c[(1, 1)], 1.7563594156e-004);
    assert_eq!(c[(1, 2)], -1.8154094869e-004);
    assert_eq!(c[(2, 2)], 2.5905718212e-004);
    assert_eq!(first.scaling, Scaling::default());
    assert_eq!(first.reference.frame, FrameId::Gda2020);
    assert_eq!(first.reference.epoch, None);

    // Every mark, from its keyed-in GDA2020 coordinate, free.
    assert_eq!(survey.stations.len(), 11);
    let beec = survey.stations.iter().find(|s| s.name == "BEEC").unwrap();
    assert_eq!(beec.constraint, Constraint::Free);
    assert_eq!(beec.origin.line, 35);
    let coordinate = beec.coordinate.as_ref().unwrap();
    assert_eq!(coordinate.reference.frame, FrameId::Gda2020);
    let CoordinateValue::Geographic {
        latitude_rad,
        longitude_rad,
        height_m,
        height,
    } = coordinate.value
    else {
        panic!()
    };
    assert!((latitude_rad.to_degrees() + 36.346_434_060).abs() < 1e-12);
    assert!((longitude_rad.to_degrees() - 146.657_743_009).abs() < 1e-12);
    assert_eq!(height_m, 442.9339);
    assert_eq!(height, HeightType::Ellipsoidal);
    assert!(survey.notes.iter().any(|n| n.message
        == "11 marks are starting positions only (11 keyed in, 0 from processing), in GDA2020; \
            to hold control, import it"));
}

#[test]
fn dynamic_frames_take_the_middle_of_each_observation() {
    let survey = read("geelong.asc", GEELONG, FrameId::Itrf2014, None).unwrap();
    assert_eq!(survey.measurements.len(), 3);
    assert_eq!(survey.stations.len(), 4);
    // 16 February 2020 22:50:34 to 17 February 2020 04:51:22: the middle is
    // 17 February 01:50:58, 47.077 days into a 366-day year.
    let epoch = survey.measurements[0].reference.epoch.unwrap();
    let expected = 2020.0 + (47.0 + (1.0 + 50.0 / 60.0 + 58.0 / 3600.0) / 24.0) / 366.0;
    assert!((epoch.decimal_year() - expected).abs() < 1e-6, "{epoch}");

    // An epoch given for the file applies to every vector.
    let given = CoordinateEpoch::from_decimal_year(2020.5).unwrap();
    let survey = read("geelong.asc", GEELONG, FrameId::Itrf2014, Some(given)).unwrap();
    assert!(
        survey
            .measurements
            .iter()
            .all(|m| m.reference.epoch == Some(given))
    );
}

fn error(text: &str) -> String {
    read("x.asc", text, FrameId::Gda2020, None)
        .unwrap_err()
        .to_string()
}

#[test]
fn refuses_what_it_cannot_read_safely() {
    // Swapping two covariance terms leaves a matrix that can't be a
    // covariance, which is how a misread order would show.
    let swapped = BRIGHT.replacen(
        "2.8657057555e-004:-1.6807920474e-004",
        "-1.6807920474e-004:2.8657057555e-004",
        1,
    );
    let e = error(&swapped);
    assert!(
        e.contains("x.asc, line 51: vector 324900360-MIMI's covariance")
            && e.contains("isn't positive definite"),
        "{e}"
    );
    let e = error(&BRIGHT.replace("GPSVectors=MarkToMark", "GPSVectors=AntennaToAntenna"));
    assert!(e.contains("not mark to mark"), "{e}");
    let e = error(&BRIGHT.replace("DistanceUnits=meters", "DistanceUnits=US survey feet"));
    assert!(e.contains("only metres are read"), "{e}");
    let e = error(&BRIGHT.replace("36.346434060S", "36.346434060Q"));
    assert!(e.contains("BEEC's latitude should end in N or S"), "{e}");
    let e = error(&BRIGHT.replace("16 12 2019:21 48 47.0", "16 13 2019:21 48 47.0"));
    assert!(e.contains("aren't dates and times TDEF writes"), "{e}");
    let e = error("[General]\nSeparator=:\n\n[Stations]\n");
    assert!(e.contains("it has no GPS vectors"), "{e}");
    let e = error("[General]\nSeparator=:\n[GPS]\nVector=1:?:A:B:1:2\n");
    assert!(e.contains("line 4: the vector has no ΔZ"), "{e}");
    assert!(error("not a TDEF file\n").contains("isn't a TDEF record"));

    // Without observation times, a dynamic frame needs an epoch.
    let untimed = "[General]\n[GPS]\nVector=1:?:A:B:1:2:3:1e-6:0:0:1e-6:0:1e-6\n";
    let e = read("x.asc", untimed, FrameId::Itrf2014, None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("give the import an epoch"), "{e}");
    let survey = read("x.asc", untimed, FrameId::Gda2020, None).unwrap();
    assert!(
        survey
            .notes
            .iter()
            .any(|n| n.message.contains("taken to be"))
    );
}

#[test]
fn stations_in_an_unknown_system_are_noted_and_skipped() {
    let other = GEELONG.replace("Australia/GDA2020", "Local/Site grid");
    let survey = read("geelong.asc", &other, FrameId::Gda2020, None).unwrap();
    assert!(survey.stations.is_empty());
    assert!(survey.notes.iter().any(|n| n.message
        == "its 4 station coordinates were skipped: the coordinate system Local/Site grid isn't \
            one whose frame is known"));
}
