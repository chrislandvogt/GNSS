//! STAR*NET GPS vector files (MicroSurvey), as commercial GNSS packages
//! export them for STAR*NET.
//!
//! Each vector is a group of records:
//!
//! ```text
//! .GPS WEIGHT COVARIANCE
//! G0 'Session 12  base 0759 to rover 3040
//! G1 0759-3040   2022.0123  -467.4561  2612.3678
//! G2 1.2345E-06  1.4321E-06  9.8765E-07
//! G3 -2.1E-07  3.4E-07  -1.2E-07
//! ```
//!
//! - `G0` names the vector, after an apostrophe. It's optional.
//! - `G1` gives the stations, `FROM-TO`, and the ECEF components in metres.
//! - `G2` and `G3` weight it. With `.GPS WEIGHT COVARIANCE`, which STAR*NET
//!   writes when it imports vectors, `G2` holds the variances of X, Y and Z
//!   and `G3` the covariances XY, XZ and YZ, in m². With `.GPS WEIGHT
//!   STDERRCORR`, `G2` holds standard errors in metres and `G3` correlations.
//!   Without the option, covariances are assumed, and values that can only be
//!   correlations are refused, since mixing the two up is a classic mistake.
//! - `.GPS FACTOR h [v]` scales the standard errors, horizontally and
//!   vertically (the variances by the square).
//!
//! The files don't say which frame or epoch the vectors are in, so the caller
//! does. Other STAR*NET records (terrestrial observations, coordinates) are
//! skipped with a note; `G4` offsets and antenna heights on `G1` are refused,
//! since ignoring them would change the vectors.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use nalgebra::{DMatrix, Vector3};

use crate::error::{FormatError, WriteError};
use crate::survey::{
    Element, Measurement, MeasurementKind, Note, Origin, Reference, Scaling, Survey,
};
use crate::text::number;

/// How `G2` and `G3` weight the vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Weighting {
    Covariance,
    StdErrCorrelation,
}

/// A vector being read.
struct Pending {
    name: Option<String>,
    from: String,
    to: String,
    vector: Vector3<f64>,
    g2: Option<[f64; 3]>,
    g3: Option<[f64; 3]>,
    line: usize,
}

