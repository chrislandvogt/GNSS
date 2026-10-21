//! Reading and writing DynaML: DynAdjust's sample network, round trips, and
//! what happens with bad files.

#![allow(clippy::unwrap_used)]

use gnss_datum::FrameId;
use gnss_formats::dynaml::{read, write_measurements, write_stations};
use gnss_formats::{
    Constraint, CoordinateValue, Element, HeightType, MeasurementKind, Reference, Survey,
};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const STATIONS: &str = include_str!("data/dynaml/gnss-networkstn.xml");
const MEASUREMENTS: &str = include_str!("data/dynaml/gnss-networkmsr.xml");

const GDA2020: Reference = Reference {
    frame: FrameId::Gda2020,
    epoch: None,
};

#[test]
fn reads_dynadjusts_sample() {
    let stations = read("stn.xml", STATIONS).unwrap();
    assert_eq!(stations.stations.len(), 43);
    let first = &stations.stations[0];
    assert_eq!(first.name, "211300470");
    assert_eq!(first.description.as_deref(), Some("BENALLA PM   47"));
    assert_eq!(first.constraint, Constraint::Free);
    let coordinate = first.coordinate.as_ref().unwrap();
    assert_eq!(coordinate.reference, GDA2020);
    let CoordinateValue::Geographic {
        latitude_rad,
        longitude_rad,
        height_m,
        height,
    } = coordinate.value
    else {
        panic!("{coordinate:?}");
    };
    // -36.3348253511 and 145.5741006918 in HP notation.
    let close = |a: f64, b: f64| (a - b).abs() < 1e-13;
    assert!(close(
        latitude_rad.to_degrees(),
        -(36.0 + 33.0 / 60.0 + 48.253511 / 3600.0)
    ));
    assert!(close(
        longitude_rad.to_degrees(),
        145.0 + 57.0 / 60.0 + 41.006918 / 3600.0
    ));
    assert_eq!((height_m, height), (172.1735, HeightType::Orthometric));

    let measurements = read("msr.xml", MEASUREMENTS).unwrap();
    let kinds = |k: MeasurementKind| {
        measurements
            .measurements
            .iter()
            .filter(|m| m.kind == k)
            .count()
    };
    assert_eq!(kinds(MeasurementKind::Baseline), 129);
    assert_eq!(kinds(MeasurementKind::BaselineCluster), 1);
    assert_eq!(kinds(MeasurementKind::PointCluster), 1);

    // The baseline cluster: four baselines with their correlations.
    let cluster = measurements
        .measurements
        .iter()
        .find(|m| m.kind == MeasurementKind::BaselineCluster)
        .unwrap();
    assert_eq!(cluster.elements.len(), 4);
    assert_eq!(cluster.reference.frame, FrameId::Itrf2014);
    assert_eq!(cluster.scaling.v, 8.95);
    assert_eq!(cluster.source.as_deref(), Some("18017.SNX.VIC.NGCA"));
    let c = &cluster.covariance;
    assert_eq!(c[(0, 0)], 9.4015092194406e-06);
    assert_eq!(c[(1, 0)], -5.8218280654235e-06);
    // The first block after the first baseline's variances: m12 is the
    // covariance of the first baseline's X with the second's Y, and m21 of
    // its Y with the second's X.
    assert_eq!(c[(0, 4)], -3.1969114896831e-06);
    assert_eq!(c[(1, 3)], -3.2013529554219e-06);
    assert_eq!(c[(4, 0)], c[(0, 4)]);
    assert!((c - c.transpose()).amax() == 0.0);

    // The point cluster: six positions in GDA2020.
    let points = measurements
        .measurements
        .iter()
        .find(|m| m.kind == MeasurementKind::PointCluster)
        .unwrap();
    assert_eq!(points.elements.len(), 6);
    assert_eq!(points.reference, GDA2020);
    assert!(matches!(&points.elements[0], Element::Position { station, .. } if station == "BEEC"));
}

