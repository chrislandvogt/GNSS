//! Generic CSV files for vectors and control, for packages with no other
//! format in common. Columns are found by name, in any order, ignoring case,
//! spaces, underscores and units in brackets (`dX (m)` is `dx`). Lines
//! starting with `#` are comments. Commas separate fields, or semicolons or
//! tabs if the heading uses them instead. The formats are documented, with
//! examples, in `docs/user-guide/formats.md`.
//!
//! **Vectors**, one per row:
//!
//! | Column | Meaning |
//! |---|---|
//! | `from`, `to` | The marks (also `base` and `rover`) |
//! | `dx`, `dy`, `dz` | ECEF components, metres |
//! | `cov_xx` … `cov_zz` | The covariance, m² (all six), or: |
//! | `sd_x`, `sd_y`, `sd_z` | standard deviations, m, with optional `corr_xy`, `corr_xz`, `corr_yz` |
//! | `frame`, `epoch` | Optional: the frame, and the epoch as a decimal year or date |
//! | `id`, `ignore` | Optional: a name for the vector, and `yes` to leave it out |
//!
//! **Control**, one mark per row:
//!
//! | Column | Meaning |
//! |---|---|
//! | `name` | The mark |
//! | `x`, `y`, `z` | ECEF, metres, or: |
//! | `latitude`, `longitude`, `height` | degrees (decimal, or `d m s`) and ellipsoidal height; `orthometric_height` (or `elevation`) instead for heights above the geoid, or: |
//! | `easting`, `northing`, `zone`, `height` | MGA/UTM south |
//! | `sd_e`, `sd_n`, `sd_u` | Standard deviations in local axes, m (or `sd_h` and `sd_v` for horizontal and vertical), or `sd_x`… with `corr_xy`…, or `cov_xx`… in ECEF |
//! | `constraint` | `fixed`, `free` (a starting position only) or `weighted`. Without one, marks with standard deviations are weighted and the rest fixed |
//! | `frame`, `epoch`, `description` | Optional |

use std::collections::BTreeMap;

use gnss_datum::tm::MgaZone;
use gnss_datum::{FrameId, Geodetic};
use nalgebra::{DMatrix, Matrix3, Vector3};

use crate::error::FormatError;
use crate::survey::{
    Constraint, Coordinate, CoordinateValue, Element, HeightType, Measurement, MeasurementKind,
    Origin, Reference, Scaling, Station, Survey,
};
use crate::text::{epoch_text, number};

/// A column heading reduced to letters and digits, without units in brackets.
fn key(heading: &str) -> String {
    let without_units = match heading.find(['(', '[']) {
        Some(i) => &heading[..i],
        None => heading,
    };
    without_units
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// A CSV file's rows, with the column each name is in.
struct Table {
    columns: BTreeMap<String, usize>,
    rows: Vec<(usize, Vec<String>)>,
}

impl Table {
    fn read(file: &str, text: &str) -> Result<Table, FormatError> {
        let heading = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .ok_or_else(|| FormatError::in_file(file, "it's empty"))?;
        let delimiter = if heading.contains(',') {
            b','
        } else if heading.contains(';') {
            b';'
        } else if heading.contains('\t') {
            b'\t'
        } else {
            b','
        };
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .trim(csv::Trim::All)
            .comment(Some(b'#'))
            .delimiter(delimiter)
            .from_reader(text.as_bytes());
        let error = |e: csv::Error| {
            let line = e.position().map_or(0, |p| p.line() as usize);
            FormatError::at(file, line.max(1), format!("it isn't valid CSV: {e}"))
        };
        let headers = reader.headers().map_err(error)?.clone();
        let mut columns = BTreeMap::new();
        for (i, h) in headers.iter().enumerate() {
            let k = key(h);
            if !k.is_empty() && columns.insert(k.clone(), i).is_some() {
                return Err(FormatError::at(
                    file,
                    1,
                    format!("the column {h:?} appears twice"),
                ));
            }
        }
        let mut rows = Vec::new();
        for record in reader.records() {
            let record = record.map_err(error)?;
            if record.iter().all(str::is_empty) {
                continue;
            }
            let line = record.position().map_or(0, |p| p.line() as usize);
            rows.push((line, record.iter().map(str::to_owned).collect()));
        }
        Ok(Table { columns, rows })
    }

    /// The column with the first of these names that the file has.
    fn column(&self, names: &[&str]) -> Option<usize> {
        names.iter().find_map(|n| self.columns.get(*n).copied())
    }
}

/// One row's fields, by column.
struct Row<'a> {
    file: &'a str,
    line: usize,
    fields: &'a [String],
}

