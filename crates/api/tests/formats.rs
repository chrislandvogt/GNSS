//! Every format carries a network the same way: DynAdjust's sample network,
//! flattened to single baselines and single control positions in GDA2020
//! (what STAR*NET and the CSV formats can hold), written as DynaML, as
//! STAR*NET with CSV control, and as CSV, read back, built and adjusted,
//! gives bit-identical results each time.

#![allow(clippy::unwrap_used)]

use gnss_adjust::{Adjustment, Options};
use gnss_api::build::{DatumChoice, Settings, build};
use gnss_api::export;
use gnss_datum::{FrameId, TransformationGraph};
use gnss_formats::{
    Constraint, Coordinate, CoordinateValue, Element, Measurement, MeasurementKind, Reference,
    Station, Survey, csv, dynaml, starnet,
};
use nalgebra::DMatrix;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const STATIONS: &str = include_str!("../../formats/tests/data/dynaml/gnss-networkstn.xml");
const MEASUREMENTS: &str = include_str!("../../formats/tests/data/dynaml/gnss-networkmsr.xml");

const GDA2020: Reference = Reference {
    frame: FrameId::Gda2020,
    epoch: None,
};

fn settings() -> Settings {
    Settings {
        frame: FrameId::Gda2020,
        epoch: None,
        datum: DatumChoice::Constrained,
    }
}

fn adjust(survey: &Survey) -> Adjustment {
    let built = build(survey, &settings(), TransformationGraph::builtin()).unwrap();
    built
        .network
        .adjust(
            &built.datum,
            &Options {
                edits: built.edits.clone(),
                ..Options::default()
            },
        )
        .unwrap()
}

/// The sample in GDA2020, each element its own measurement, and the marks at
/// their starting positions.
fn flattened() -> Survey {
    let mut survey = dynaml::read("stn.xml", STATIONS).unwrap();
    survey.extend(dynaml::read("msr.xml", MEASUREMENTS).unwrap());
    let built = build(&survey, &settings(), TransformationGraph::builtin()).unwrap();
    let mut approximate = built.network.approximate_positions().unwrap();
    // Control marks start at their observed positions, as in the CSV control
    // format, which writes one position per mark.
    for o in built.network.observations() {
        for e in &o.elements {
            if let gnss_adjust::Element::Position { mark, position, .. } = e {
                approximate[*mark] = *position;
            }
        }
    }
    let stations = built
        .network
        .marks()
        .iter()
        .zip(&approximate)
        .map(|(mark, xyz)| Station {
            name: mark.name.clone(),
            description: None,
            coordinate: Some(Coordinate {
                value: CoordinateValue::Cartesian(*xyz),
                reference: GDA2020,
            }),
            constraint: Constraint::Free,
            origin: gnss_formats::Origin {
                file: String::new(),
                line: 0,
            },
        })
        .collect();
    let mut measurements = Vec::new();
    for m in export::measurements(&built) {
        for (i, element) in m.elements.iter().enumerate() {
            let block = m.covariance.fixed_view::<3, 3>(3 * i, 3 * i);
            measurements.push(Measurement {
                kind: match element {
                    Element::Vector { .. } => MeasurementKind::Baseline,
                    Element::Position { .. } => MeasurementKind::PointCluster,
                },
                elements: vec![element.clone()],
                covariance: DMatrix::from_column_slice(3, 3, block.into_owned().as_slice()),
                id: Some(format!("{}/{}", m.id.clone().unwrap_or_default(), i + 1)),
                ..m.clone()
            });
        }
    }
    // Control positions in the order of their marks, as the CSV control
    // format writes them, so every route sums the same terms in the same order.
    let mark = |m: &Measurement| match &m.elements[0] {
        Element::Position { station, .. } => built.network.mark(station),
        Element::Vector { .. } => None,
    };
    measurements.sort_by_key(|m| mark(m));
    Survey {
        stations,
        measurements,
        notes: Vec::new(),
    }
}

