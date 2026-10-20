//! GeoPackage export. The file's structure is checked here; SQLite's
//! integrity check and GDAL check the files the ignored `write_samples` test
//! writes (`tests/oracle/check_gpkg.py`, run in CI).

#![allow(clippy::unwrap_used)]

use gnss_datum::FrameId;
use gnss_formats::gpkg::{Feature, FieldType, FieldValue, Geometry, GeometryType, Layer, write};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const WHEN: &str = "2026-10-20T00:00:00.000Z";

/// Layers big enough to need interior pages (5,000 points) and overflow
/// pages (polygons of 400 points), with an empty one.
fn layers() -> Vec<Layer> {
    let marks = Layer {
        name: "marks".into(),
        description: "Adjusted marks".into(),
        geometry: GeometryType::Point,
        fields: vec![
            ("name".into(), FieldType::Text),
            ("height".into(), FieldType::Real),
            ("fixed".into(), FieldType::Integer),
        ],
        features: (0..5000)
            .map(|i| Feature {
                geometry: Geometry::Point([
                    150.0 + f64::from(i) * 1e-4,
                    -33.0 - f64::from(i) * 1e-4,
                ]),
                values: vec![
                    FieldValue::Text(format!("PM{i}")),
                    FieldValue::Real(f64::from(i) * 0.5),
                    FieldValue::Integer(i64::from(i % 2)),
                ],
            })
            .collect(),
    };
    let shapes = Layer {
        name: "shapes".into(),
        description: String::new(),
        geometry: GeometryType::Polygon,
        fields: vec![("name".into(), FieldType::Text)],
        features: (0..50)
            .map(|i| {
                let x = 150.0 + f64::from(i) * 0.1;
                let mut ring: Vec<[f64; 2]> = (0..400)
                    .map(|k| {
                        let t = f64::from(k) / 400.0;
                        [x + 0.01 * t, -33.0 + 0.005 * t * (1.0 - t)]
                    })
                    .collect();
                ring.push(ring[0]);
                Feature {
                    geometry: Geometry::Polygon(vec![ring]),
                    values: vec![FieldValue::Text(format!("S{i}"))],
                }
            })
            .collect(),
    };
    let lines = Layer {
        name: "vectors".into(),
        description: "GNSS vectors".into(),
        geometry: GeometryType::LineString,
        fields: vec![("id".into(), FieldType::Text)],
        features: vec![Feature {
            geometry: Geometry::LineString(vec![[150.0, -33.0], [150.1, -33.1]]),
            values: vec![FieldValue::Null],
        }],
    };
    let empty = Layer {
        name: "empty".into(),
        description: String::new(),
        geometry: GeometryType::Point,
        fields: Vec::new(),
        features: Vec::new(),
    };
    vec![marks, shapes, lines, empty]
}

#[test]
fn writes_a_geopackage_header_and_the_same_bytes_each_time() {
    let bytes = write(&layers(), FrameId::Gda2020, WHEN).unwrap();
    assert_eq!(&bytes[..16], b"SQLite format 3\0");
    assert_eq!(bytes.len() % 4096, 0);
    let page_count = u32::from_be_bytes(bytes[28..32].try_into().unwrap()) as usize;
    assert_eq!(page_count * 4096, bytes.len());
    // GeoPackage's application id ("GPKG") and version 1.4.
    assert_eq!(&bytes[68..72], b"GPKG");
    assert_eq!(
        u32::from_be_bytes(bytes[60..64].try_into().unwrap()),
        10_400
    );
    assert_eq!(bytes, write(&layers(), FrameId::Gda2020, WHEN).unwrap());
}

#[test]
fn refuses_what_it_cant_write() {
    let mut bad = layers();
    bad[0].name = "my marks".into();
    assert!(
        write(&bad, FrameId::Gda2020, WHEN)
            .unwrap_err()
            .0
            .contains("layer name")
    );
    let mut bad = layers();
    bad[2].fields[0].0 = "fid".into();
    assert!(
        write(&bad, FrameId::Gda2020, WHEN)
            .unwrap_err()
            .0
            .contains("not fid or geom")
    );
    let mut bad = layers();
    bad[0].features[3].values[1] = FieldValue::Text("tall".into());
    assert_eq!(
        write(&bad, FrameId::Gda2020, WHEN).unwrap_err().0,
        "feature 4 of layer marks has a value of the wrong type"
    );
    let mut bad = layers();
    bad[2].features[0].geometry = Geometry::Point([0.0, 0.0]);
    assert_eq!(
        write(&bad, FrameId::Gda2020, WHEN).unwrap_err().0,
        "feature 1 of layer vectors doesn't match the layer's geometry and fields"
    );
    let mut bad = layers();
    bad[2].features[0].geometry = Geometry::LineString(vec![[f64::NAN, 0.0]]);
    assert!(
        write(&bad, FrameId::Gda2020, WHEN)
            .unwrap_err()
            .0
            .contains("isn't finite")
    );
}

/// Writes the samples to `$GNSS_GPKG_DIR` for `tests/oracle/check_gpkg.py`.
#[test]
#[ignore = "writes files for the oracle script"]
fn write_samples() {
    let directory = std::env::var("GNSS_GPKG_DIR").unwrap();
    for frame in [FrameId::Gda2020, FrameId::Itrf2014, FrameId::Wgs84] {
        let path = format!("{directory}/{}.gpkg", frame.geographic_epsg_code());
        std::fs::write(path, write(&layers(), frame, WHEN).unwrap()).unwrap();
    }
}