impl Row<'_> {
    fn error(&self, message: String) -> FormatError {
        FormatError::at(self.file, self.line, message)
    }

    fn text(&self, column: Option<usize>) -> Option<&str> {
        column
            .and_then(|c| self.fields.get(c))
            .map(String::as_str)
            .filter(|t| !t.is_empty())
    }

    fn required(&self, column: Option<usize>, what: &str) -> Result<&str, FormatError> {
        self.text(column)
            .ok_or_else(|| self.error(format!("{what} is empty")))
    }

    fn number(&self, column: Option<usize>, what: &str) -> Result<f64, FormatError> {
        number(what, self.required(column, what)?).map_err(|m| self.error(m))
    }

    fn optional(&self, column: Option<usize>, what: &str) -> Result<Option<f64>, FormatError> {
        self.text(column)
            .map(|t| number(what, t).map_err(|m| self.error(m)))
            .transpose()
    }

    /// The row's frame and epoch, or the file's default.
    fn reference(
        &self,
        frame: Option<usize>,
        epoch: Option<usize>,
        default: Option<Reference>,
    ) -> Result<Reference, FormatError> {
        let frame_id = match self.text(frame) {
            Some(name) => FrameId::parse(name).map_err(|e| self.error(e.to_string()))?,
            None => default.map(|d| d.frame).ok_or_else(|| {
                self.error(
                    "no frame: give a frame column, or say which frame the file is in".into(),
                )
            })?,
        };
        let epoch = match self.text(epoch) {
            Some(text) => Some(epoch_text("epoch", text).map_err(|m| self.error(m))?),
            None => default
                .filter(|d| d.frame == frame_id)
                .and_then(|d| d.epoch),
        };
        match (frame_id.reference_epoch(), epoch) {
            (Some(reference), Some(given)) if given != reference => Err(self.error(format!(
                "{frame_id} is a static frame, so its epoch can only be {reference}, not {given}"
            ))),
            (Some(_), _) => Ok(Reference {
                frame: frame_id,
                epoch: None,
            }),
            (None, Some(e)) => Ok(Reference {
                frame: frame_id,
                epoch: Some(e),
            }),
            (None, None) => Err(self.error(format!(
                "{frame_id} is a dynamic frame, so it needs an epoch: give an epoch column, or \
                 say which epoch the file is at"
            ))),
        }
    }
}

/// Covariance columns: a full ECEF covariance, or standard deviations with
/// optional correlations.
struct CovarianceColumns {
    full: Option<[usize; 6]>,
    sigmas: Option<[usize; 3]>,
    correlations: [Option<usize>; 3],
}

impl CovarianceColumns {
    fn find(table: &Table) -> Self {
        let full =
            ["covxx", "covxy", "covxz", "covyy", "covyz", "covzz"].map(|n| table.column(&[n]));
        let sigmas = [
            table.column(&["sdx", "sigmax", "stdx"]),
            table.column(&["sdy", "sigmay", "stdy"]),
            table.column(&["sdz", "sigmaz", "stdz"]),
        ];
        Self {
            full: full
                .iter()
                .all(Option::is_some)
                .then(|| full.map(|c| c.unwrap_or(0))),
            sigmas: sigmas
                .iter()
                .all(Option::is_some)
                .then(|| sigmas.map(|c| c.unwrap_or(0))),
            correlations: [
                table.column(&["corrxy", "rxy"]),
                table.column(&["corrxz", "rxz"]),
                table.column(&["corryz", "ryz"]),
            ],
        }
    }

