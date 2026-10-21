//! DynaML, DynAdjust's XML format (Geoscience Australia): stations, GNSS
//! baselines (type G), baseline clusters (X) and point clusters (Y).
//!
//! The format is described in the DynAdjust User's Guide, appendix B.3, and
//! by `DynaML.xsd`. Points worth knowing:
//!
//! - Station latitudes and longitudes are in HP notation (`ddd.mmssss`).
//!   `LLH` heights are orthometric and `LLh` ellipsoidal; `UTM` heights are
//!   orthometric.
//! - Constraints are a letter per coordinate, `C` held and `F` free.
//! - The root's `referenceframe` and `epoch` (`dd.mm.yyyy`) apply to stations
//!   and to measurements that don't give their own.
//! - Cluster covariances: `GPSCovariance` (or `PointCovariance`) blocks follow
//!   each element's own variances, one for each later element, with `m12`
//!   the covariance of this element's X with the later one's Y.
//! - `Vscale`, `Pscale`, `Lscale` and `Hscale` scale the covariance as
//!   [`Scaling`] describes. As in DynAdjust, a scale below 1e-6 means 1.
//! - `<Ignore>*</Ignore>` leaves a measurement out of the adjustment.
//!
//! Terrestrial measurements (angles, distances, heights and so on) are skipped
//! with a note: they arrive with M23.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use gnss_datum::FrameId;
use nalgebra::{DMatrix, Vector3};

use crate::error::{FormatError, WriteError};
use crate::survey::{
    Constraint, Coordinate, CoordinateValue, Element, HeightType, Measurement, MeasurementKind,
    Note, Origin, Reference, Scaling, Station, Survey,
};
use crate::text::{dmy_epoch, epoch_dmy, hp_degrees, hp_text, number};
use crate::xml::{Document, Node, escape};

/// DynAdjust treats scales below this as 1.
const SMALLEST_SCALE: f64 = 1e-6;

/// The names of DynaML's measurement types, for notes about skipped ones.
const TYPE_NAMES: [(char, &str); 20] = [
    ('A', "horizontal angle"),
    ('B', "geodetic azimuth"),
    ('C', "chord distance"),
    ('D', "direction set"),
    ('E', "ellipsoid arc"),
    ('G', "GNSS baseline"),
    ('H', "orthometric height"),
    ('I', "astronomic latitude"),
    ('J', "astronomic longitude"),
    ('K', "astronomic azimuth"),
    ('L', "level difference"),
    ('M', "mean sea level arc"),
    ('P', "geodetic latitude"),
    ('Q', "geodetic longitude"),
    ('R', "ellipsoidal height"),
    ('S', "slope distance"),
    ('V', "zenith distance"),
    ('X', "GNSS baseline cluster"),
    ('Y', "GNSS point cluster"),
    ('Z', "vertical angle"),
];

/// Reads a DynaML station, measurement or combined file.
///
/// `file` names the file in messages.
pub fn read(file: &str, text: &str) -> Result<Survey, FormatError> {
    let mut document = Document::open(file, text)?;
    if document.root.name != "DnaXmlFormat" {
        return Err(FormatError::at(
            file,
            document.root.line,
            format!(
                "it isn't a DynaML file: the root element is <{}>, not <DnaXmlFormat>",
                document.root.name
            ),
        ));
    }
    let root_line = document.root.line;
    let mut notes = Vec::new();
    let frame = document
        .root
        .attribute("referenceframe")
        .unwrap_or("GDA2020");
    let epoch = document.root.attribute("epoch");
    let reference = reference(file, root_line, frame, epoch, &mut notes)?;

    let mut survey = Survey {
        notes,
        ..Survey::default()
    };
    // Skipped measurement types: how many, and the first line of each.
    let mut skipped: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    while let Some(node) = document.next_child()? {
        match node.name.as_str() {
            "DnaStation" => survey.stations.push(station(file, &node, reference)?),
            "DnaMeasurement" => {
                let kind = required(file, &node, "Type")?;
                match kind {
                    "G" | "X" | "Y" => {
                        let measurement = measurement(file, &node, reference, &mut survey.notes)?;
                        survey.measurements.push(measurement);
                    }
                    other => {
                        let entry = skipped.entry(other.to_owned()).or_insert((0, node.line));
                        entry.0 += 1;
                    }
                }
            }
            other => survey.notes.push(Note {
                origin: Origin {
                    file: file.to_owned(),
                    line: node.line,
                },
                message: format!("skipped <{other}>, which isn't a DynaML station or measurement"),
            }),
        }
    }
    for (kind, (count, line)) in skipped {
        let name = TYPE_NAMES
            .iter()
            .find(|(c, _)| kind.len() == 1 && kind.starts_with(*c))
            .map_or("unknown", |(_, n)| n);
        survey.notes.push(Note {
            origin: Origin {
                file: file.to_owned(),
                line,
            },
            message: format!(
                "skipped {count} measurement{} of type {kind} ({name}): only GNSS measurements \
                 (G, X and Y) are adjusted so far",
                if count == 1 { "" } else { "s" }
            ),
        });
    }
    Ok(survey)
}