/// Reads a STAR*NET GPS vector file. `reference` gives the frame and epoch
/// of its vectors, which the file doesn't record.
pub fn read(file: &str, text: &str, reference: Reference) -> Result<Survey, FormatError> {
    let mut survey = Survey::default();
    let mut weighting: Option<Weighting> = None;
    let mut factor = (1.0, 1.0);
    let mut name: Option<String> = None;
    let mut pending: Option<Pending> = None;
    let mut skipped: BTreeMap<String, (usize, usize)> = BTreeMap::new();

    let finish = |p: Pending,
                  weighting: Option<Weighting>,
                  factor: (f64, f64),
                  survey: &mut Survey|
     -> Result<(), FormatError> {
        let at = |m: String| FormatError::at(file, p.line, m);
        let label = format!("vector {}-{}", p.from, p.to);
        let g2 =
            p.g2.ok_or_else(|| at(format!("{label} has no G2 record to weight it")))?;
        let g3 = p.g3.unwrap_or([0.0; 3]);
        let mode = weighting.unwrap_or(Weighting::Covariance);
        let covariance = match mode {
            Weighting::Covariance => {
                if g2.iter().any(|&v| v <= 0.0) {
                    return Err(at(format!(
                        "{label}: G2 variances must be positive, but they're {g2:?}"
                    )));
                }
                if weighting.is_none() && g3.iter().any(|v| v.abs() >= 0.01) {
                    return Err(at(format!(
                        "{label}: its G3 values {g3:?} look like correlations, not covariances. \
                         Add .GPS WEIGHT STDERRCORR to the file if G2 and G3 hold standard \
                         errors and correlations, or .GPS WEIGHT COVARIANCE if they really are \
                         covariances"
                    )));
                }
                let [xx, yy, zz] = g2;
                let [xy, xz, yz] = g3;
                nalgebra::Matrix3::new(xx, xy, xz, xy, yy, yz, xz, yz, zz)
            }
            Weighting::StdErrCorrelation => {
                if g2.iter().any(|&v| v <= 0.0) {
                    return Err(at(format!(
                        "{label}: G2 standard errors must be positive, but they're {g2:?}"
                    )));
                }
                if g3.iter().any(|v| v.abs() > 1.0) {
                    return Err(at(format!(
                        "{label}: G3 correlations must be between -1 and 1, but they're {g3:?}"
                    )));
                }
                let [sx, sy, sz] = g2;
                let [rxy, rxz, ryz] = g3;
                nalgebra::Matrix3::new(
                    sx * sx,
                    rxy * sx * sy,
                    rxz * sx * sz,
                    rxy * sx * sy,
                    sy * sy,
                    ryz * sy * sz,
                    rxz * sx * sz,
                    ryz * sy * sz,
                    sz * sz,
                )
            }
        };
        let (h, v) = factor;
        survey.measurements.push(Measurement {
            kind: MeasurementKind::Baseline,
            elements: vec![Element::Vector {
                from: p.from,
                to: p.to,
                vector: p.vector,
            }],
            covariance: DMatrix::from_column_slice(3, 3, covariance.as_slice()),
            reference,
            scaling: Scaling {
                v: 1.0,
                p: h * h,
                l: h * h,
                h: v * v,
            },
            ignored: false,
            source: None,
            id: p.name,
            origin: Origin {
                file: file.to_owned(),
                line: p.line,
            },
        });
        Ok(())
    };

    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let at = |m: String| FormatError::at(file, line, m);
        // A comment runs from # to the end of the line.
        let content = raw.split('#').next().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }
        let (code, rest) = content
            .split_once(char::is_whitespace)
            .unwrap_or((content, ""));
        let rest = rest.trim();
        let upper = code.to_ascii_uppercase();
        if upper.starts_with('.') {
            let words: Vec<String> = rest
                .split_whitespace()
                .map(str::to_ascii_uppercase)
                .collect();
            if upper == ".GPS" {
                match words.first().map(String::as_str) {
                    Some("WEIGHT") => {
                        weighting = match words.get(1).map(String::as_str) {
                            Some(w) if w.starts_with("COV") => Some(Weighting::Covariance),
                            Some(w) if w.starts_with("STD") => Some(Weighting::StdErrCorrelation),
                            other => {
                                return Err(at(format!(
                                    ".GPS WEIGHT should be COVARIANCE or STDERRCORR, not {}",
                                    other.unwrap_or("nothing")
                                )));
                            }
                        };
                    }
                    Some("FACTOR") => {
                        let values: Vec<f64> = rest
                            .split_whitespace()
                            .skip(1)
                            .filter(|w| !w.eq_ignore_ascii_case("VERT"))
                            .map(|w| number(".GPS FACTOR", w))
                            .collect::<Result<_, _>>()
                            .map_err(at)?;
                        factor = match values[..] {
                            [h] => (h, h),
                            [h, v] => (h, v),
                            _ => {
                                return Err(at(".GPS FACTOR needs one or two numbers".into()));
                            }
                        };
                        if factor.0 <= 0.0 || factor.1 <= 0.0 {
                            return Err(at(".GPS FACTOR must be positive".into()));
                        }
                    }
                    _ => survey.notes.push(Note {
                        origin: Origin {
                            file: file.to_owned(),
                            line,
                        },
                        message: format!("ignored the option {content}"),
                    }),
                }
            }
            continue;
        }
        match upper.as_str() {
            "G0" => {
                if let Some(p) = pending.take() {
                    finish(p, weighting, factor, &mut survey)?;
                }
                name = rest
                    .split_once('\'')
                    .map(|(_, n)| n.trim().to_owned())
                    .filter(|n| !n.is_empty());
            }
            "G1" => {
                if let Some(p) = pending.take() {
                    finish(p, weighting, factor, &mut survey)?;
                }
                // A descriptor may follow an apostrophe.
                let (fields, descriptor) = match rest.split_once('\'') {
                    Some((f, d)) => (f, Some(d.trim().to_owned()).filter(|d| !d.is_empty())),
                    None => (rest, None),
                };
                let words: Vec<&str> = fields.split_whitespace().collect();
                if words.len() > 4 {
                    return Err(at(format!(
                        "G1 has {} values after the stations; antenna heights on G1 aren't read \
                         yet, so give mark-to-mark vectors",
                        words.len() - 1
                    )));
                }
                let [stations, x, y, z] = words[..] else {
                    return Err(at("G1 needs FROM-TO and three components".into()));
                };
                let (from, to) = stations.split_once('-').ok_or_else(|| {
                    at(format!(
                        "G1 should name the stations FROM-TO, not {stations:?}"
                    ))
                })?;
                if from.is_empty() || to.is_empty() {
                    return Err(at(format!(
                        "G1 should name the stations FROM-TO, not {stations:?}"
                    )));
                }
                if from == to {
                    return Err(at(format!("the vector from {from} ends at {from} too")));
                }
                let vector = Vector3::new(
                    number("dX", x).map_err(at)?,
                    number("dY", y).map_err(at)?,
                    number("dZ", z).map_err(at)?,
                );
                pending = Some(Pending {
                    name: name.take().or(descriptor),
                    from: from.to_owned(),
                    to: to.to_owned(),
                    vector,
                    g2: None,
                    g3: None,
                    line,
                });
            }
            "G2" | "G3" => {
                let p = pending
                    .as_mut()
                    .ok_or_else(|| at(format!("{upper} comes before any G1 record")))?;
                let values: Vec<f64> = rest
                    .split_whitespace()
                    .map(|w| number(&upper, w))
                    .collect::<Result<_, _>>()
                    .map_err(at)?;
                let [a, b, c] = values[..] else {
                    return Err(at(format!("{upper} needs three values")));
                };
                if upper == "G2" {
                    p.g2 = Some([a, b, c]);
                } else {
                    p.g3 = Some([a, b, c]);
                }
            }
            "G4" => {
                return Err(at(
                    "G4 offsets aren't read yet, so give vectors without them".into(),
                ));
            }
            _ => {
                let entry = skipped.entry(upper).or_insert((0, line));
                entry.0 += 1;
            }
        }
    }
    if let Some(p) = pending.take() {
        finish(p, weighting, factor, &mut survey)?;
    }
    for (code, (count, line)) in skipped {
        survey.notes.push(Note {
            origin: Origin {
                file: file.to_owned(),
                line,
            },
            message: format!(
                "skipped {count} {code} record{}: only GPS vectors (G records) are read",
                if count == 1 { "" } else { "s" }
            ),
        });
    }
    if survey.measurements.is_empty() {
        return Err(FormatError::in_file(
            file,
            "it has no GPS vectors (G1 records)",
        ));
    }
    Ok(survey)
}

