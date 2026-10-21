//! From a built network and its adjustment back to the exchange formats'
//! terms: measurements as they were adjusted (in the network's frame and
//! epoch, with scaling applied) and marks at their adjusted positions.

use std::collections::BTreeMap;

use gnss_adjust::{Action, Adjustment, Datum, Network, Uncertainty};
use gnss_datum::Geodetic;
use gnss_datum::tm::{MgaGrid, MgaZone, to_mga};
use gnss_formats::gpkg::{Feature, FieldType, FieldValue, Geometry, GeometryType, Layer};
use gnss_formats::landxml::{ControlPoint, CoordinateSystem};
use gnss_formats::{
    Constraint, Coordinate, CoordinateValue, Element, Measurement, Origin, Reference, Station,
};
use nalgebra::Vector3;

pub use gnss_adjust::Scaling;

use crate::build::Built;

/// The network's frame and epoch as a reference.
pub fn reference(network: &Network) -> Reference {
    let frame = network.frame();
    Reference {
        frame,
        epoch: (!frame.is_static()).then_some(network.epoch()),
    }
}

/// The observations as adjusted: in the network's frame and epoch, with their
/// variance scaling applied, and marked ignored where an edit excludes them.
/// Each keeps the identifier it was adjusted under.
pub fn measurements(built: &Built) -> Vec<Measurement> {
    let network = &built.network;
    let name = |mark: usize| network.marks()[mark].name.clone();
    let excluded = |id: &str| {
        built
            .edits
            .iter()
            .rev()
            .find(|e| e.observation == id)
            .is_some_and(|e| e.action == Action::Exclude)
    };
    network
        .observations()
        .iter()
        .zip(&built.sources)
        .map(|(o, source)| Measurement {
            kind: source.kind,
            elements: o
                .elements
                .iter()
                .map(|e| match *e {
                    gnss_adjust::Element::Difference {
                        from, to, vector, ..
                    } => Element::Vector {
                        from: name(from),
                        to: name(to),
                        vector,
                    },
                    gnss_adjust::Element::Position { mark, position, .. } => Element::Position {
                        station: name(mark),
                        position,
                    },
                })
                .collect(),
            covariance: o.covariance.clone(),
            reference: reference(network),
            scaling: gnss_formats::Scaling::NONE,
            ignored: excluded(&o.id),
            source: None,
            id: Some(o.id.clone()),
            origin: Origin {
                file: source.origin.file.clone(),
                line: source.origin.line,
            },
        })
        .collect()
}

/// The marks at their adjusted positions, in the network's frame and epoch,
/// held fixed or free as they were in the adjustment.
pub fn stations(built: &Built, adjustment: &Adjustment) -> Vec<Station> {
    let fixed: &[usize] = match &built.datum {
        Datum::Constrained { fixed } => fixed,
        Datum::FixedMark(mark) => std::slice::from_ref(mark),
        Datum::Inner => &[],
    };
    built
        .network
        .marks()
        .iter()
        .zip(adjustment.marks())
        .enumerate()
        .map(|(i, (mark, adjusted))| Station {
            name: mark.name.clone(),
            description: None,
            coordinate: Some(Coordinate {
                value: CoordinateValue::Cartesian(adjusted.position),
                reference: reference(&built.network),
            }),
            constraint: if fixed.contains(&i) {
                Constraint::Fixed
            } else {
                Constraint::Free
            },
            origin: Origin {
                file: String::new(),
                line: 0,
            },
        })
        .collect()
}

/// What the result exports work from: the network as built, its adjustment,
/// and how uncertainties are scaled.
#[derive(Debug, Clone, Copy)]
pub struct Results<'a> {
    /// The network as built.
    pub built: &'a Built,
    /// Its adjustment.
    pub adjustment: &'a Adjustment,
    /// Whether uncertainties use the a priori or the a posteriori variance factor.
    pub scaling: Scaling,
}

/// A problem exporting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ExportError(pub String);

/// The square root of the 95% point of χ²(2): the 95% ellipse's axes are
/// the standard ellipse's times this.
const ELLIPSE_95: f64 = 2.447_746_830_680_816;

/// One mark's results.
struct MarkRow {
    name: String,
    xyz: Vector3<f64>,
    geodetic: Geodetic,
    mga: Option<(u8, f64, f64)>,
    uncertainty: Uncertainty,
    status: &'static str,
}