/// A frame and epoch from DynaML's text. A static frame takes no epoch: one
/// that differs from the frame's reference epoch is noted and dropped.
fn reference(
    file: &str,
    line: usize,
    frame: &str,
    epoch: Option<&str>,
    notes: &mut Vec<Note>,
) -> Result<Reference, FormatError> {
    let frame_id = FrameId::parse(frame).map_err(|e| FormatError::at(file, line, e.to_string()))?;
    let epoch = epoch
        .filter(|e| !e.trim().is_empty())
        .map(|e| dmy_epoch("the epoch", e))
        .transpose()
        .map_err(|m| FormatError::at(file, line, m))?;
    match (frame_id.reference_epoch(), epoch) {
        (Some(reference_epoch), Some(given)) => {
            if given != reference_epoch {
                notes.push(Note {
                    origin: Origin {
                        file: file.to_owned(),
                        line,
                    },
                    message: format!(
                        "{frame_id} is a static frame, so its epoch is {reference_epoch}; the \
                         file's epoch {} was ignored",
                        epoch_dmy(given).unwrap_or_default()
                    ),
                });
            }
            Ok(Reference {
                frame: frame_id,
                epoch: None,
            })
        }
        (Some(_), None) => Ok(Reference {
            frame: frame_id,
            epoch: None,
        }),
        (None, Some(e)) => Ok(Reference {
            frame: frame_id,
            epoch: Some(e),
        }),
        (None, None) => Err(FormatError::at(
            file,
            line,
            format!("{frame_id} is a dynamic frame, so it needs an epoch, but none is given"),
        )),
    }
}

fn required<'n>(file: &str, node: &'n Node, name: &str) -> Result<&'n str, FormatError> {
    match node.text_of(name) {
        Some(text) if !text.is_empty() => Ok(text),
        _ => Err(FormatError::at(
            file,
            node.line,
            format!("<{}> has no <{name}>", node.name),
        )),
    }
}

fn value(file: &str, node: &Node, name: &str) -> Result<f64, FormatError> {
    let text = required(file, node, name)?;
    number(name, text).map_err(|m| FormatError::at(file, node.line, m))
}

