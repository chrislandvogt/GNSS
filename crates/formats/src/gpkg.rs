//! GeoPackage export (OGC 12-128r18, version 1.4): layers of points, lines
//! and polygons with attributes, in a frame's geographic CRS, for GIS.
//!
//! The file is written directly ([`crate::sqlite`]), with the metadata
//! tables the standard requires (`gpkg_spatial_ref_sys`, `gpkg_contents`
//! and `gpkg_geometry_columns`) and a feature table per layer. Geometries
//! are 2D, longitude then latitude in degrees, in GeoPackage's binary
//! encoding (little-endian, with an envelope for lines and polygons). There's
//! no spatial index, which the standard leaves optional.

use gnss_datum::FrameId;

use crate::error::WriteError;
use crate::sqlite::{Index, Table, Value, write as write_sqlite};

/// GeoPackage's application id, "GPKG", for the database header.
const APPLICATION_ID: u32 = 0x4750_4B47;
/// The version: 1.4.0.
const USER_VERSION: u32 = 10_400;

/// A geometry, in degrees: longitude, then latitude.
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    /// A point.
    Point([f64; 2]),
    /// A line through the points.
    LineString(Vec<[f64; 2]>),
    /// A polygon: its outer ring, closed (the last point repeats the first),
    /// then any holes.
    Polygon(Vec<Vec<[f64; 2]>>),
}

/// The kind of geometry a layer holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryType {
    /// Points.
    Point,
    /// Lines.
    LineString,
    /// Polygons.
    Polygon,
}

impl GeometryType {
    fn name(self) -> &'static str {
        match self {
            GeometryType::Point => "POINT",
            GeometryType::LineString => "LINESTRING",
            GeometryType::Polygon => "POLYGON",
        }
    }
}

/// An attribute's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    /// Text.
    Text,
    /// A real number.
    Real,
    /// An integer.
    Integer,
}

/// An attribute's value.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    /// No value.
    Null,
    /// Text.
    Text(String),
    /// A real number.
    Real(f64),
    /// An integer.
    Integer(i64),
}

/// A feature: its geometry and its attributes, in the layer's field order.
#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    /// The geometry.
    pub geometry: Geometry,
    /// The attributes.
    pub values: Vec<FieldValue>,
}

/// A layer: a feature table.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    /// The table's name: letters, digits and underscores.
    pub name: String,
    /// A description for `gpkg_contents`.
    pub description: String,
    /// The geometry its features have.
    pub geometry: GeometryType,
    /// The attribute names and types.
    pub fields: Vec<(String, FieldType)>,
    /// The features.
    pub features: Vec<Feature>,
}