    fn present(&self) -> bool {
        self.full.is_some() || self.sigmas.is_some()
    }

    /// The row's ECEF covariance, if it gives one. A row may leave all the
    /// covariance fields empty, as fixed and free control does.
    fn read(&self, row: &Row<'_>) -> Result<Option<Matrix3<f64>>, FormatError> {
        let columns: Vec<usize> = match (self.full, self.sigmas) {
            (Some(c), _) => c.to_vec(),
            (None, Some(c)) => c.to_vec(),
            (None, None) => Vec::new(),
        };
        if columns.iter().all(|&c| row.text(Some(c)).is_none()) {
            return Ok(None);
        }
        if let Some(c) = self.full {
            let names = ["cov_xx", "cov_xy", "cov_xz", "cov_yy", "cov_yz", "cov_zz"];
            let mut v = [0.0; 6];
            for i in 0..6 {
                v[i] = row.number(Some(c[i]), names[i])?;
            }
            let [xx, xy, xz, yy, yz, zz] = v;
            return Ok(Some(Matrix3::new(xx, xy, xz, xy, yy, yz, xz, yz, zz)));
        }
        let Some(c) = self.sigmas else {
            return Ok(None);
        };
        let [sx, sy, sz] = [
            row.number(Some(c[0]), "sd_x")?,
            row.number(Some(c[1]), "sd_y")?,
            row.number(Some(c[2]), "sd_z")?,
        ];
        let mut r = [0.0; 3];
        for (i, name) in ["corr_xy", "corr_xz", "corr_yz"].iter().enumerate() {
            r[i] = row.optional(self.correlations[i], name)?.unwrap_or(0.0);
            if r[i].abs() > 1.0 {
                return Err(row.error(format!("{name} is {}, beyond ±1", r[i])));
            }
        }
        let [rxy, rxz, ryz] = r;
        Ok(Some(Matrix3::new(
            sx * sx,
            rxy * sx * sy,
            rxz * sx * sz,
            rxy * sx * sy,
            sy * sy,
            ryz * sy * sz,
            rxz * sx * sz,
            ryz * sy * sz,
            sz * sz,
        )))
    }
}

fn positive(row: &Row<'_>, c: &Matrix3<f64>) -> Result<(), FormatError> {
    if c.diagonal().iter().any(|&v| v <= 0.0) {
        return Err(row.error("the variances must be positive".into()));
    }
    Ok(())
}