impl Results<'_> {
    fn marks(&self) -> Result<Vec<MarkRow>, ExportError> {
        let network = &self.built.network;
        let ellipsoid = network.frame().ellipsoid();
        let mut weighted = vec![false; network.marks().len()];
        for o in network.observations() {
            for e in &o.elements {
                if let gnss_adjust::Element::Position { mark, .. } = e {
                    weighted[*mark] = true;
                }
            }
        }
        let fixed: &[usize] = match &self.built.datum {
            Datum::Constrained { fixed } => fixed,
            Datum::FixedMark(mark) => std::slice::from_ref(mark),
            Datum::Inner => &[],
        };
        let mga = MgaGrid::for_frame(network.frame());
        network
            .marks()
            .iter()
            .zip(self.adjustment.marks())
            .enumerate()
            .map(|(i, (mark, adjusted))| {
                let error = |e: String| ExportError(format!("mark {}: {e}", mark.name));
                let geodetic = ellipsoid
                    .ecef_to_geodetic(&adjusted.position)
                    .map_err(|e| error(e.to_string()))?;
                let mga = match mga {
                    Some(_) => {
                        let position = self
                            .adjustment
                            .position(i, self.scaling)
                            .map_err(|e| error(e.to_string()))?;
                        let (c, _) = to_mga(&position, None).map_err(|e| error(e.to_string()))?;
                        Some((c.zone.number(), c.easting_m, c.northing_m))
                    }
                    None => None,
                };
                let uncertainty = self
                    .adjustment
                    .positional_uncertainty(i, self.scaling)
                    .map_err(|e| error(e.to_string()))?;
                Ok(MarkRow {
                    name: mark.name.clone(),
                    xyz: adjusted.position,
                    geodetic,
                    mga,
                    uncertainty,
                    status: if fixed.contains(&i) {
                        "fixed"
                    } else if weighted[i] {
                        "weighted"
                    } else {
                        "free"
                    },
                })
            })
            .collect()
    }
}

fn number(v: f64, decimals: usize) -> String {
    format!("{v:.decimals$}")
}

fn optional(v: Option<f64>, decimals: usize) -> String {
    v.map(|v| number(v, decimals)).unwrap_or_default()
}

/// The adjusted marks as CSV: ECEF, latitude, longitude and ellipsoidal
/// height, MGA where the frame has it, standard deviations on local axes,
/// SP1's 95% positional uncertainty, and how each mark was held. It reads
/// back as CSV control (fixed or free).
pub fn marks_csv(results: &Results<'_>) -> Result<String, ExportError> {
    let rows = results.marks()?;
    let reference = reference(&results.built.network);
    let epoch = reference
        .epoch
        .map(|e| e.decimal_year().to_string())
        .unwrap_or_default();
    let body: Vec<Vec<String>> = rows
        .iter()
        .map(|m| {
            let u = &m.uncertainty;
            let sd = |k: usize| number(libm::sqrt(u.enu[(k, k)].max(0.0)), 4);
            let (zone, easting, northing) = match m.mga {
                Some((z, e, n)) => (z.to_string(), number(e, 4), number(n, 4)),
                None => Default::default(),
            };
            vec![
                m.name.clone(),
                number(m.xyz.x, 4),
                number(m.xyz.y, 4),
                number(m.xyz.z, 4),
                number(m.geodetic.latitude_deg(), 10),
                number(m.geodetic.longitude_deg(), 10),
                number(m.geodetic.ellipsoidal_height_m(), 4),
                zone,
                easting,
                northing,
                sd(0),
                sd(1),
                sd(2),
                number(u.horizontal_95, 4),
                number(u.vertical_95, 4),
                if m.status == "fixed" { "fixed" } else { "free" }.to_owned(),
                m.status.to_owned(),
                reference.frame.name().to_owned(),
                epoch.clone(),
            ]
        })
        .collect();
    Ok(gnss_formats::csv::write_table(
        &[
            "name",
            "x",
            "y",
            "z",
            "latitude",
            "longitude",
            "height",
            "zone",
            "easting",
            "northing",
            "sd_e",
            "sd_n",
            "sd_u",
            "pu_h95",
            "pu_v95",
            "constraint",
            "held",
            "frame",
            "epoch",
        ],
        &body,
    ))
}