/// A name for a table or column: letters, digits and underscores, starting
/// with a letter, so it never needs quoting.
fn plain(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The frame's geographic CRS as GDAL-style WKT 1.
fn wkt(frame: FrameId) -> String {
    let e = frame.ellipsoid();
    let (ellipsoid, ellipsoid_code) = if e.name() == "WGS 84" {
        ("WGS 84", 7030)
    } else {
        ("GRS 1980", 7019)
    };
    format!(
        "GEOGCS[\"{}\",DATUM[\"{}\",SPHEROID[\"{ellipsoid}\",{},{},AUTHORITY[\"EPSG\",\"{ellipsoid_code}\"]]],\
         PRIMEM[\"Greenwich\",0,AUTHORITY[\"EPSG\",\"8901\"]],\
         UNIT[\"degree\",0.0174532925199433,AUTHORITY[\"EPSG\",\"9122\"]],\
         AXIS[\"Latitude\",NORTH],AXIS[\"Longitude\",EAST],\
         AUTHORITY[\"EPSG\",\"{}\"]]",
        frame.name(),
        frame.datum_name().replace(' ', "_"),
        e.semi_major_axis_m(),
        e.inverse_flattening(),
        frame.geographic_epsg_code()
    )
}

/// GeoPackage's binary geometry: its header, then the geometry as WKB.
fn encode(geometry: &Geometry, srs_id: i32) -> Vec<u8> {
    let points: Vec<[f64; 2]> = match geometry {
        Geometry::Point(p) => vec![*p],
        Geometry::LineString(line) => line.clone(),
        Geometry::Polygon(rings) => rings.concat(),
    };
    let envelope = !matches!(geometry, Geometry::Point(_));
    let mut out = vec![b'G', b'P', 0, 0x01 | if envelope { 0x02 } else { 0 }];
    out.extend_from_slice(&srs_id.to_le_bytes());
    if envelope {
        let fold = |axis: usize, f: fn(f64, f64) -> f64, start: f64| {
            points.iter().map(|p| p[axis]).fold(start, f)
        };
        for v in [
            fold(0, f64::min, f64::INFINITY),
            fold(0, f64::max, f64::NEG_INFINITY),
            fold(1, f64::min, f64::INFINITY),
            fold(1, f64::max, f64::NEG_INFINITY),
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out.push(1);
    let coordinates = |out: &mut Vec<u8>, ps: &[[f64; 2]]| {
        for p in ps {
            out.extend_from_slice(&p[0].to_le_bytes());
            out.extend_from_slice(&p[1].to_le_bytes());
        }
    };
    match geometry {
        Geometry::Point(p) => {
            out.extend_from_slice(&1u32.to_le_bytes());
            coordinates(&mut out, &[*p]);
        }
        Geometry::LineString(line) => {
            out.extend_from_slice(&2u32.to_le_bytes());
            out.extend_from_slice(&(line.len() as u32).to_le_bytes());
            coordinates(&mut out, line);
        }
        Geometry::Polygon(rings) => {
            out.extend_from_slice(&3u32.to_le_bytes());
            out.extend_from_slice(&(rings.len() as u32).to_le_bytes());
            for ring in rings {
                out.extend_from_slice(&(ring.len() as u32).to_le_bytes());
                coordinates(&mut out, ring);
            }
        }
    }
    out
}

/// Writes layers as a GeoPackage in `frame`'s geographic CRS. `last_change`
/// is the time recorded for every layer, as `YYYY-MM-DDTHH:MM:SS.SSSZ`: the
/// caller supplies it, so the same results give the same file.
pub fn write(layers: &[Layer], frame: FrameId, last_change: &str) -> Result<Vec<u8>, WriteError> {
    let srs_id = frame.geographic_epsg_code() as i32;
    let text = |s: &str| Value::Text(s.to_owned());

    let mut srs_rows = vec![
        (
            -1,
            vec![
                text("Undefined cartesian SRS"),
                Value::Null,
                text("NONE"),
                Value::Integer(-1),
                text("undefined"),
                text("undefined cartesian coordinate reference system"),
            ],
        ),
        (
            0,
            vec![
                text("Undefined geographic SRS"),
                Value::Null,
                text("NONE"),
                Value::Integer(0),
                text("undefined"),
                text("undefined geographic coordinate reference system"),
            ],
        ),
        (
            4326,
            vec![
                text("WGS 84 geodetic"),
                Value::Null,
                text("EPSG"),
                Value::Integer(4326),
                text(&wkt(FrameId::Wgs84)),
                text("longitude/latitude coordinates in decimal degrees on the WGS 84 spheroid"),
            ],
        ),
    ];
    if srs_id != 4326 {
        srs_rows.push((
            i64::from(srs_id),
            vec![
                text(frame.name()),
                Value::Null,
                text("EPSG"),
                Value::Integer(i64::from(srs_id)),
                text(&wkt(frame)),
                text(&format!("{} geographic 2D", frame.name())),
            ],
        ));
    }

    let mut contents = Vec::new();
    let mut geometry_columns = Vec::new();
    let mut feature_tables = Vec::new();
    for (k, layer) in layers.iter().enumerate() {
        if !plain(&layer.name) || layer.name.starts_with("gpkg") {
            return Err(WriteError(format!(
                "layer name {:?} must be letters, digits and underscores, starting with a letter \
                 and not with gpkg",
                layer.name
            )));
        }
        for (name, _) in &layer.fields {
            if !plain(name) || name == "fid" || name == "geom" {
                return Err(WriteError(format!(
                    "field name {name:?} in layer {} must be letters, digits and underscores, \
                     starting with a letter, and not fid or geom",
                    layer.name
                )));
            }
        }
        let mut rows = Vec::with_capacity(layer.features.len());
        let (mut west, mut south) = (f64::INFINITY, f64::INFINITY);
        let (mut east, mut north) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for (i, feature) in layer.features.iter().enumerate() {
            let matches = matches!(
                (&feature.geometry, layer.geometry),
                (Geometry::Point(_), GeometryType::Point)
                    | (Geometry::LineString(_), GeometryType::LineString)
                    | (Geometry::Polygon(_), GeometryType::Polygon)
            );
            if !matches || feature.values.len() != layer.fields.len() {
                return Err(WriteError(format!(
                    "feature {} of layer {} doesn't match the layer's geometry and fields",
                    i + 1,
                    layer.name
                )));
            }
            let points: Vec<[f64; 2]> = match &feature.geometry {
                Geometry::Point(p) => vec![*p],
                Geometry::LineString(l) => l.clone(),
                Geometry::Polygon(r) => r.concat(),
            };
            for p in &points {
                if !p[0].is_finite() || !p[1].is_finite() {
                    return Err(WriteError(format!(
                        "feature {} of layer {} has a coordinate that isn't finite",
                        i + 1,
                        layer.name
                    )));
                }
                west = west.min(p[0]);
                east = east.max(p[0]);
                south = south.min(p[1]);
                north = north.max(p[1]);
            }
            let mut values = vec![Value::Null, Value::Blob(encode(&feature.geometry, srs_id))];
            for (value, (_, kind)) in feature.values.iter().zip(&layer.fields) {
                values.push(match (value, kind) {
                    (FieldValue::Null, _) => Value::Null,
                    (FieldValue::Text(t), FieldType::Text) => Value::Text(t.clone()),
                    (FieldValue::Real(r), FieldType::Real) if r.is_finite() => Value::Real(*r),
                    (FieldValue::Real(_), FieldType::Real) => Value::Null,
                    (FieldValue::Integer(n), FieldType::Integer) => Value::Integer(*n),
                    _ => {
                        return Err(WriteError(format!(
                            "feature {} of layer {} has a value of the wrong type",
                            i + 1,
                            layer.name
                        )));
                    }
                });
            }
            rows.push((i as i64 + 1, values));
        }
        let bounds = |v: f64| {
            if v.is_finite() {
                Value::Real(v)
            } else {
                Value::Null
            }
        };
        contents.push((
            k as i64 + 1,
            vec![
                text(&layer.name),
                text("features"),
                text(&layer.name),
                text(&layer.description),
                text(last_change),
                bounds(west),
                bounds(south),
                bounds(east),
                bounds(north),
                Value::Integer(i64::from(srs_id)),
            ],
        ));
        geometry_columns.push((
            k as i64 + 1,
            vec![
                text(&layer.name),
                text("geom"),
                text(layer.geometry.name()),
                Value::Integer(i64::from(srs_id)),
                Value::Integer(0),
                Value::Integer(0),
            ],
        ));
        let columns: String = layer
            .fields
            .iter()
            .map(|(name, kind)| {
                let t = match kind {
                    FieldType::Text => "TEXT",
                    FieldType::Real => "REAL",
                    FieldType::Integer => "INTEGER",
                };
                format!(", {name} {t}")
            })
            .collect();
        feature_tables.push(Table {
            name: layer.name.clone(),
            sql: format!(
                "CREATE TABLE {} (fid INTEGER PRIMARY KEY NOT NULL, geom {}{columns})",
                layer.name,
                layer.geometry.name()
            ),
            rows,
            indexes: Vec::new(),
        });
    }

    let mut tables = vec![
        Table {
            name: "gpkg_spatial_ref_sys".into(),
            sql: "CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL, srs_id INTEGER \
                  PRIMARY KEY, organization TEXT NOT NULL, organization_coordsys_id INTEGER NOT \
                  NULL, definition TEXT NOT NULL, description TEXT)"
                .into(),
            rows: srs_rows,
            indexes: Vec::new(),
        },
        Table {
            name: "gpkg_contents".into(),
            sql: "CREATE TABLE gpkg_contents (table_name TEXT NOT NULL PRIMARY KEY, data_type \
                  TEXT NOT NULL, identifier TEXT UNIQUE, description TEXT DEFAULT '', \
                  last_change DATETIME NOT NULL DEFAULT \
                  (strftime('%Y-%m-%dT%H:%M:%fZ','now')), min_x DOUBLE, min_y DOUBLE, max_x \
                  DOUBLE, max_y DOUBLE, srs_id INTEGER, CONSTRAINT fk_gc_r_srs_id FOREIGN KEY \
                  (srs_id) REFERENCES gpkg_spatial_ref_sys(srs_id))"
                .into(),
            rows: contents,
            indexes: vec![
                Index {
                    name: "sqlite_autoindex_gpkg_contents_1".into(),
                    sql: None,
                    columns: vec![0],
                },
                Index {
                    name: "sqlite_autoindex_gpkg_contents_2".into(),
                    sql: None,
                    columns: vec![2],
                },
            ],
        },
        Table {
            name: "gpkg_geometry_columns".into(),
            sql: "CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL, column_name \
                  TEXT NOT NULL, geometry_type_name TEXT NOT NULL, srs_id INTEGER NOT NULL, z \
                  TINYINT NOT NULL, m TINYINT NOT NULL, CONSTRAINT pk_geom_cols PRIMARY KEY \
                  (table_name, column_name), CONSTRAINT uk_gc_table_name UNIQUE (table_name), \
                  CONSTRAINT fk_gc_tn FOREIGN KEY (table_name) REFERENCES \
                  gpkg_contents(table_name), CONSTRAINT fk_gc_srs FOREIGN KEY (srs_id) \
                  REFERENCES gpkg_spatial_ref_sys (srs_id))"
                .into(),
            rows: geometry_columns,
            indexes: vec![
                Index {
                    name: "sqlite_autoindex_gpkg_geometry_columns_1".into(),
                    sql: None,
                    columns: vec![0, 1],
                },
                Index {
                    name: "sqlite_autoindex_gpkg_geometry_columns_2".into(),
                    sql: None,
                    columns: vec![0],
                },
            ],
        },
    ];
    tables.extend(feature_tables);
    write_sqlite(&tables, USER_VERSION, APPLICATION_ID).map_err(|e| WriteError(e.0))
}