/// Reads a CSV file of vectors. `default` gives the frame and epoch for rows
/// without them.
pub fn read_vectors(
    file: &str,
    text: &str,
    default: Option<Reference>,
) -> Result<Survey, FormatError> {
    let table = Table::read(file, text)?;
    let missing = |what: &str| FormatError::at(file, 1, format!("there's no {what} column"));
    let from = table.column(&["from", "base", "fromstation", "station1"]);
    let to = table.column(&["to", "rover", "tostation", "station2"]);
    let d = [
        table.column(&["dx", "deltax"]),
        table.column(&["dy", "deltay"]),
        table.column(&["dz", "deltaz"]),
    ];
    if from.is_none() {
        return Err(missing("from"));
    }
    if to.is_none() {
        return Err(missing("to"));
    }
    if d.iter().any(Option::is_none) {
        return Err(missing("dx, dy and dz"));
    }
    let covariance = CovarianceColumns::find(&table);
    if !covariance.present() {
        return Err(FormatError::at(
            file,
            1,
            "there are no covariance columns: give cov_xx, cov_xy, cov_xz, cov_yy, cov_yz and \
             cov_zz, or sd_x, sd_y and sd_z",
        ));
    }
    let (frame, epoch) = (table.column(&["frame"]), table.column(&["epoch"]));
    let (id, ignore) = (table.column(&["id", "name"]), table.column(&["ignore"]));
    let mut survey = Survey::default();
    for (line, fields) in &table.rows {
        let row = Row {
            file,
            line: *line,
            fields,
        };
        let from_name = row.required(from, "from")?.to_owned();
        let to_name = row.required(to, "to")?.to_owned();
        if from_name == to_name {
            return Err(row.error(format!(
                "the vector from {from_name} ends at {from_name} too"
            )));
        }
        let vector = Vector3::new(
            row.number(d[0], "dx")?,
            row.number(d[1], "dy")?,
            row.number(d[2], "dz")?,
        );
        let Some(c) = covariance.read(&row)? else {
            return Err(row.error("the covariance is empty".into()));
        };
        positive(&row, &c)?;
        let ignored = match row.text(ignore).map(str::to_ascii_lowercase).as_deref() {
            None | Some("no" | "n" | "false" | "0") => false,
            Some("yes" | "y" | "true" | "1" | "*") => true,
            Some(other) => {
                return Err(row.error(format!("ignore should be yes or no, not {other:?}")));
            }
        };
        survey.measurements.push(Measurement {
            kind: MeasurementKind::Baseline,
            elements: vec![Element::Vector {
                from: from_name,
                to: to_name,
                vector,
            }],
            covariance: DMatrix::from_column_slice(3, 3, c.as_slice()),
            reference: row.reference(frame, epoch, default)?,
            scaling: Scaling::NONE,
            ignored,
            source: None,
            id: row.text(id).map(str::to_owned),
            origin: Origin {
                file: file.to_owned(),
                line: *line,
            },
        });
    }
    if survey.measurements.is_empty() {
        return Err(FormatError::in_file(file, "it has no vectors"));
    }
    Ok(survey)
}

/// An angle in decimal degrees, or degrees, minutes and seconds separated by
/// spaces, the sign on the degrees.
fn degrees(row: &Row<'_>, column: Option<usize>, what: &str) -> Result<f64, FormatError> {
    let text = row.required(column, what)?;
    let parts: Vec<&str> = text.split_whitespace().collect();
    match parts[..] {
        [decimal] => number(what, decimal).map_err(|m| row.error(m)),
        [d, m, s] => {
            let negative = d.starts_with('-');
            let d = number(what, d).map_err(|e| row.error(e))?.abs();
            let m = number(what, m).map_err(|e| row.error(e))?;
            let s = number(what, s).map_err(|e| row.error(e))?;
            if !(0.0..60.0).contains(&m) || !(0.0..60.0).contains(&s) {
                return Err(row.error(format!(
                    "{what} {text:?} has minutes or seconds out of range"
                )));
            }
            let value = d + m / 60.0 + s / 3600.0;
            Ok(if negative { -value } else { value })
        }
        _ => Err(row.error(format!(
            "{what} should be decimal degrees or \"d m s\", not {text:?}"
        ))),
    }
}