/// Each observed element's results as CSV: the residuals and w-tests on the
/// first mark's local axes, the vector test, redundancy numbers, minimal
/// detectable biases, DynAdjust's N-statistics, and whether it was used.
pub fn observations_csv(results: &Results<'_>) -> String {
    let network = &results.built.network;
    let name = |m: usize| network.marks()[m].name.clone();
    let criteria = results.adjustment.criteria();
    let mut rows = Vec::new();
    for ((o, r), source) in network
        .observations()
        .iter()
        .zip(results.adjustment.observations())
        .zip(&results.built.sources)
    {
        for (k, (e, er)) in o.elements.iter().zip(&r.elements).enumerate() {
            let (from, to) = match *e {
                gnss_adjust::Element::Difference { from, to, .. } => (name(from), name(to)),
                gnss_adjust::Element::Position { mark, .. } => (name(mark), String::new()),
            };
            let flagged = er
                .components
                .iter()
                .any(|c| c.w.is_some_and(|w| w.abs() > criteria.w))
                || er.vector_test.is_some_and(|t| t > criteria.vector);
            let mut row = vec![o.id.clone(), (k + 1).to_string(), from, to];
            row.extend(er.components.iter().map(|c| number(c.residual, 4)));
            row.extend(er.components.iter().map(|c| optional(c.w, 2)));
            row.push(optional(er.vector_test, 2));
            row.extend(er.components.iter().map(|c| number(c.redundancy, 3)));
            row.extend(er.components.iter().map(|c| optional(c.mdb, 4)));
            row.extend(er.standardised.iter().map(|n| optional(*n, 2)));
            row.push(if r.used { "yes" } else { "no" }.to_owned());
            row.push(if flagged { "yes" } else { "no" }.to_owned());
            row.push(format!("{}:{}", source.origin.file, source.origin.line));
            rows.push(row);
        }
    }
    gnss_formats::csv::write_table(
        &[
            "id",
            "element",
            "from",
            "to",
            "v_e",
            "v_n",
            "v_u",
            "w_e",
            "w_n",
            "w_u",
            "vector_test",
            "r_e",
            "r_n",
            "r_u",
            "mdb_e",
            "mdb_n",
            "mdb_u",
            "n_x",
            "n_y",
            "n_z",
            "used",
            "flagged",
            "source",
        ],
        &rows,
    )
}