fn station(file: &str, node: &Node, reference: Reference) -> Result<Station, FormatError> {
    let at = |message: String| FormatError::at(file, node.line, message);
    let name = required(file, node, "Name")?.to_owned();
    let code = node.text_of("Constraints").unwrap_or("FFF");
    let constraint = match code.to_ascii_uppercase().as_str() {
        "FFF" => Constraint::Free,
        "CCC" => Constraint::Fixed,
        c if c.len() == 3 && c.bytes().all(|b| b == b'C' || b == b'F') => {
            Constraint::Partial(code.to_owned())
        }
        _ => {
            return Err(at(format!(
                "station {name}: constraints {code:?} should be three letters, C (held) or F (free)"
            )));
        }
    };
    let kind = required(file, node, "Type")?;
    let coord = node
        .child("StationCoord")
        .ok_or_else(|| at(format!("station {name} has no <StationCoord>")))?;
    let field = |n: &str| -> Result<&str, FormatError> {
        coord
            .text_of(n)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| at(format!("station {name}: <StationCoord> has no <{n}>")))
    };
    let parse = |n: &str| -> Result<f64, FormatError> {
        number(&format!("station {name}'s {n}"), field(n)?).map_err(at)
    };
    let value = match kind {
        "LLH" | "LLh" => {
            let latitude =
                hp_degrees(&format!("station {name}'s latitude"), field("XAxis")?).map_err(at)?;
            let longitude =
                hp_degrees(&format!("station {name}'s longitude"), field("YAxis")?).map_err(at)?;
            if latitude.abs() > 90.0 {
                return Err(at(format!(
                    "station {name}'s latitude is {latitude}°, beyond the poles"
                )));
            }
            CoordinateValue::Geographic {
                latitude_rad: latitude.to_radians(),
                longitude_rad: longitude.to_radians(),
                height_m: parse("Height")?,
                height: if kind == "LLh" {
                    HeightType::Ellipsoidal
                } else {
                    HeightType::Orthometric
                },
            }
        }
        "XYZ" => CoordinateValue::Cartesian(Vector3::new(
            parse("XAxis")?,
            parse("YAxis")?,
            parse("Height")?,
        )),
        "UTM" => {
            let zone_text = coord.text_of("HemisphereZone").unwrap_or("").trim();
            let digits = zone_text
                .strip_prefix(['S', 's'])
                .unwrap_or(zone_text)
                .trim();
            if zone_text.starts_with(['N', 'n']) {
                return Err(at(format!(
                    "station {name} is in the northern hemisphere ({zone_text}); only southern \
                     UTM zones are read so far"
                )));
            }
            let zone = digits
                .parse::<u8>()
                .ok()
                .filter(|z| (1..=60).contains(z))
                .ok_or_else(|| {
                    at(format!(
                        "station {name}: UTM coordinates need a zone such as S55 in \
                         <HemisphereZone>, not {zone_text:?}"
                    ))
                })?;
            CoordinateValue::Grid {
                easting_m: parse("XAxis")?,
                northing_m: parse("YAxis")?,
                zone,
                height_m: parse("Height")?,
                height: HeightType::Orthometric,
            }
        }
        other => {
            return Err(at(format!(
                "station {name}: coordinate type {other:?} isn't LLH, LLh, XYZ or UTM"
            )));
        }
    };
    Ok(Station {
        name,
        description: node
            .text_of("Description")
            .filter(|d| !d.is_empty())
            .map(str::to_owned),
        coordinate: Some(Coordinate { value, reference }),
        constraint,
        origin: Origin {
            file: file.to_owned(),
            line: node.line,
        },
    })
}

/// An element as read, before the covariance is assembled.
struct Part {
    element: Element,
    variances: [f64; 6],
    covariances: Vec<[f64; 9]>,
    line: usize,
}