/// Reads a CSV file of control marks. `default` gives the frame and epoch for
/// rows without them.
pub fn read_control(
    file: &str,
    text: &str,
    default: Option<Reference>,
) -> Result<Survey, FormatError> {
    let table = Table::read(file, text)?;
    let name = table
        .column(&["name", "station", "mark", "point", "code"])
        .ok_or_else(|| FormatError::at(file, 1, "there's no name column"))?;
    let xyz = [
        table.column(&["x"]),
        table.column(&["y"]),
        table.column(&["z"]),
    ];
    let latitude = table.column(&["latitude", "lat"]);
    let longitude = table.column(&["longitude", "lon", "long"]);
    let easting = table.column(&["easting", "east"]);
    let northing = table.column(&["northing", "north"]);
    let zone = table.column(&["zone"]);
    let ellipsoidal = table.column(&["height", "h", "ellipsoidalheight"]);
    let orthometric = table.column(&["orthometricheight", "elevation", "rl"]);
    let form = if xyz.iter().all(Option::is_some) {
        "xyz"
    } else if latitude.is_some() && longitude.is_some() {
        "geographic"
    } else if easting.is_some() && northing.is_some() && zone.is_some() {
        "grid"
    } else {
        return Err(FormatError::at(
            file,
            1,
            "there are no coordinate columns: give x, y and z, or latitude, longitude and \
             height, or easting, northing, zone and height",
        ));
    };
    if form != "xyz" && ellipsoidal.is_none() && orthometric.is_none() {
        return Err(FormatError::at(
            file,
            1,
            "there's no height column: give height (ellipsoidal) or orthometric_height",
        ));
    }
    if form != "xyz" && ellipsoidal.is_some() && orthometric.is_some() {
        return Err(FormatError::at(
            file,
            1,
            "give one height column, height (ellipsoidal) or orthometric_height, not both",
        ));
    }
    let ecef = CovarianceColumns::find(&table);
    let local = [
        table.column(&["sde", "sdeast"]),
        table.column(&["sdn", "sdnorth"]),
        table.column(&["sdu", "sdup"]),
    ];
    let horizontal = table.column(&["sdh", "sdhorizontal"]);
    let vertical = table.column(&["sdv", "sdvertical"]);
    let has_local =
        local.iter().all(Option::is_some) || (horizontal.is_some() && vertical.is_some());
    let constraint = table.column(&["constraint"]);
    let (frame, epoch) = (table.column(&["frame"]), table.column(&["epoch"]));
    let description = table.column(&["description", "desc", "comment"]);

    let mut survey = Survey::default();
    for (line, fields) in &table.rows {
        let row = Row {
            file,
            line: *line,
            fields,
        };
        let mark = row.required(Some(name), "name")?.to_owned();
        let reference = row.reference(frame, epoch, default)?;
        let (height_column, height) = match ellipsoidal {
            Some(c) => (Some(c), HeightType::Ellipsoidal),
            None => (orthometric, HeightType::Orthometric),
        };
        let value = match form {
            "xyz" => CoordinateValue::Cartesian(Vector3::new(
                row.number(xyz[0], "x")?,
                row.number(xyz[1], "y")?,
                row.number(xyz[2], "z")?,
            )),
            "geographic" => {
                let lat = degrees(&row, latitude, "latitude")?;
                if lat.abs() > 90.0 {
                    return Err(row.error(format!("latitude {lat} is beyond the poles")));
                }
                CoordinateValue::Geographic {
                    latitude_rad: lat.to_radians(),
                    longitude_rad: degrees(&row, longitude, "longitude")?.to_radians(),
                    height_m: row.number(height_column, "height")?,
                    height,
                }
            }
            _ => {
                let z = row.required(zone, "zone")?;
                let z = z
                    .trim_start_matches(['S', 's'])
                    .parse::<u8>()
                    .ok()
                    .filter(|z| (1..=60).contains(z))
                    .ok_or_else(|| {
                        row.error(format!("zone {z:?} isn't a UTM zone from 1 to 60"))
                    })?;
                CoordinateValue::Grid {
                    easting_m: row.number(easting, "easting")?,
                    northing_m: row.number(northing, "northing")?,
                    zone: z,
                    height_m: row.number(height_column, "height")?,
                    height,
                }
            }
        };
        // The covariance, in ECEF, if the row gives one.
        let covariance =
            if has_local && (row.text(local[0]).is_some() || row.text(horizontal).is_some()) {
                let (e, n, u) = if local.iter().all(Option::is_some) {
                    (
                        row.number(local[0], "sd_e")?,
                        row.number(local[1], "sd_n")?,
                        row.number(local[2], "sd_u")?,
                    )
                } else {
                    let h = row.number(horizontal, "sd_h")?;
                    (h, h, row.number(vertical, "sd_v")?)
                };
                let geodetic = geodetic_of(&value, reference).map_err(|m| row.error(m))?;
                let r = geodetic.enu_rotation();
                let enu = Matrix3::from_diagonal(&Vector3::new(e * e, n * n, u * u));
                Some(r.transpose() * enu * r)
            } else {
                ecef.read(&row)?
            };
        if let Some(c) = &covariance {
            positive(&row, c)?;
        }
        let kind = row.text(constraint).map(str::to_ascii_lowercase);
        let weighted = match (kind.as_deref(), covariance.is_some()) {
            (Some("fixed" | "held" | "ccc"), _) => false,
            (Some("free" | "fff"), _) => false,
            (Some("weighted"), true) | (None, true) => true,
            (Some("weighted"), false) => {
                return Err(row.error(format!(
                    "mark {mark} is weighted control, but has no standard deviations"
                )));
            }
            (None, false) => false,
            (Some(other), _) => {
                return Err(row.error(format!(
                    "constraint should be fixed, free or weighted, not {other:?}"
                )));
            }
        };
        let station_constraint = match kind.as_deref() {
            Some("free" | "fff") => Constraint::Free,
            Some("fixed" | "held" | "ccc") => Constraint::Fixed,
            _ if weighted => Constraint::Free,
            _ => Constraint::Fixed,
        };
        let origin = Origin {
            file: file.to_owned(),
            line: *line,
        };
        if let (true, Some(c)) = (weighted, covariance) {
            let position = position_of(&value, reference).map_err(|m| row.error(m))?;
            survey.measurements.push(Measurement {
                kind: MeasurementKind::PointCluster,
                elements: vec![Element::Position {
                    station: mark.clone(),
                    position,
                }],
                covariance: DMatrix::from_column_slice(3, 3, c.as_slice()),
                reference,
                scaling: Scaling::NONE,
                ignored: false,
                source: None,
                id: Some(mark.clone()),
                origin: origin.clone(),
            });
        }
        survey.stations.push(Station {
            name: mark,
            description: row.text(description).map(str::to_owned),
            coordinate: Some(Coordinate { value, reference }),
            constraint: station_constraint,
            origin,
        });
    }
    if survey.stations.is_empty() {
        return Err(FormatError::in_file(file, "it has no marks"));
    }
    Ok(survey)
}