/// Writes GNSS baselines as a STAR*NET GPS vector file, with
/// `.GPS WEIGHT COVARIANCE`.
///
/// Covariances are written as given: variance scaling isn't applied, so pass
/// measurements whose covariances already include it. STAR*NET vectors are
/// independent, so the correlations between a cluster's baselines are
/// dropped; callers should say so. Position elements are left out. Station
/// names can't hold spaces, hyphens, `#` or apostrophes, which STAR*NET's
/// records use as separators.
pub fn write(measurements: &[Measurement]) -> Result<String, WriteError> {
    let mut out = String::from("# GPS vectors written for STAR*NET\n.GPS WEIGHT COVARIANCE\n");
    for m in measurements {
        for (i, element) in m.elements.iter().enumerate() {
            let Element::Vector { from, to, vector } = element else {
                continue;
            };
            for station in [from, to] {
                if station.is_empty()
                    || station
                        .chars()
                        .any(|c| c.is_whitespace() || matches!(c, '-' | '#' | '\''))
                {
                    return Err(WriteError(format!(
                        "station name {station:?} can't be written for STAR*NET, which separates \
                         FROM-TO with a hyphen: rename it"
                    )));
                }
            }
            let c = |r: usize, s: usize| m.covariance[(3 * i + r, 3 * i + s)];
            let name =
                m.id.clone()
                    .unwrap_or_else(|| format!("{}:{}", m.origin.file, m.origin.line));
            let _ = write!(
                out,
                "G0 '{name}\nG1 {from}-{to} {} {} {}\nG2 {:e} {:e} {:e}\nG3 {:e} {:e} {:e}\n",
                vector.x,
                vector.y,
                vector.z,
                c(0, 0),
                c(1, 1),
                c(2, 2),
                c(0, 1),
                c(0, 2),
                c(1, 2)
            );
        }
    }
    Ok(out)
}