fn measurement(
    file: &str,
    node: &Node,
    default: Reference,
    notes: &mut Vec<Note>,
) -> Result<Measurement, FormatError> {
    let at = |line: usize, message: String| FormatError::at(file, line, message);
    let kind_text = required(file, node, "Type")?;
    let kind = match kind_text {
        "G" => MeasurementKind::Baseline,
        "X" => MeasurementKind::BaselineCluster,
        _ => MeasurementKind::PointCluster,
    };
    let reference = match (node.text_of("ReferenceFrame"), node.text_of("Epoch")) {
        (None, None) => default,
        (frame, epoch) => {
            let frame = frame
                .filter(|f| !f.is_empty())
                .unwrap_or(default.frame.name());
            let epoch_text;
            let epoch = match epoch {
                Some(e) => Some(e),
                None => {
                    epoch_text = default.epoch.and_then(epoch_dmy);
                    epoch_text.as_deref()
                }
            };
            reference(file, node.line, frame, epoch, notes)?
        }
    };
    if kind == MeasurementKind::PointCluster {
        let coords = node.text_of("Coords").unwrap_or("XYZ");
        if coords != "XYZ" {
            return Err(at(
                node.line,
                format!(
                    "point cluster with {coords} coordinates: only XYZ point clusters are read so far"
                ),
            ));
        }
    }

    let mut scale = |name: &str| -> Result<f64, FormatError> {
        let Some(text) = node.text_of(name).filter(|t| !t.is_empty()) else {
            return Ok(1.0);
        };
        let v = number(name, text).map_err(|m| at(node.line, m))?;
        if v < SMALLEST_SCALE {
            notes.push(Note {
                origin: Origin {
                    file: file.to_owned(),
                    line: node.line,
                },
                message: format!("{name} {v} was taken as 1, as DynAdjust does"),
            });
            return Ok(1.0);
        }
        Ok(v)
    };
    let scaling = Scaling {
        v: scale("Vscale")?,
        p: scale("Pscale")?,
        l: scale("Lscale")?,
        h: scale("Hscale")?,
    };

    // The elements: First (and Second) names, then the values.
    let mut parts: Vec<Part> = Vec::new();
    let mut first: Option<&str> = None;
    let mut second: Option<&str> = None;
    let values_name = if kind == MeasurementKind::PointCluster {
        "Clusterpoint"
    } else {
        "GPSBaseline"
    };
    for child in &node.children {
        match child.name.as_str() {
            "First" => first = Some(child.text.as_str()),
            "Second" => second = Some(child.text.as_str()),
            name if name == values_name => {
                let from = first.take().filter(|f| !f.is_empty()).ok_or_else(|| {
                    at(
                        child.line,
                        format!("<{name}> has no <First> station before it"),
                    )
                })?;
                let xyz = Vector3::new(
                    value(file, child, "X")?,
                    value(file, child, "Y")?,
                    value(file, child, "Z")?,
                );
                let element = if kind == MeasurementKind::PointCluster {
                    Element::Position {
                        station: from.to_owned(),
                        position: xyz,
                    }
                } else {
                    let to = second.take().filter(|s| !s.is_empty()).ok_or_else(|| {
                        at(
                            child.line,
                            format!("<{name}> has no <Second> station before it"),
                        )
                    })?;
                    if to == from {
                        return Err(at(
                            child.line,
                            format!("the baseline from {from} ends at {from} too"),
                        ));
                    }
                    Element::Vector {
                        from: from.to_owned(),
                        to: to.to_owned(),
                        vector: xyz,
                    }
                };
                let mut variances = [0.0; 6];
                for (v, n) in variances.iter_mut().zip([
                    "SigmaXX", "SigmaXY", "SigmaXZ", "SigmaYY", "SigmaYZ", "SigmaZZ",
                ]) {
                    *v = value(file, child, n)?;
                }
                let block_name = if kind == MeasurementKind::PointCluster {
                    "PointCovariance"
                } else {
                    "GPSCovariance"
                };
                let mut covariances = Vec::new();
                for block in child.children.iter().filter(|c| c.name == block_name) {
                    let mut m = [0.0; 9];
                    for (v, n) in m.iter_mut().zip([
                        "m11", "m12", "m13", "m21", "m22", "m23", "m31", "m32", "m33",
                    ]) {
                        *v = value(file, block, n)?;
                    }
                    covariances.push(m);
                }
                parts.push(Part {
                    element,
                    variances,
                    covariances,
                    line: child.line,
                });
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        return Err(at(
            node.line,
            format!("the type {kind_text} measurement has no <{values_name}>"),
        ));
    }
    let n = parts.len();
    if kind == MeasurementKind::Baseline && n > 1 {
        return Err(at(
            node.line,
            format!("a type G measurement holds one baseline, but this one has {n}: use type X"),
        ));
    }
    if let Some(total) = node.text_of("Total").filter(|t| !t.is_empty()) {
        let total: usize = total
            .parse()
            .map_err(|_| at(node.line, format!("<Total> isn't a count: {total:?}")))?;
        if total != n && kind != MeasurementKind::Baseline {
            return Err(at(
                node.line,
                format!("<Total> says {total}, but the cluster has {n}"),
            ));
        }
    }
    // Check the blocks first, so the matrix's size is bounded by the file's.
    for (i, part) in parts.iter().enumerate() {
        if part.covariances.len() != n - i - 1 {
            return Err(at(
                part.line,
                format!(
                    "element {} of {n} has {} covariance blocks, but needs one for each later \
                     element ({})",
                    i + 1,
                    part.covariances.len(),
                    n - i - 1
                ),
            ));
        }
    }
    let mut covariance = DMatrix::zeros(3 * n, 3 * n);
    for (i, part) in parts.iter().enumerate() {
        let [xx, xy, xz, yy, yz, zz] = part.variances;
        let block = nalgebra::Matrix3::new(xx, xy, xz, xy, yy, yz, xz, yz, zz);
        covariance
            .view_mut((3 * i, 3 * i), (3, 3))
            .copy_from(&block);
        for (k, m) in part.covariances.iter().enumerate() {
            let j = i + 1 + k;
            for r in 0..3 {
                for c in 0..3 {
                    covariance[(3 * i + r, 3 * j + c)] = m[3 * r + c];
                    covariance[(3 * j + c, 3 * i + r)] = m[3 * r + c];
                }
            }
        }
    }
    let ignored = node.text_of("Ignore").is_some_and(|t| t.trim() == "*");
    let text = |name: &str| {
        node.text_of(name)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    Ok(Measurement {
        kind,
        elements: parts.into_iter().map(|p| p.element).collect(),
        covariance,
        reference,
        scaling,
        ignored,
        source: text("Source"),
        id: text("MeasurementID").or_else(|| text("ClusterID")),
        origin: Origin {
            file: file.to_owned(),
            line: node.line,
        },
    })
}

/// The root element's attributes for a frame and epoch.
fn root_attributes(reference: Reference) -> String {
    let epoch = reference
        .epoch
        .or(reference.frame.reference_epoch())
        .and_then(epoch_dmy)
        .unwrap_or_default();
    format!(
        "referenceframe=\"{}\" epoch=\"{epoch}\"",
        escape(reference.frame.name())
    )
}

fn header(kind: &str, reference: Reference) -> String {
    format!(
        "<?xml version=\"1.0\"?>\n<DnaXmlFormat type=\"{kind}\" {} \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xsi:noNamespaceSchemaLocation=\"DynaML.xsd\">\n",
        root_attributes(reference)
    )
}

/// Writes stations as a DynaML station file in `reference`'s frame and epoch.
///
/// Every station with a coordinate must be in that frame and epoch; stations
/// without one are left out, since DynaML needs a coordinate. Geographic
/// coordinates are written in HP notation to 1e-6 seconds (0.03 mm).
pub fn write_stations(stations: &[Station], reference: Reference) -> Result<String, WriteError> {
    let mut out = header("Station File", reference);
    for station in stations {
        let Some(coordinate) = &station.coordinate else {
            continue;
        };
        if !same_reference(coordinate.reference, reference) {
            return Err(WriteError(format!(
                "station {} is in {}, but the file is in {}",
                station.name,
                describe(coordinate.reference),
                describe(reference)
            )));
        }
        let constraint = match &station.constraint {
            Constraint::Free => "FFF".to_owned(),
            Constraint::Fixed => "CCC".to_owned(),
            Constraint::Partial(code) => code.clone(),
        };
        let (kind, x, y, height, zone) = match coordinate.value {
            CoordinateValue::Cartesian(xyz) => (
                "XYZ",
                format!("{}", xyz.x),
                format!("{}", xyz.y),
                format!("{}", xyz.z),
                None,
            ),
            CoordinateValue::Geographic {
                latitude_rad,
                longitude_rad,
                height_m,
                height,
            } => (
                match height {
                    HeightType::Ellipsoidal => "LLh",
                    HeightType::Orthometric => "LLH",
                },
                hp_text(latitude_rad.to_degrees(), 6),
                hp_text(longitude_rad.to_degrees(), 6),
                format!("{height_m:.4}"),
                None,
            ),
            CoordinateValue::Grid {
                easting_m,
                northing_m,
                zone,
                height_m,
                height,
            } => {
                if height == HeightType::Ellipsoidal {
                    return Err(WriteError(format!(
                        "station {}: DynaML's UTM heights are orthometric, but this one is ellipsoidal",
                        station.name
                    )));
                }
                (
                    "UTM",
                    format!("{easting_m:.4}"),
                    format!("{northing_m:.4}"),
                    format!("{height_m:.4}"),
                    Some(zone),
                )
            }
        };
        let name = escape(&station.name);
        let _ = write!(
            out,
            "  <DnaStation>\n    <Name>{name}</Name>\n    <Constraints>{}</Constraints>\n    \
             <Type>{kind}</Type>\n    <StationCoord>\n      <Name>{name}</Name>\n      \
             <XAxis>{x}</XAxis>\n      <YAxis>{y}</YAxis>\n      <Height>{height}</Height>\n",
            escape(&constraint)
        );
        if let Some(zone) = zone {
            let _ = writeln!(out, "      <HemisphereZone>S{zone}</HemisphereZone>");
        }
        out.push_str("    </StationCoord>\n");
        if let Some(description) = &station.description {
            let _ = writeln!(
                out,
                "    <Description>{}</Description>",
                escape(description)
            );
        }
        out.push_str("  </DnaStation>\n");
    }
    out.push_str("</DnaXmlFormat>\n");
    Ok(out)
}

/// Writes GNSS measurements as a DynaML measurement file whose root is in
/// `reference`'s frame and epoch. Each measurement gives its own frame and
/// epoch, so they needn't match the root.
///
/// Numbers are written so they read back exactly.
pub fn write_measurements(measurements: &[Measurement], reference: Reference) -> String {
    let mut out = header("Measurement File", reference);
    for m in measurements {
        let kind = match m.kind {
            MeasurementKind::Baseline if m.elements.len() == 1 => "G",
            MeasurementKind::Baseline | MeasurementKind::BaselineCluster => "X",
            MeasurementKind::PointCluster => "Y",
        };
        let epoch = m
            .reference
            .epoch
            .or(m.reference.frame.reference_epoch())
            .and_then(epoch_dmy)
            .unwrap_or_default();
        let _ = write!(out, "  <DnaMeasurement>\n    <Type>{kind}</Type>\n");
        if let Some(source) = &m.source {
            let _ = writeln!(out, "    <Source>{}</Source>", escape(source));
        }
        out.push_str(if m.ignored {
            "    <Ignore>*</Ignore>\n"
        } else {
            "    <Ignore/>\n"
        });
        let _ = write!(
            out,
            "    <ReferenceFrame>{}</ReferenceFrame>\n    <Epoch>{epoch}</Epoch>\n    \
             <Vscale>{}</Vscale>\n    <Pscale>{}</Pscale>\n    <Lscale>{}</Lscale>\n    \
             <Hscale>{}</Hscale>\n",
            escape(m.reference.frame.name()),
            m.scaling.v,
            m.scaling.p,
            m.scaling.l,
            m.scaling.h
        );
        if kind == "Y" {
            out.push_str("    <Coords>XYZ</Coords>\n");
        }
        if kind != "G" {
            let _ = writeln!(out, "    <Total>{}</Total>", m.elements.len());
        }
        let n = m.elements.len();
        for (i, element) in m.elements.iter().enumerate() {
            let (values, block, xyz) = match element {
                Element::Vector { from, to, vector } => {
                    let _ = write!(
                        out,
                        "    <First>{}</First>\n    <Second>{}</Second>\n",
                        escape(from),
                        escape(to)
                    );
                    ("GPSBaseline", "GPSCovariance", vector)
                }
                Element::Position { station, position } => {
                    let _ = writeln!(out, "    <First>{}</First>", escape(station));
                    ("Clusterpoint", "PointCovariance", position)
                }
            };
            let c = |r: usize, s: usize| m.covariance[(3 * i + r, 3 * i + s)];
            let _ = write!(
                out,
                "    <{values}>\n      <X>{}</X>\n      <Y>{}</Y>\n      <Z>{}</Z>\n",
                xyz.x, xyz.y, xyz.z
            );
            if i == 0
                && let Some(id) = &m.id
            {
                let _ = writeln!(out, "      <MeasurementID>{}</MeasurementID>", escape(id));
            }
            let _ = write!(
                out,
                "      <SigmaXX>{:e}</SigmaXX>\n      <SigmaXY>{:e}</SigmaXY>\n      \
                 <SigmaXZ>{:e}</SigmaXZ>\n      <SigmaYY>{:e}</SigmaYY>\n      \
                 <SigmaYZ>{:e}</SigmaYZ>\n      <SigmaZZ>{:e}</SigmaZZ>\n",
                c(0, 0),
                c(0, 1),
                c(0, 2),
                c(1, 1),
                c(1, 2),
                c(2, 2)
            );
            for j in i + 1..n {
                let _ = writeln!(out, "      <{block}>");
                for r in 0..3 {
                    for s in 0..3 {
                        let _ = writeln!(
                            out,
                            "        <m{}{}>{:e}</m{}{}>",
                            r + 1,
                            s + 1,
                            m.covariance[(3 * i + r, 3 * j + s)],
                            r + 1,
                            s + 1
                        );
                    }
                }
                let _ = writeln!(out, "      </{block}>");
            }
            let _ = writeln!(out, "    </{values}>");
        }
        out.push_str("  </DnaMeasurement>\n");
    }
    out.push_str("</DnaXmlFormat>\n");
    out
}

/// True if two references are the same frame at the same epoch, treating a
/// static frame's missing epoch as its reference epoch.
fn same_reference(a: Reference, b: Reference) -> bool {
    let epoch = |r: Reference| r.epoch.or(r.frame.reference_epoch());
    a.frame == b.frame && epoch(a) == epoch(b)
}

fn describe(reference: Reference) -> String {
    match (reference.frame.is_static(), reference.epoch) {
        (false, Some(e)) => format!("{} at {}", reference.frame, e),
        _ => reference.frame.to_string(),
    }
}