/// A coordinate's geodetic position on its frame's ellipsoid, for local axes.
/// Orthometric heights are used as they are, which changes nothing in the axes.
fn geodetic_of(value: &CoordinateValue, reference: Reference) -> Result<Geodetic, String> {
    match *value {
        CoordinateValue::Cartesian(xyz) => reference
            .frame
            .ellipsoid()
            .ecef_to_geodetic(&xyz)
            .map_err(|e| e.to_string()),
        CoordinateValue::Geographic {
            latitude_rad,
            longitude_rad,
            height_m,
            ..
        } => Geodetic::new(latitude_rad, longitude_rad, height_m).map_err(|e| e.to_string()),
        CoordinateValue::Grid {
            easting_m,
            northing_m,
            zone,
            height_m,
            ..
        } => MgaZone::new(zone)
            .and_then(|z| z.projection().inverse(easting_m, northing_m))
            .and_then(|(g, _)| Geodetic::new(g.latitude_rad(), g.longitude_rad(), height_m))
            .map_err(|e| e.to_string()),
    }
}

/// A coordinate's ECEF position, for weighted control: its height must be
/// ellipsoidal.
fn position_of(value: &CoordinateValue, reference: Reference) -> Result<Vector3<f64>, String> {
    let orthometric = matches!(
        value,
        CoordinateValue::Geographic {
            height: HeightType::Orthometric,
            ..
        } | CoordinateValue::Grid {
            height: HeightType::Orthometric,
            ..
        }
    );
    if orthometric {
        return Err(
            "weighted control needs an ellipsoidal height, but this one is orthometric".into(),
        );
    }
    if let CoordinateValue::Cartesian(xyz) = value {
        return Ok(*xyz);
    }
    let geodetic = geodetic_of(value, reference)?;
    Ok(reference.frame.ellipsoid().geodetic_to_ecef(&geodetic))
}

