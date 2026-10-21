//! Trimble Data Exchange Format (TDEF) files, as Trimble Business Center
//! writes them with its TDEF exporter: processed GNSS vectors with their
//! full covariances, the stations they join, and keyed-in coordinates.
//! CHC Geomatics Office's "TGO baseline result" export is reported to use
//! the same format.
//!
//! There's no public specification, so this follows real TBC exports and
//! ICSM's PynAdjust converter. A file is sections of `key=value` records,
//! with fields split by the `[General]` section's `Separator` (`:`) and
//! `MissingValue` (`?`) for empty fields:
//!
//! ```text
//! [General]
//! Source=Trimble Business Center
//! ProjCoordinateSystem=Australia/GDA2020
//! GPSVectors=MarkToMark
//! Separator=:
//!
//! [Stations]
//! Station=2:?:BEEC:36.346446515S:146.657755886E:445.9390:5977570.0907:469289.7754:434.6931:...
//!
//! [Keyed In Coordinates]
//! LLCoord=1:?:BEEC:36.346434060S:146.657743009E:442.9339:?:U:U:U:E:W
//!
//! [GPS]
//! Vector=1:?:324900360:BEEC:-8628.7131:12647.1564:18788.9403:1.80e-04:-1.09e-04:1.51e-04:8.61e-05:-1.02e-04:1.54e-04:1.7366:0.1074:PostProcessed:?:0.019:?:16 12 2019:21 48 47.0:17 12 2019:04 12 47.0:...
//! ```
//!
//! - A `Vector` gives the marks, ΔX, ΔY and ΔZ in metres, and the
//!   covariance in m² as XX, XY, XZ, YY, YZ, ZZ, then the antenna heights
//!   (already applied: the file says `GPSVectors=MarkToMark`) and the
//!   observation's start and end. A covariance that isn't positive definite
//!   is refused, which also guards the order of its terms.
//! - Vectors that aren't mark to mark, and units other than metres, are
//!   refused rather than guessed at.
//! - Stations and keyed-in coordinates are latitude, longitude and
//!   ellipsoidal height in the project's coordinate system. They become
//!   free marks, starting positions only, with keyed-in coordinates
//!   preferred: holding control is the person's choice, made by importing
//!   it.
//! - **The file doesn't say which frame the vectors are in**, so the caller
//!   does. For a dynamic frame, each vector's epoch is the middle of its
//!   observation unless the caller gives one.

use std::collections::BTreeMap;

use gnss_datum::hifitime::{Epoch, TimeScale};
use gnss_datum::{CoordinateEpoch, FrameId};
use nalgebra::{DMatrix, Matrix3, Vector3};

use crate::error::FormatError;
use crate::survey::{
    Constraint, Coordinate, CoordinateValue, Element, HeightType, Measurement, MeasurementKind,
    Note, Origin, Reference, Scaling, Station, Survey,
};
use crate::text::number;

/// A latitude or longitude written as decimal degrees with a hemisphere
/// letter, such as `36.558426172S`, in radians.
fn angle(field: &str, text: &str, positive: char, negative: char) -> Result<f64, String> {
    let text = text.trim();
    let (value, sign) = match text.chars().last() {
        Some(c) if c.eq_ignore_ascii_case(&positive) => (&text[..text.len() - 1], 1.0),
        Some(c) if c.eq_ignore_ascii_case(&negative) => (&text[..text.len() - 1], -1.0),
        _ => {
            return Err(format!(
                "{field} should end in {positive} or {negative}, not {text:?}"
            ));
        }
    };
    let degrees = number(field, value)?;
    if !(0.0..=180.0).contains(&degrees) {
        return Err(format!("{field} {text} is out of range"));
    }
    Ok((sign * degrees).to_radians())
}

/// A date and time as TDEF writes them (`16 12 2019`, `21 48 47.0`).
fn time(date: &str, clock: &str) -> Option<Epoch> {
    let d: Vec<&str> = date.split_whitespace().collect();
    let c: Vec<&str> = clock.split_whitespace().collect();
    let [day, month, year] = d[..] else {
        return None;
    };
    let [hour, minute, second] = c[..] else {
        return None;
    };
    let seconds: f64 = second.parse().ok()?;
    if !(0.0..61.0).contains(&seconds) {
        return None;
    }
    let whole = seconds.floor();
    Epoch::maybe_from_gregorian(
        year.parse().ok()?,
        month.parse().ok()?,
        day.parse().ok()?,
        hour.parse().ok()?,
        minute.parse().ok()?,
        whole as u8,
        (((seconds - whole) * 1e9).round() as u32).min(999_999_999),
        TimeScale::UTC,
    )
    .ok()
}

/// The frame of a TBC coordinate system's name, such as `Australia/GDA2020`.
fn system_frame(system: &str) -> Option<FrameId> {
    let name = system.rsplit('/').next().unwrap_or(system).trim();
    FrameId::parse(name).ok()
}