#[test]
fn measurements_round_trip_exactly() {
    let original = read("msr.xml", MEASUREMENTS).unwrap();
    let written = write_measurements(&original.measurements, GDA2020);
    let again = read("again.xml", &written).unwrap();
    assert_eq!(again.measurements.len(), original.measurements.len());
    for (a, b) in original.measurements.iter().zip(&again.measurements) {
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.elements, b.elements);
        assert_eq!(a.covariance, b.covariance);
        assert_eq!(a.reference, b.reference);
        assert_eq!(a.scaling, b.scaling);
        assert_eq!(a.ignored, b.ignored);
        assert_eq!(a.source, b.source);
        assert_eq!(a.id, b.id);
    }
    // And writing what was read back gives the same text.
    assert_eq!(write_measurements(&again.measurements, GDA2020), written);
}

#[test]
fn stations_round_trip() {
    let original = read("stn.xml", STATIONS).unwrap();
    let written = write_stations(&original.stations, GDA2020).unwrap();
    let again = read("again.xml", &written).unwrap();
    assert_eq!(again.stations.len(), original.stations.len());
    for (a, b) in original.stations.iter().zip(&again.stations) {
        assert_eq!(
            (&a.name, &a.description, &a.constraint),
            (&b.name, &b.description, &b.constraint)
        );
        let (ca, cb) = (
            a.coordinate.as_ref().unwrap(),
            b.coordinate.as_ref().unwrap(),
        );
        assert_eq!(ca.reference, cb.reference);
        match (ca.value, cb.value) {
            (
                CoordinateValue::Geographic {
                    latitude_rad: la,
                    longitude_rad: oa,
                    height_m: ha,
                    height: ta,
                },
                CoordinateValue::Geographic {
                    latitude_rad: lb,
                    longitude_rad: ob,
                    height_m: hb,
                    height: tb,
                },
            ) => {
                // HP notation to a microsecond of arc: the file's own precision.
                assert!(
                    (la - lb).abs() < 1e-14 && (oa - ob).abs() < 1e-14,
                    "{}",
                    a.name
                );
                assert_eq!((ha, ta), (hb, tb));
            }
            (CoordinateValue::Cartesian(xa), CoordinateValue::Cartesian(xb)) => {
                assert_eq!(xa, xb, "{}", a.name);
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn writes_every_kind_of_station() {
    let text = r#"<DnaXmlFormat type="Station File" referenceframe="ITRF2014" epoch="17.01.2018">
  <DnaStation><Name>A &amp; B</Name><Constraints>CCC</Constraints><Type>XYZ</Type>
    <StationCoord><Name>A</Name><XAxis>-4052052.7373</XAxis><YAxis>4212835.9835</YAxis><Height>-2545104.5867</Height></StationCoord></DnaStation>
  <DnaStation><Name>C</Name><Constraints>CCF</Constraints><Type>UTM</Type>
    <StationCoord><Name>C</Name><XAxis>372772.813</XAxis><YAxis>5931491.145</YAxis><Height>175.668</Height><HemisphereZone>S55</HemisphereZone></StationCoord></DnaStation>
  <DnaStation><Name>D</Name><Constraints>FFF</Constraints><Type>LLh</Type>
    <StationCoord><Name>D</Name><XAxis>-37.4801765569</XAxis><YAxis>144.5717295509</YAxis><Height>83.9</Height></StationCoord></DnaStation>
</DnaXmlFormat>"#;
    let survey = read("s.xml", text).unwrap();
    let reference = survey.stations[0].coordinate.as_ref().unwrap().reference;
    assert_eq!(reference.frame, FrameId::Itrf2014);
    assert_eq!(survey.stations[0].name, "A & B");
    assert_eq!(
        survey.stations[1].constraint,
        Constraint::Partial("CCF".into())
    );
    assert!(matches!(
        survey.stations[1].coordinate.as_ref().unwrap().value,
        CoordinateValue::Grid {
            zone: 55,
            height: HeightType::Orthometric,
            ..
        }
    ));
    assert!(matches!(
        survey.stations[2].coordinate.as_ref().unwrap().value,
        CoordinateValue::Geographic {
            height: HeightType::Ellipsoidal,
            ..
        }
    ));
    let written = write_stations(&survey.stations, reference).unwrap();
    assert!(written.contains("referenceframe=\"ITRF2014\" epoch=\"17.01.2018\""));
    assert!(written.contains("<Name>A &amp; B</Name>"));
    assert!(written.contains("<HemisphereZone>S55</HemisphereZone>"));
    let again = read("again.xml", &written).unwrap();
    for (a, b) in survey.stations.iter().zip(&again.stations) {
        assert_eq!(
            (&a.name, &a.description, &a.coordinate, &a.constraint),
            (&b.name, &b.description, &b.coordinate, &b.constraint)
        );
    }
    // Stations must be in the file's frame.
    let err = write_stations(&survey.stations, GDA2020).unwrap_err();
    assert_eq!(
        err.to_string(),
        "station A & B is in ITRF2014 at 2018.0438, but the file is in GDA2020"
    );
}

/// A measurement file around one measurement's elements.
fn measurement(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?>\n<DnaXmlFormat type=\"Measurement File\" referenceframe=\"GDA2020\" \
         epoch=\"01.01.2020\">\n<DnaMeasurement>\n{body}\n</DnaMeasurement>\n</DnaXmlFormat>\n"
    )
}

const BASELINE: &str = "<X>1</X><Y>2</Y><Z>3</Z><SigmaXX>1e-6</SigmaXX><SigmaXY>0</SigmaXY>\
    <SigmaXZ>0</SigmaXZ><SigmaYY>1e-6</SigmaYY><SigmaYZ>0</SigmaYZ><SigmaZZ>1e-6</SigmaZZ>";

fn error(text: &str) -> String {
    read("m.xml", text).unwrap_err().to_string()
}

#[test]
fn explains_bad_measurements() {
    let g = |extra: &str, values: &str| {
        measurement(&format!(
            "<Type>G</Type>{extra}<First>A</First><Second>B</Second><GPSBaseline>{values}</GPSBaseline>"
        ))
    };
    assert_eq!(
        error(&g("", &BASELINE.replace("<X>1</X>", "<X>one</X>"))),
        "m.xml, line 4: X isn't a number: \"one\""
    );
    assert_eq!(
        error(&g("", &BASELINE.replace("<SigmaZZ>1e-6</SigmaZZ>", ""))),
        "m.xml, line 4: <GPSBaseline> has no <SigmaZZ>"
    );
    assert_eq!(
        error(&g("<ReferenceFrame>ITRF1992</ReferenceFrame>", BASELINE)),
        "m.xml, line 3: unknown frame \"ITRF1992\". Known frames: ITRF2020, IGS20, ITRF2014, \
         ITRF2008, ITRF2005, ATRF2014, GDA2020, GDA94, WGS 84 (G2296), WGS 84 (G2139), WGS 84"
    );
    assert_eq!(
        error(&g(
            "<ReferenceFrame>ITRF2014</ReferenceFrame><Epoch>31.02.2018</Epoch>",
            BASELINE
        )),
        "m.xml, line 3: the epoch isn't a date in dd.mm.yyyy form: \"31.02.2018\""
    );
    assert_eq!(
        error(&measurement(&format!(
            "<Type>G</Type><First>A</First><Second>A</Second><GPSBaseline>{BASELINE}</GPSBaseline>"
        ))),
        "m.xml, line 4: the baseline from A ends at A too"
    );
    assert_eq!(
        error(&measurement(&format!(
            "<Type>G</Type><First>A</First><GPSBaseline>{BASELINE}</GPSBaseline>"
        ))),
        "m.xml, line 4: <GPSBaseline> has no <Second> station before it"
    );
    assert_eq!(
        error(&measurement(
            "<Type>G</Type><First>A</First><Second>B</Second>"
        )),
        "m.xml, line 3: the type G measurement has no <GPSBaseline>"
    );
    // A cluster needs a covariance block for each later baseline.
    let two = |blocks: &str| {
        measurement(&format!(
            "<Type>X</Type><Total>2</Total>\n<First>A</First><Second>B</Second>\
             <GPSBaseline>{BASELINE}{blocks}</GPSBaseline>\n<First>B</First><Second>C</Second>\
             <GPSBaseline>{BASELINE}</GPSBaseline>"
        ))
    };
    assert_eq!(
        error(&two("")),
        "m.xml, line 5: element 1 of 2 has 0 covariance blocks, but needs one for each later element (1)"
    );
    let block = "<GPSCovariance><m11>1e-7</m11><m12>2e-7</m12><m13>0</m13><m21>3e-7</m21>\
                 <m22>0</m22><m23>0</m23><m31>0</m31><m32>0</m32><m33>0</m33></GPSCovariance>";
    let survey = read("m.xml", &two(block)).unwrap();
    let c = &survey.measurements[0].covariance;
    assert_eq!((c[(0, 3)], c[(0, 4)], c[(1, 3)]), (1e-7, 2e-7, 3e-7));
    assert_eq!((c[(3, 0)], c[(4, 0)], c[(3, 1)]), (1e-7, 2e-7, 3e-7));
    assert_eq!(
        error(&two(block).replace("<Total>2</Total>", "<Total>3</Total>")),
        "m.xml, line 3: <Total> says 3, but the cluster has 2"
    );
    assert_eq!(
        error(&measurement(&format!(
            "<Type>G</Type><First>A</First><Second>B</Second><GPSBaseline>{BASELINE}</GPSBaseline>\
             <First>B</First><Second>C</Second><GPSBaseline>{BASELINE}</GPSBaseline>"
        ))),
        "m.xml, line 3: a type G measurement holds one baseline, but this one has 2: use type X"
    );
}

#[test]
fn notes_what_it_skips_or_assumes() {
    let text = format!(
        "<DnaXmlFormat type=\"Measurement File\" referenceframe=\"GDA2020\" epoch=\"18.02.2015\">\n\
         <DnaMeasurement><Type>S</Type><First>A</First><Second>B</Second><Value>10</Value></DnaMeasurement>\n\
         <DnaMeasurement><Type>S</Type><First>A</First><Second>B</Second><Value>10</Value></DnaMeasurement>\n\
         <DnaMeasurement><Type>L</Type><First>A</First><Second>B</Second><Value>1</Value></DnaMeasurement>\n\
         <DnaMeasurement><Type>G</Type><Ignore>*</Ignore><Vscale>0</Vscale><Hscale>2.5</Hscale>\
         <First>A</First><Second>B</Second><GPSBaseline>{BASELINE}</GPSBaseline></DnaMeasurement>\n\
         <Something/>\n</DnaXmlFormat>"
    );
    let survey = read("m.xml", &text).unwrap();
    let notes: Vec<String> = survey
        .notes
        .iter()
        .map(|n| format!("{}: {}", n.origin, n.message))
        .collect();
    assert_eq!(
        notes,
        [
            "m.xml, line 1: GDA2020 is a static frame, so its epoch is 2020.0; the file's \
             epoch 18.02.2015 was ignored",
            "m.xml, line 5: Vscale 0 was taken as 1, as DynAdjust does",
            "m.xml, line 6: skipped <Something>, which isn't a DynaML station or measurement",
            "m.xml, line 4: skipped 1 measurement of type L (level difference): only GNSS \
             measurements (G, X and Y) are adjusted so far",
            "m.xml, line 2: skipped 2 measurements of type S (slope distance): only GNSS \
             measurements (G, X and Y) are adjusted so far",
        ]
    );
    let m = &survey.measurements[0];
    assert!(m.ignored);
    assert_eq!((m.scaling.v, m.scaling.h, m.reference), (1.0, 2.5, GDA2020));
}

#[test]
fn rejects_other_files() {
    assert_eq!(
        error("<LandXML/>"),
        "m.xml, line 1: it isn't a DynaML file: the root element is <LandXML>, not <DnaXmlFormat>"
    );
    assert_eq!(
        error("<DnaXmlFormat referenceframe=\"ITRF2014\"></DnaXmlFormat>"),
        "m.xml, line 1: ITRF2014 is a dynamic frame, so it needs an epoch, but none is given"
    );
    assert!(read("m.xml", "<DnaXmlFormat/>").unwrap() == Survey::default());
    assert_eq!(
        error(
            "<DnaXmlFormat><DnaStation><Name>A</Name><Type>LLH</Type></DnaStation></DnaXmlFormat>"
        ),
        "m.xml, line 1: station A has no <StationCoord>"
    );
    assert_eq!(
        error(
            "<DnaXmlFormat><DnaStation><Name>A</Name><Constraints>CXC</Constraints>\
             <Type>LLH</Type></DnaStation></DnaXmlFormat>"
        ),
        "m.xml, line 1: station A: constraints \"CXC\" should be three letters, C (held) or F (free)"
    );
}