/// The marks, vectors and 95% error ellipses as a GeoPackage in the
/// network's frame. `exaggeration` scales the ellipses as drawn, since
/// millimetre ellipses are invisible at map scales; their attributes give
/// the true sizes. `last_change` stamps the layers (`YYYY-MM-DDTHH:MM:SS.SSSZ`).
pub fn geopackage(
    results: &Results<'_>,
    exaggeration: f64,
    last_change: &str,
) -> Result<Vec<u8>, ExportError> {
    if !(exaggeration.is_finite() && exaggeration > 0.0) {
        return Err(ExportError(format!(
            "the ellipses' exaggeration must be positive, not {exaggeration}"
        )));
    }
    let network = &results.built.network;
    let ellipsoid = network.frame().ellipsoid();
    let marks = results.marks()?;
    let lonlat = |g: &Geodetic| [g.longitude_deg(), g.latitude_deg()];
    let real = |v: f64| FieldValue::Real(v);
    let text = |t: &str| FieldValue::Text(t.to_owned());

    // MGA columns where the frame has the grid.
    let has_mga = marks.iter().any(|m| m.mga.is_some());
    let mut mark_fields = vec![
        ("name", FieldType::Text),
        ("latitude", FieldType::Real),
        ("longitude", FieldType::Real),
        ("height", FieldType::Real),
        ("x", FieldType::Real),
        ("y", FieldType::Real),
        ("z", FieldType::Real),
    ];
    if has_mga {
        mark_fields.extend([
            ("zone", FieldType::Integer),
            ("easting", FieldType::Real),
            ("northing", FieldType::Real),
        ]);
    }
    mark_fields.extend([
        ("sd_e", FieldType::Real),
        ("sd_n", FieldType::Real),
        ("sd_u", FieldType::Real),
        ("pu_h95", FieldType::Real),
        ("pu_v95", FieldType::Real),
        ("held", FieldType::Text),
    ]);
    let point_features: Vec<Feature> = marks
        .iter()
        .map(|m| {
            let u = &m.uncertainty;
            let sd = |k: usize| real(libm::sqrt(u.enu[(k, k)].max(0.0)));
            let mut values = vec![
                text(&m.name),
                real(m.geodetic.latitude_deg()),
                real(m.geodetic.longitude_deg()),
                real(m.geodetic.ellipsoidal_height_m()),
                real(m.xyz.x),
                real(m.xyz.y),
                real(m.xyz.z),
            ];
            if has_mga {
                values.extend(match m.mga {
                    Some((z, e, n)) => [FieldValue::Integer(i64::from(z)), real(e), real(n)],
                    None => [FieldValue::Null, FieldValue::Null, FieldValue::Null],
                });
            }
            values.extend([
                sd(0),
                sd(1),
                sd(2),
                real(u.horizontal_95),
                real(u.vertical_95),
                text(m.status),
            ]);
            Feature {
                geometry: Geometry::Point(lonlat(&m.geodetic)),
                values,
            }
        })
        .collect();

    let criteria = results.adjustment.criteria();
    let mut vector_features = Vec::new();
    for ((o, r), source) in network
        .observations()
        .iter()
        .zip(results.adjustment.observations())
        .zip(&results.built.sources)
    {
        for (k, (e, er)) in o.elements.iter().zip(&r.elements).enumerate() {
            let gnss_adjust::Element::Difference {
                from, to, vector, ..
            } = *e
            else {
                continue;
            };
            let flagged = er
                .components
                .iter()
                .any(|c| c.w.is_some_and(|w| w.abs() > criteria.w))
                || er.vector_test.is_some_and(|t| t > criteria.vector);
            let w = |c: usize| er.components[c].w.map_or(FieldValue::Null, real);
            vector_features.push(Feature {
                geometry: Geometry::LineString(vec![
                    lonlat(&marks[from].geodetic),
                    lonlat(&marks[to].geodetic),
                ]),
                values: vec![
                    text(&if o.elements.len() == 1 {
                        o.id.clone()
                    } else {
                        format!("{}/{}", o.id, k + 1)
                    }),
                    text(&marks[from].name),
                    text(&marks[to].name),
                    real(vector.norm()),
                    real(er.components[0].residual),
                    real(er.components[1].residual),
                    real(er.components[2].residual),
                    w(0),
                    w(1),
                    w(2),
                    er.vector_test.map_or(FieldValue::Null, real),
                    FieldValue::Integer(i64::from(flagged)),
                    FieldValue::Integer(i64::from(r.used)),
                    text(&format!("{}:{}", source.origin.file, source.origin.line)),
                ],
            });
        }
    }

    let mut ellipse_features = Vec::new();
    for m in &marks {
        let e = &m.uncertainty.ellipse;
        let (a, b) = (e.semi_major * ELLIPSE_95, e.semi_minor * ELLIPSE_95);
        let (sin_az, cos_az) = (libm::sin(e.azimuth), libm::cos(e.azimuth));
        let r = m.geodetic.enu_rotation();
        let mut ring = Vec::with_capacity(73);
        for k in 0..72 {
            let t = std::f64::consts::TAU * f64::from(k) / 72.0;
            let (s, c) = (libm::sin(t), libm::cos(t));
            // Along the major axis (azimuth clockwise from north), then the minor.
            let east = exaggeration * (a * c * sin_az + b * s * cos_az);
            let north = exaggeration * (a * c * cos_az - b * s * sin_az);
            let xyz = m.xyz + r.transpose() * Vector3::new(east, north, 0.0);
            let g = ellipsoid
                .ecef_to_geodetic(&xyz)
                .map_err(|e| ExportError(format!("mark {}: {e}", m.name)))?;
            ring.push(lonlat(&g));
        }
        ring.push(ring[0]);
        ellipse_features.push(Feature {
            geometry: Geometry::Polygon(vec![ring]),
            values: vec![
                text(&m.name),
                real(a),
                real(b),
                real(e.azimuth.to_degrees()),
                real(exaggeration),
            ],
        });
    }

    let layer =
        |name: &str, description: &str, geometry, fields: Vec<(&str, FieldType)>, features| Layer {
            name: name.into(),
            description: description.into(),
            geometry,
            fields: fields.into_iter().map(|(n, t)| (n.to_owned(), t)).collect(),
            features,
        };
    let layers = vec![
        layer(
            "marks",
            "Adjusted marks, with standard deviations and 95% positional uncertainty",
            GeometryType::Point,
            mark_fields,
            point_features,
        ),
        layer(
            "vectors",
            "GNSS vectors, with residuals and test statistics on the first mark's local axes",
            GeometryType::LineString,
            vec![
                ("id", FieldType::Text),
                ("from_mark", FieldType::Text),
                ("to_mark", FieldType::Text),
                ("length", FieldType::Real),
                ("v_e", FieldType::Real),
                ("v_n", FieldType::Real),
                ("v_u", FieldType::Real),
                ("w_e", FieldType::Real),
                ("w_n", FieldType::Real),
                ("w_u", FieldType::Real),
                ("vector_test", FieldType::Real),
                ("flagged", FieldType::Integer),
                ("used", FieldType::Integer),
                ("source", FieldType::Text),
            ],
            vector_features,
        ),
        layer(
            "ellipses",
            &format!("95% error ellipses, drawn {exaggeration} times their true size"),
            GeometryType::Polygon,
            vec![
                ("name", FieldType::Text),
                ("semi_major_95", FieldType::Real),
                ("semi_minor_95", FieldType::Real),
                ("azimuth", FieldType::Real),
                ("exaggeration", FieldType::Real),
            ],
            ellipse_features,
        ),
    ];
    gnss_formats::gpkg::write(&layers, network.frame(), last_change).map_err(|e| ExportError(e.0))
}