fn same(a: &Adjustment, b: &Adjustment, route: &str) {
    assert_eq!(a.marks().len(), b.marks().len(), "{route}");
    for (x, y) in a.marks().iter().zip(b.marks()) {
        assert_eq!(x.position, y.position, "{route}");
        assert_eq!(x.cofactor, y.cofactor, "{route}");
    }
    assert_eq!(
        a.statistics().sum_of_squares,
        b.statistics().sum_of_squares,
        "{route}"
    );
}

#[test]
fn every_format_carries_the_network_exactly() {
    let survey = flattened();
    let (vectors, points): (Vec<Measurement>, Vec<Measurement>) = survey
        .measurements
        .iter()
        .cloned()
        .partition(|m| m.kind == MeasurementKind::Baseline);
    assert_eq!((vectors.len(), points.len()), (133, 6));
    let direct = adjust(&survey);

    // DynaML.
    let mut via_dynaml = dynaml::read(
        "s.xml",
        &dynaml::write_stations(&survey.stations, GDA2020).unwrap(),
    )
    .unwrap();
    via_dynaml.extend(
        dynaml::read(
            "m.xml",
            &dynaml::write_measurements(&survey.measurements, GDA2020),
        )
        .unwrap(),
    );
    same(&direct, &adjust(&via_dynaml), "DynaML");

    // STAR*NET vectors with CSV control.
    let control = csv::write_control(&survey.stations, &points).unwrap();
    let mut via_starnet = csv::read_control("c.csv", &control, None).unwrap();
    let vector_file = starnet::write(&vectors).unwrap();
    let mut starnet_survey = starnet::read("v.gps", &vector_file, GDA2020).unwrap();
    // Control positions follow the vectors, as in the original.
    let positions = std::mem::take(&mut via_starnet.measurements);
    via_starnet.measurements = std::mem::take(&mut starnet_survey.measurements);
    via_starnet.measurements.extend(positions);
    same(&direct, &adjust(&via_starnet), "STAR*NET and CSV control");

    // CSV vectors with CSV control.
    let mut via_csv = csv::read_control("c.csv", &control, None).unwrap();
    let positions = std::mem::take(&mut via_csv.measurements);
    via_csv.measurements = csv::read_vectors("v.csv", &csv::write_vectors(&vectors), None)
        .unwrap()
        .measurements;
    via_csv.measurements.extend(positions);
    same(&direct, &adjust(&via_csv), "CSV");
}

#[test]
fn an_rtklib_baseline_joins_csv_control_in_gda2020() {
    let pos = include_str!("../../formats/tests/data/rtklib/static-xyz.pos");
    let names = gnss_formats::rtklib::Names {
        base: Some("MTHR".into()),
        rover: Some("SNGO".into()),
    };
    let mut survey = gnss_formats::rtklib::read("static-xyz.pos", pos, None, &names).unwrap();
    survey.extend(
        csv::read_control(
            "control.csv",
            "name,x,y,z\nMTHR,-4707913.7460,2599003.2450,-3418163.9530\n",
            Some(GDA2020),
        )
        .unwrap(),
    );
    let built = build(&survey, &settings(), TransformationGraph::builtin()).unwrap();
    // The CSV's fixed MTHR wins over the RTKLIB header's free one.
    assert_eq!(
        built.datum,
        gnss_adjust::Datum::Constrained { fixed: vec![0] }
    );
    let steps = &built.transformations[0].steps;
    assert_eq!(steps.len(), 3, "{steps:?}");
    assert!(steps[2].contains("ITRF2014 to GDA2020 (1)"), "{steps:?}");
    let adjustment = built
        .network
        .adjust(&built.datum, &Options::default())
        .unwrap();
    let mthr = adjustment.marks()[0].position;
    let sngo = adjustment.marks()[1].position;
    let adjusted = sngo - mthr;
    let Element::Vector { vector: given, .. } = &survey.measurements[0].elements[0] else {
        panic!();
    };
    // The plate motion model turns the 9.6 km vector by 5.45 years of
    // rotation, about 0.4 mm, without changing its length.
    let turned = (adjusted - *given).norm();
    assert!((2e-4..6e-4).contains(&turned), "{turned}");
    assert!((adjusted.norm() - given.norm()).abs() < 1e-9);
}
