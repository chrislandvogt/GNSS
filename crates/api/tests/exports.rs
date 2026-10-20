//! Result exports, on DynAdjust's sample network adjusted in GDA2020: each
//! reads back as what it should be. GDAL checks the GeoPackage
//! (`crates/formats/tests/oracle/check_gpkg.py`).

#![allow(clippy::unwrap_used)]

use gnss_adjust::{Adjustment, Options};
use gnss_api::build::{Built, DatumChoice, Settings, build};
use gnss_api::export::{self, Results, Scaling};
use gnss_datum::{FrameId, TransformationGraph};
use gnss_formats::{Constraint, CoordinateValue, Survey, csv, dynaml, starnet};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const STATIONS: &str = include_str!("../../formats/tests/data/dynaml/gnss-networkstn.xml");
const MEASUREMENTS: &str = include_str!("../../formats/tests/data/dynaml/gnss-networkmsr.xml");

fn adjusted() -> (Built, Adjustment) {
    let mut survey: Survey = dynaml::read("gnss-networkstn.xml", STATIONS).unwrap();
    survey.extend(dynaml::read("gnss-networkmsr.xml", MEASUREMENTS).unwrap());
    let settings = Settings {
        frame: FrameId::Gda2020,
        epoch: None,
        datum: DatumChoice::Constrained,
    };
    let built = build(&survey, &settings, TransformationGraph::builtin()).unwrap();
    let adjustment = built
        .network
        .adjust(
            &built.datum,
            &Options {
                edits: built.edits.clone(),
                ..Options::default()
            },
        )
        .unwrap();
    (built, adjustment)
}

fn results<'a>(built: &'a Built, adjustment: &'a Adjustment) -> Results<'a> {
    Results {
        built,
        adjustment,
        scaling: Scaling::APriori,
    }
}

#[test]
fn csv_exports_read_back() {
    let (built, adjustment) = adjusted();
    let r = results(&built, &adjustment);
    let marks = export::marks_csv(&r).unwrap();
    assert_eq!(marks.lines().count(), 44);
    assert!(marks.starts_with("name,x,y,z,latitude,longitude,height,zone,easting,northing,"));
    // They read back as control, at the adjusted positions to 0.1 mm.
    let control = csv::read_control("marks.csv", &marks, None).unwrap();
    assert_eq!(control.stations.len(), 43);
    for (station, mark) in control.stations.iter().zip(adjustment.marks()) {
        assert_eq!(station.constraint, Constraint::Free);
        let CoordinateValue::Cartesian(xyz) = station.coordinate.as_ref().unwrap().value else {
            panic!();
        };
        assert!((xyz - mark.position).amax() <= 5e-5, "{}", station.name);
    }

    let observations = export::observations_csv(&r);
    assert_eq!(observations.lines().count(), 1 + 139);
    let first = observations.lines().nth(1).unwrap();
    assert!(
        first.starts_with("gnss-networkmsr.xml:21,1,324900360,BEEC,"),
        "{first}"
    );
    assert!(first.ends_with(",yes,no,gnss-networkmsr.xml:21"), "{first}");
}

#[test]
fn landxml_holds_mga_control_points() {
    let (built, adjustment) = adjusted();
    let r = results(&built, &adjustment);
    let text = export::landxml(&r, "2026-10-20", "08:00:00").unwrap();
    // Bright, in Victoria, is in zone 55.
    assert!(text.contains(
        "<CoordinateSystem name=\"GDA2020 / MGA zone 55\" epsgCode=\"7855\" horizontalDatum=\"GDA2020\""
    ));
    assert_eq!(text.matches("<CgPoint ").count(), 43);
    let line = text.lines().find(|l| l.contains("name=\"BEEC\"")).unwrap();
    let values: Vec<f64> = line
        .split('>')
        .nth(1)
        .unwrap()
        .split('<')
        .next()
        .unwrap()
        .split_whitespace()
        .map(|v| v.parse().unwrap())
        .collect();
    let mark = built.network.mark("BEEC").unwrap();
    let position = adjustment.position(mark, Scaling::APriori).unwrap();
    let zone = gnss_datum::tm::MgaZone::new(55).unwrap();
    let (mga, _) = gnss_datum::tm::to_mga(&position, Some(zone)).unwrap();
    assert!((values[0] - mga.northing_m).abs() <= 5e-5);
    assert!((values[1] - mga.easting_m).abs() <= 5e-5);
    assert_eq!(values.len(), 3);

    // Outside GDA2020 and GDA94 there's no MGA.
    let mut itrf = Survey::default();
    itrf.extend(dynaml::read("gnss-networkstn.xml", STATIONS).unwrap());
    itrf.extend(dynaml::read("gnss-networkmsr.xml", MEASUREMENTS).unwrap());
    let epoch = gnss_datum::CoordinateEpoch::from_decimal_year(2026.5).ok();
    let built = build(
        &itrf,
        &Settings {
            frame: FrameId::Itrf2014,
            epoch,
            datum: DatumChoice::Minimal,
        },
        TransformationGraph::builtin(),
    )
    .unwrap();
    let adjustment = built
        .network
        .adjust(&built.datum, &Options::default())
        .unwrap();
    let err = export::landxml(&results(&built, &adjustment), "2026-10-20", "08:00:00").unwrap_err();
    assert!(
        err.0.contains("MGA, which is defined on GDA2020 and GDA94"),
        "{err}"
    );
}

#[test]
fn dynaml_and_starnet_exports_carry_the_adjustment() {
    let (built, adjustment) = adjusted();
    let r = results(&built, &adjustment);
    let (stations, measurements) = export::dynaml(&r).unwrap();
    let mut again = dynaml::read("stations.xml", &stations).unwrap();
    again.extend(dynaml::read("measurements.xml", &measurements).unwrap());
    assert_eq!(again.stations.len(), 43);
    assert_eq!(again.measurements.len(), 131);
    // Adjusting the export again gives the same marks: it's the same network,
    // starting from the adjusted positions.
    let settings = Settings {
        frame: FrameId::Gda2020,
        epoch: None,
        datum: DatumChoice::Constrained,
    };
    let rebuilt = build(&again, &settings, TransformationGraph::builtin()).unwrap();
    let readjusted = rebuilt
        .network
        .adjust(&rebuilt.datum, &Options::default())
        .unwrap();
    for (a, b) in adjustment.marks().iter().zip(readjusted.marks()) {
        assert!((a.position - b.position).amax() < 1e-9);
    }

    let vectors = export::starnet(&r).unwrap();
    let read = starnet::read(
        "vectors.gps",
        &vectors,
        gnss_formats::Reference {
            frame: FrameId::Gda2020,
            epoch: None,
        },
    )
    .unwrap();
    assert_eq!(read.measurements.len(), 133);
}

#[test]
fn geopackage_has_marks_vectors_and_ellipses() {
    let (built, adjustment) = adjusted();
    let r = results(&built, &adjustment);
    let bytes = export::geopackage(&r, 1000.0, "2026-10-20T08:00:00.000Z").unwrap();
    assert_eq!(&bytes[68..72], b"GPKG");
    assert_eq!(
        bytes,
        export::geopackage(&r, 1000.0, "2026-10-20T08:00:00.000Z").unwrap()
    );
    assert!(export::geopackage(&r, 0.0, "x").is_err());
    if let Ok(directory) = std::env::var("GNSS_GPKG_DIR") {
        std::fs::write(format!("{directory}/sample-network.gpkg"), bytes).unwrap();
    }
}