/// The adjusted marks as LandXML control points on MGA, in the zone that
/// holds the most of them, with ellipsoidal heights. The network must be in
/// GDA2020 or GDA94. `date` (YYYY-MM-DD) and `time` (HH:MM:SS) stamp the file.
pub fn landxml(results: &Results<'_>, date: &str, time: &str) -> Result<String, ExportError> {
    let network = &results.built.network;
    let grid = MgaGrid::for_frame(network.frame()).ok_or_else(|| {
        ExportError(format!(
            "LandXML control points are written on MGA, which is defined on GDA2020 and GDA94, \
             but the network is in {}",
            network.frame()
        ))
    })?;
    let marks = results.marks()?;
    // The zone holding the most marks, the lowest on a tie.
    let mut counts = BTreeMap::new();
    for m in &marks {
        if let Some((zone, ..)) = m.mga {
            *counts.entry(zone).or_insert(0usize) += 1;
        }
    }
    let zone_number = counts
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
        .map(|(z, _)| *z)
        .ok_or_else(|| ExportError("there are no marks to write".into()))?;
    let zone = MgaZone::new(zone_number).map_err(|e| ExportError(e.to_string()))?;
    let mut points = Vec::with_capacity(marks.len());
    for (i, m) in marks.iter().enumerate() {
        let position = results
            .adjustment
            .position(i, results.scaling)
            .map_err(|e| ExportError(e.to_string()))?;
        let (c, _) = to_mga(&position, Some(zone)).map_err(|e| ExportError(e.to_string()))?;
        points.push(ControlPoint {
            name: m.name.clone(),
            northing: c.northing_m,
            easting: c.easting_m,
            elevation: Some(m.geodetic.ellipsoidal_height_m()),
            description: Some(format!(
                "PU 95%: horizontal {:.4} m, vertical {:.4} m; {}",
                m.uncertainty.horizontal_95, m.uncertainty.vertical_95, m.status
            )),
        });
    }
    let (datum, code) = match grid {
        MgaGrid::Mga2020 => ("GDA2020", 7800 + u32::from(zone_number)),
        MgaGrid::Mga94 => ("GDA94", 28300 + u32::from(zone_number)),
    };
    let system = CoordinateSystem {
        name: format!("{datum} / MGA zone {zone_number}"),
        epsg_code: code,
        horizontal_datum: datum.into(),
        vertical_datum: "GRS80 ellipsoid (ellipsoidal heights)".into(),
    };
    Ok(gnss_formats::landxml::write(
        &points,
        &system,
        date,
        time,
        "GNSS",
        crate::VERSION,
    ))
}

/// The adjusted marks and the observations as adjusted, as DynaML station
/// and measurement files in the network's frame: for cross-checks in
/// DynAdjust and for agency submission. Marks are written in ECEF, so
/// nothing is lost to rounding.
pub fn dynaml(results: &Results<'_>) -> Result<(String, String), ExportError> {
    let reference = reference(&results.built.network);
    let stations = gnss_formats::dynaml::write_stations(
        &stations(results.built, results.adjustment),
        reference,
    )
    .map_err(|e| ExportError(e.0))?;
    let measurements =
        gnss_formats::dynaml::write_measurements(&measurements(results.built), reference);
    Ok((stations, measurements))
}

/// The vectors as adjusted, as a STAR*NET GPS file in the network's frame.
/// Correlations between a cluster's baselines are dropped: STAR*NET has no
/// way to hold them.
pub fn starnet(results: &Results<'_>) -> Result<String, ExportError> {
    gnss_formats::starnet::write(&measurements(results.built)).map_err(|e| ExportError(e.0))
}