/// A CSV field, quoted if it holds a separator, a quote or a line break.
fn field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) || text != text.trim() {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// A reference as `frame,epoch` fields, the epoch empty for a static frame.
fn reference_fields(reference: Reference) -> String {
    let epoch = reference
        .epoch
        .filter(|_| !reference.frame.is_static())
        .map(|e| e.decimal_year().to_string())
        .unwrap_or_default();
    format!("{},{epoch}", field(reference.frame.name()))
}

/// Writes GNSS baselines in the CSV vector format, with full covariances.
/// Numbers are written so they read back exactly. Covariances are written as
/// given, without variance scaling; correlations between a cluster's
/// baselines are dropped, since each row is one vector.
pub fn write_vectors(measurements: &[Measurement]) -> String {
    let mut out = String::from(
        "id,from,to,dx,dy,dz,cov_xx,cov_xy,cov_xz,cov_yy,cov_yz,cov_zz,frame,epoch,ignore\n",
    );
    for m in measurements {
        for (i, element) in m.elements.iter().enumerate() {
            let Element::Vector { from, to, vector } = element else {
                continue;
            };
            let c = |r: usize, s: usize| m.covariance[(3 * i + r, 3 * i + s)];
            let id = match (&m.id, m.elements.len()) {
                (Some(id), 1) => id.clone(),
                (Some(id), _) => format!("{id}/{}", i + 1),
                (None, _) => String::new(),
            };
            out.push_str(&format!(
                "{},{},{},{},{},{},{:e},{:e},{:e},{:e},{:e},{:e},{},{}\n",
                field(&id),
                field(from),
                field(to),
                vector.x,
                vector.y,
                vector.z,
                c(0, 0),
                c(0, 1),
                c(0, 2),
                c(1, 1),
                c(1, 2),
                c(2, 2),
                reference_fields(m.reference),
                if m.ignored { "yes" } else { "no" }
            ));
        }
    }
    out
}

/// Writes control marks in the CSV control format, in ECEF. A mark with an
/// observed position in `positions` (a one-point measurement) is written as
/// weighted control, at that position with its covariance; the rest with
/// their station coordinates, which must be Cartesian, as fixed or free.
pub fn write_control(
    stations: &[Station],
    positions: &[Measurement],
) -> Result<String, crate::error::WriteError> {
    let mut out = String::from(
        "name,x,y,z,cov_xx,cov_xy,cov_xz,cov_yy,cov_yz,cov_zz,constraint,frame,epoch,description\n",
    );
    for s in stations {
        let observed = positions.iter().find_map(|m| match &m.elements[..] {
            [Element::Position { station, position }] if station == &s.name => Some((m, position)),
            _ => None,
        });
        let (xyz, covariance, constraint, reference) = match (observed, &s.coordinate) {
            (Some((m, position)), _) => (
                *position,
                Some(m.covariance.fixed_view::<3, 3>(0, 0).into_owned()),
                "weighted",
                m.reference,
            ),
            (
                None,
                Some(Coordinate {
                    value: CoordinateValue::Cartesian(xyz),
                    reference,
                }),
            ) => (
                *xyz,
                None,
                match s.constraint {
                    Constraint::Fixed => "fixed",
                    _ => "free",
                },
                *reference,
            ),
            (None, _) => {
                return Err(crate::error::WriteError(format!(
                    "station {} has no Cartesian coordinate to write",
                    s.name
                )));
            }
        };
        let covariance = covariance
            .map(|c| {
                format!(
                    "{:e},{:e},{:e},{:e},{:e},{:e}",
                    c[(0, 0)],
                    c[(0, 1)],
                    c[(0, 2)],
                    c[(1, 1)],
                    c[(1, 2)],
                    c[(2, 2)]
                )
            })
            .unwrap_or_else(|| ",,,,,".to_owned());
        out.push_str(&format!(
            "{},{},{},{},{covariance},{constraint},{},{}\n",
            field(&s.name),
            xyz.x,
            xyz.y,
            xyz.z,
            reference_fields(reference),
            field(s.description.as_deref().unwrap_or(""))
        ));
    }
    Ok(out)
}