/// Reads a TDEF file. `frame` is the frame of its vectors, which the file
/// doesn't record; `epoch`, if given, is their epoch in a dynamic frame,
/// instead of each vector's observation time.
pub fn read(
    file: &str,
    text: &str,
    frame: FrameId,
    epoch: Option<CoordinateEpoch>,
) -> Result<Survey, FormatError> {
    let mut survey = Survey::default();
    let mut section = String::new();
    let mut general: BTreeMap<String, String> = BTreeMap::new();
    // Stations and keyed-in coordinates, by name: (line, fields, keyed in).
    let mut positions: BTreeMap<String, (usize, Vec<String>, bool)> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut skipped: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut separator = ':';
    let mut missing = "?".to_owned();
    let mut checked = false;

    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let at = |m: String| FormatError::at(file, line, m);
        let content = raw.trim();
        if content.is_empty() {
            continue;
        }
        if let Some(name) = content.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.trim().to_ascii_lowercase();
            continue;
        }
        let Some((key, value)) = content.split_once('=') else {
            return Err(at(format!(
                "{content:?} isn't a TDEF record (key=value) or a [section]"
            )));
        };
        let key = key.trim();
        if section == "general" {
            general.insert(key.to_ascii_lowercase(), value.trim().to_owned());
            continue;
        }
        // The format's settings apply from the first record after [General].
        if !checked {
            checked = true;
            if let Some(s) = general.get("separator") {
                let mut chars = s.chars();
                separator = match (chars.next(), chars.next()) {
                    (Some(c), None) => c,
                    _ => return Err(at(format!("the separator {s:?} isn't one character"))),
                };
            }
            if let Some(m) = general.get("missingvalue") {
                missing.clone_from(m);
            }
            for unit in ["coordinateunits", "distanceunits", "elevationheightunits"] {
                if let Some(u) = general.get(unit)
                    && !u.eq_ignore_ascii_case("meters")
                    && !u.eq_ignore_ascii_case("metres")
                {
                    return Err(at(format!(
                        "its {unit} are {u}; only metres are read. Export again in metres"
                    )));
                }
            }
            match general.get("gpsvectors") {
                Some(v) if v.eq_ignore_ascii_case("marktomark") => {}
                Some(v) => {
                    return Err(at(format!(
                        "its vectors are {v}, not mark to mark, so antenna heights would have \
                         to be applied. Export them mark to mark"
                    )));
                }
                None => survey.notes.push(Note {
                    origin: Origin {
                        file: file.to_owned(),
                        line,
                    },
                    message: "the file doesn't say whether its vectors are mark to mark; they \
                              were taken to be"
                        .into(),
                }),
            }
        }
        let fields: Vec<String> = value
            .split(separator)
            .map(|f| f.trim().to_owned())
            .collect();
        let field = |i: usize| -> Option<&str> {
            fields
                .get(i)
                .map(String::as_str)
                .filter(|f| !f.is_empty() && *f != missing)
        };
        match (section.as_str(), key.to_ascii_lowercase().as_str()) {
            ("gps", "vector") => {
                let need = |i: usize, what: &str| {
                    field(i).ok_or_else(|| at(format!("the vector has no {what}")))
                };
                let from = need(2, "first mark")?.to_owned();
                let to = need(3, "second mark")?.to_owned();
                let label = format!("vector {from}-{to}");
                let value = |i: usize, what: &str| -> Result<f64, FormatError> {
                    number(&format!("{label}'s {what}"), need(i, what)?).map_err(at)
                };
                let vector = Vector3::new(value(4, "ΔX")?, value(5, "ΔY")?, value(6, "ΔZ")?);
                let [xx, xy, xz, yy, yz, zz] = [
                    value(7, "XX variance")?,
                    value(8, "XY covariance")?,
                    value(9, "XZ covariance")?,
                    value(10, "YY variance")?,
                    value(11, "YZ covariance")?,
                    value(12, "ZZ variance")?,
                ];
                let covariance = Matrix3::new(xx, xy, xz, xy, yy, yz, xz, yz, zz);
                if covariance.cholesky().is_none() {
                    return Err(at(format!(
                        "{label}'s covariance (XX, XY, XZ, YY, YZ, ZZ = {xx}, {xy}, {xz}, {yy}, \
                         {yz}, {zz}) isn't positive definite"
                    )));
                }
                let observed = match (field(19), field(20), field(21), field(22)) {
                    (Some(sd), Some(st), Some(ed), Some(et)) => {
                        match (time(sd, st), time(ed, et)) {
                            (Some(s), Some(e)) if e >= s => Some(s + (e - s) * 0.5),
                            _ => {
                                return Err(at(format!(
                                    "{label}'s observation times ({sd} {st} to {ed} {et}) \
                                     aren't dates and times TDEF writes"
                                )));
                            }
                        }
                    }
                    _ => None,
                };
                let vector_epoch = if frame.is_static() {
                    None
                } else if let Some(e) = epoch {
                    Some(e)
                } else {
                    let middle = observed.ok_or_else(|| {
                        at(format!(
                            "{label} has no observation times, so its epoch in {frame} isn't \
                             known; give the import an epoch"
                        ))
                    })?;
                    Some(CoordinateEpoch::from_time(middle).map_err(|e| at(e.to_string()))?)
                };
                survey.measurements.push(Measurement {
                    kind: MeasurementKind::Baseline,
                    elements: vec![Element::Vector { from, to, vector }],
                    covariance: DMatrix::from_column_slice(3, 3, covariance.as_slice()),
                    reference: Reference {
                        frame,
                        epoch: vector_epoch,
                    },
                    scaling: Scaling::default(),
                    ignored: false,
                    source: None,
                    id: None,
                    origin: Origin {
                        file: file.to_owned(),
                        line,
                    },
                });
            }
            ("stations", "station") | ("keyed in coordinates", "llcoord") => {
                let keyed = section != "stations";
                let Some(name) = field(2).map(str::to_owned) else {
                    return Err(at("the station has no name".into()));
                };
                match positions.get(&name) {
                    // Keyed-in coordinates win over the processor's stations.
                    Some((_, _, true)) => continue,
                    Some(_) if !keyed => continue,
                    None => order.push(name.clone()),
                    Some(_) => {}
                }
                positions.insert(name, (line, fields.clone(), keyed));
            }
            (section, _) => {
                let what = if section.is_empty() {
                    key.to_owned()
                } else {
                    format!("{key} ({section})")
                };
                let entry = skipped.entry(what).or_insert((0, line));
                entry.0 += 1;
            }
        }
    }

    // Starting positions, in the project's coordinate system.
    if !positions.is_empty() {
        let system = general.get("projcoordinatesystem").cloned();
        match system.as_deref().and_then(system_frame) {
            Some(station_frame) if station_frame.is_static() => {
                let reference = Reference {
                    frame: station_frame,
                    epoch: None,
                };
                let (mut keyed, mut processed) = (0, 0);
                for name in order {
                    let Some((line, fields, is_keyed)) = positions.remove(&name) else {
                        continue;
                    };
                    let at = |m: String| FormatError::at(file, line, m);
                    let get = |i: usize, what: &str| {
                        fields
                            .get(i)
                            .map(String::as_str)
                            .filter(|f| !f.is_empty() && *f != missing)
                            .ok_or_else(|| at(format!("{name} has no {what}")))
                    };
                    let latitude =
                        angle(&format!("{name}'s latitude"), get(3, "latitude")?, 'N', 'S')
                            .map_err(at)?;
                    let longitude = angle(
                        &format!("{name}'s longitude"),
                        get(4, "longitude")?,
                        'E',
                        'W',
                    )
                    .map_err(at)?;
                    let height =
                        number(&format!("{name}'s height"), get(5, "height")?).map_err(at)?;
                    if is_keyed {
                        keyed += 1;
                    } else {
                        processed += 1;
                    }
                    survey.stations.push(Station {
                        name,
                        description: None,
                        coordinate: Some(Coordinate {
                            value: CoordinateValue::Geographic {
                                latitude_rad: latitude,
                                longitude_rad: longitude,
                                height_m: height,
                                height: HeightType::Ellipsoidal,
                            },
                            reference,
                        }),
                        constraint: Constraint::Free,
                        origin: Origin {
                            file: file.to_owned(),
                            line,
                        },
                    });
                }
                survey.notes.push(Note {
                    origin: Origin {
                        file: file.to_owned(),
                        line: 1,
                    },
                    message: format!(
                        "{} marks are starting positions only ({keyed} keyed in, {processed} \
                         from processing), in {station_frame}; to hold control, import it",
                        keyed + processed
                    ),
                });
            }
            _ => survey.notes.push(Note {
                origin: Origin {
                    file: file.to_owned(),
                    line: 1,
                },
                message: format!(
                    "its {} station coordinates were skipped: the coordinate system {} isn't one \
                     whose frame is known",
                    positions.len(),
                    system.as_deref().unwrap_or("(none given)")
                ),
            }),
        }
    }
    for (what, (count, line)) in skipped {
        survey.notes.push(Note {
            origin: Origin {
                file: file.to_owned(),
                line,
            },
            message: format!(
                "skipped {count} {what} record{}: only GPS vectors and station coordinates are \
                 read",
                if count == 1 { "" } else { "s" }
            ),
        });
    }
    if survey.measurements.is_empty() {
        return Err(FormatError::in_file(
            file,
            "it has no GPS vectors (Vector records in a [GPS] section)",
        ));
    }
    Ok(survey)
}
