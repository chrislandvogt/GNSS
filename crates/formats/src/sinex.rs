//! SINEX solutions: station positions with their full covariance, such as
//! AUSPOS's or an IGS weekly solution. They become weighted control: one
//! point cluster holding every site's position, with the correlations between
//! sites.
//!
//! Read: `SITE/ID` (descriptions), `SOLUTION/ESTIMATE` (STAX, STAY and STAZ;
//! velocities and other parameters are noted and skipped), and
//! `SOLUTION/MATRIX_ESTIMATE` as a covariance (`COVA`), correlations with
//! standard deviations on the diagonal (`CORR`), or an information matrix
//! (`INFO`), which is inverted, lower or upper triangle.
//!
//! SINEX has no field for the reference frame, so it's taken from the
//! caller, or else found in the file's comments (AUSPOS, for example, names
//! it); a file that names more than one isn't guessed at. Every site must
//! share one reference epoch.

use std::collections::BTreeMap;

use gnss_datum::hifitime::{Duration, Epoch, TimeScale};
use gnss_datum::{CoordinateEpoch, FrameId};
use nalgebra::{DMatrix, Vector3};

use crate::error::FormatError;
use crate::survey::{
    Constraint, Coordinate, CoordinateValue, Element, Measurement, MeasurementKind, Note, Origin,
    Reference, Scaling, Station, Survey,
};
use crate::text::number;

/// Names of frames as SINEX files write them, IGS realisations included.
const FRAMES: [(&str, FrameId); 13] = [
    ("IGS20", FrameId::Igs20),
    ("IGB20", FrameId::Igs20),
    ("ITRF2020", FrameId::Itrf2020),
    ("ITRF20", FrameId::Itrf2020),
    ("IGB14", FrameId::Itrf2014),
    ("IGS14", FrameId::Itrf2014),
    ("ITRF2014", FrameId::Itrf2014),
    ("IGB08", FrameId::Itrf2008),
    ("IGS08", FrameId::Itrf2008),
    ("ITRF2008", FrameId::Itrf2008),
    ("IGS05", FrameId::Itrf2005),
    ("ITRF2005", FrameId::Itrf2005),
    ("GDA2020", FrameId::Gda2020),
];

/// A SINEX time, `YY:DDD:SSSSS` or `YYYY:DDD:SSSSS`, in UTC.
fn sinex_time(text: &str) -> Option<Epoch> {
    let mut parts = text.split(':');
    let (y, d, s) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let year: i32 = y.parse().ok()?;
    let year = match y.len() {
        2 if year < 50 => 2000 + year,
        2 => 1900 + year,
        4 => year,
        _ => return None,
    };
    let day: u32 = d.parse().ok()?;
    let seconds: u32 = s.parse().ok()?;
    if !(1..=366).contains(&day) || seconds > 86_400 {
        return None;
    }
    let start = Epoch::maybe_from_gregorian(year, 1, 1, 0, 0, 0, 0, TimeScale::UTC).ok()?;
    let day_start = start + Duration::from_days(f64::from(day - 1));
    // Day 366 only exists in leap years.
    if day_start.to_gregorian(TimeScale::UTC).0 != year {
        return None;
    }
    Some(day_start + Duration::from_seconds(f64::from(seconds)))
}

/// A site's position estimate.
struct Site {
    code: String,
    solution: String,
    epoch_text: String,
    indices: [Option<usize>; 3],
    values: [f64; 3],
    line: usize,
}

/// Reads a SINEX file's station positions and their covariance. `frame`
/// gives the frame when the file doesn't name it in its comments, and
/// overrides what the comments say.
pub fn read(file: &str, text: &str, frame: Option<FrameId>) -> Result<Survey, FormatError> {
    let mut notes = Vec::new();
    if !text.trim_start().starts_with("%=SNX") {
        return Err(FormatError::at(
            file,
            1,
            "it isn't a SINEX file: the first line should start %=SNX",
        ));
    }
    let mut block: Option<String> = None;
    let mut descriptions: BTreeMap<String, String> = BTreeMap::new();
    let mut named_frames: Vec<FrameId> = Vec::new();
    let mut sites: Vec<Site> = Vec::new();
    let mut skipped: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut parameters = 0usize;
    // (row, column, value) from the matrix, 1-based, and its kind.
    let mut entries: Vec<(usize, usize, f64)> = Vec::new();
    let mut matrix: Option<(String, usize)> = None;

    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let at = |m: String| FormatError::at(file, line, m);
        if raw.trim().is_empty() {
            continue;
        }
        let commentary = matches!(
            block.as_deref().and_then(|b| b.split_whitespace().next()),
            Some("FILE/COMMENT" | "FILE/REFERENCE" | "INPUT/ACKNOWLEDGEMENTS" | "INPUT/HISTORY")
        );
        // Comment lines, which in the commentary blocks can name the frame.
        if raw.starts_with('*') && !commentary {
            continue;
        }
        if let Some(name) = raw.strip_prefix('+') {
            block = Some(name.trim().to_owned());
            let words: Vec<&str> = name.split_whitespace().collect();
            if words.first() == Some(&"SOLUTION/MATRIX_ESTIMATE") {
                let shape = words.get(1).copied().unwrap_or("");
                let kind = words.get(2).copied().unwrap_or("");
                if !matches!(shape, "L" | "U") || !matches!(kind, "COVA" | "CORR" | "INFO") {
                    return Err(at(format!(
                        "SOLUTION/MATRIX_ESTIMATE {shape} {kind} isn't a matrix form SINEX \
                         defines (L or U, then COVA, CORR or INFO)"
                    )));
                }
                matrix = Some((kind.to_owned(), line));
            }
            continue;
        }
        if raw.starts_with('-') {
            block = None;
            continue;
        }
        if raw.starts_with('%') {
            continue;
        }
        let Some(current) = block.as_deref() else {
            continue;
        };
        let upper = raw.to_ascii_uppercase();
        match current.split_whitespace().next().unwrap_or("") {
            "FILE/COMMENT" | "FILE/REFERENCE" | "INPUT/ACKNOWLEDGEMENTS" | "INPUT/HISTORY" => {
                for (name, id) in FRAMES {
                    let found = upper
                        .split(|c: char| !c.is_ascii_alphanumeric())
                        .any(|w| w == name);
                    if found && !named_frames.contains(&id) {
                        named_frames.push(id);
                    }
                }
            }
            "SITE/ID" => {
                let code = raw.get(1..5).unwrap_or("").trim().to_owned();
                let description = raw.get(21..43).unwrap_or("").trim().to_owned();
                if !code.is_empty() && !description.is_empty() {
                    descriptions.insert(code, description);
                }
            }
            "SOLUTION/ESTIMATE" => {
                let words: Vec<&str> = raw.split_whitespace().collect();
                let [
                    index,
                    kind,
                    code,
                    _point,
                    solution,
                    epoch,
                    _unit,
                    _s,
                    value,
                    ..,
                ] = words[..]
                else {
                    return Err(at(format!(
                        "a SOLUTION/ESTIMATE line needs ten fields, but this has {}",
                        words.len()
                    )));
                };
                let index: usize = index.parse().ok().filter(|&i| i > 0).ok_or_else(|| {
                    at(format!(
                        "the parameter index {index:?} isn't a count from 1"
                    ))
                })?;
                parameters = parameters.max(index);
                let axis = match kind {
                    "STAX" => 0,
                    "STAY" => 1,
                    "STAZ" => 2,
                    other => {
                        let entry = skipped.entry(other.to_owned()).or_insert((0, line));
                        entry.0 += 1;
                        continue;
                    }
                };
                let value = number(kind, value).map_err(at)?;
                let site = match sites
                    .iter_mut()
                    .find(|s| s.code == code && s.solution == solution)
                {
                    Some(site) => site,
                    None => {
                        sites.push(Site {
                            code: code.to_owned(),
                            solution: solution.to_owned(),
                            epoch_text: epoch.to_owned(),
                            indices: [None; 3],
                            values: [0.0; 3],
                            line,
                        });
                        let last = sites.len() - 1;
                        &mut sites[last]
                    }
                };
                if site.epoch_text != epoch {
                    return Err(at(format!(
                        "site {code}'s X, Y and Z have different reference epochs"
                    )));
                }
                if site.indices[axis].replace(index).is_some() {
                    return Err(at(format!("site {code} has {kind} twice")));
                }
                site.values[axis] = value;
            }
            "SOLUTION/MATRIX_ESTIMATE" => {
                let words: Vec<&str> = raw.split_whitespace().collect();
                if words.len() < 3 || words.len() > 5 {
                    return Err(at(
                        "a matrix line needs two indices and one to three values".into(),
                    ));
                }
                let row: usize = words[0]
                    .parse()
                    .map_err(|_| at(format!("the row {:?} isn't a number", words[0])))?;
                let column: usize = words[1]
                    .parse()
                    .map_err(|_| at(format!("the column {:?} isn't a number", words[1])))?;
                for (k, w) in words[2..].iter().enumerate() {
                    entries.push((row, column + k, number("a matrix value", w).map_err(at)?));
                }
            }
            _ => {}
        }
    }

    if sites.is_empty() {
        return Err(FormatError::in_file(
            file,
            "it has no station positions (STAX, STAY and STAZ in SOLUTION/ESTIMATE)",
        ));
    }
    // One cluster holds every site's correlations, so its size is limited.
    const MOST_SITES: usize = 1000;
    if sites.len() > MOST_SITES {
        return Err(FormatError::in_file(
            file,
            format!(
                "it has {} sites; solutions of more than {MOST_SITES} aren't read as one cluster",
                sites.len()
            ),
        ));
    }
    for site in &sites {
        if site.indices.iter().any(Option::is_none) {
            return Err(FormatError::at(
                file,
                site.line,
                format!("site {} doesn't have all of STAX, STAY and STAZ", site.code),
            ));
        }
    }
    let first_epoch = &sites[0].epoch_text;
    if let Some(other) = sites.iter().find(|s| &s.epoch_text != first_epoch) {
        return Err(FormatError::at(
            file,
            other.line,
            format!(
                "site {} is at {}, but site {} is at {first_epoch}: the sites must share one \
                 reference epoch",
                other.code, other.epoch_text, sites[0].code
            ),
        ));
    }
    let epoch = sinex_time(first_epoch)
        .and_then(|t| CoordinateEpoch::from_time(t).ok())
        .ok_or_else(|| {
            FormatError::at(
                file,
                sites[0].line,
                format!("the reference epoch {first_epoch:?} isn't a SINEX time (YY:DDD:SSSSS)"),
            )
        })?;
    let frame = match (frame, &named_frames[..]) {
        (Some(f), _) => f,
        (None, [only]) => {
            notes.push(Note {
                origin: Origin {
                    file: file.to_owned(),
                    line: 1,
                },
                message: format!("the frame was taken as {only}, which the file's comments name"),
            });
            *only
        }
        (None, []) => {
            return Err(FormatError::in_file(
                file,
                "SINEX doesn't record the frame and the file's comments don't name one: say \
                 which frame the solution is in",
            ));
        }
        (None, several) => {
            return Err(FormatError::in_file(
                file,
                format!(
                    "the file's comments name {}: say which frame the solution is in",
                    several
                        .iter()
                        .map(|f| f.name())
                        .collect::<Vec<_>>()
                        .join(" and ")
                ),
            ));
        }
    };

    // The sites' rows and columns of the matrix. Covariances and
    // correlations are read for those alone; an information matrix has to be
    // inverted whole, so it's limited in size.
    let (kind, matrix_line) = matrix.ok_or_else(|| {
        FormatError::in_file(
            file,
            "it has no SOLUTION/MATRIX_ESTIMATE, so the positions have no covariance",
        )
    })?;
    let needed: Vec<usize> = sites
        .iter()
        .flat_map(|s| s.indices.iter().map(|i| i.unwrap_or(1)))
        .collect();
    let position: BTreeMap<usize, usize> =
        needed.iter().enumerate().map(|(k, &i)| (i, k)).collect();
    let n = needed.len();
    let outside = |row: usize, column: usize| {
        FormatError::at(
            file,
            matrix_line,
            format!("matrix element ({row}, {column}) is outside the {parameters} parameters"),
        )
    };
    for &(row, column, _) in &entries {
        if row == 0 || column == 0 || row > parameters || column > parameters {
            return Err(outside(row, column));
        }
    }
    let mut covariance = DMatrix::<f64>::zeros(n, n);
    match kind.as_str() {
        "COVA" | "CORR" => {
            for &(row, column, value) in &entries {
                if let (Some(&r), Some(&c)) = (position.get(&row), position.get(&column)) {
                    covariance[(r, c)] = value;
                    covariance[(c, r)] = value;
                }
            }
            if kind == "CORR" {
                let sigmas: Vec<f64> = (0..n).map(|i| covariance[(i, i)]).collect();
                for r in 0..n {
                    for c in 0..n {
                        covariance[(r, c)] = if r == c {
                            sigmas[r] * sigmas[r]
                        } else {
                            covariance[(r, c)] * sigmas[r] * sigmas[c]
                        };
                    }
                }
            }
        }
        _ => {
            const LARGEST: usize = 3000;
            if parameters > LARGEST {
                return Err(FormatError::at(
                    file,
                    matrix_line,
                    format!(
                        "the information matrix has {parameters} parameters; ones over {LARGEST} \
                         aren't inverted. Export the covariance (COVA) instead"
                    ),
                ));
            }
            let mut full = DMatrix::<f64>::zeros(parameters, parameters);
            for &(row, column, value) in &entries {
                full[(row - 1, column - 1)] = value;
                full[(column - 1, row - 1)] = value;
            }
            let inverse = full.cholesky().map(|c| c.inverse()).ok_or_else(|| {
                FormatError::at(
                    file,
                    matrix_line,
                    "the information matrix isn't positive definite, so it has no covariance",
                )
            })?;
            for (r, &i) in needed.iter().enumerate() {
                for (c, &j) in needed.iter().enumerate() {
                    covariance[(r, c)] = inverse[(i - 1, j - 1)];
                }
            }
        }
    }
    if (0..n).any(|i| covariance[(i, i)] <= 0.0 || !covariance[(i, i)].is_finite()) {
        return Err(FormatError::at(
            file,
            matrix_line,
            "a station coordinate has no variance in the matrix",
        ));
    }

    let reference = Reference {
        frame,
        epoch: (!frame.is_static()).then_some(epoch),
    };
    let repeated = |code: &str| sites.iter().filter(|s| s.code == code).count() > 1;
    let name = |s: &Site| {
        if repeated(&s.code) {
            format!("{}_{}", s.code, s.solution)
        } else {
            s.code.clone()
        }
    };
    let stations = sites
        .iter()
        .map(|s| Station {
            name: name(s),
            description: descriptions.get(&s.code).cloned(),
            coordinate: Some(Coordinate {
                value: CoordinateValue::Cartesian(Vector3::from(s.values)),
                reference,
            }),
            constraint: Constraint::Free,
            origin: Origin {
                file: file.to_owned(),
                line: s.line,
            },
        })
        .collect();
    let elements = sites
        .iter()
        .map(|s| Element::Position {
            station: name(s),
            position: Vector3::from(s.values),
        })
        .collect();
    for (kind, (count, line)) in skipped {
        notes.push(Note {
            origin: Origin {
                file: file.to_owned(),
                line,
            },
            message: format!(
                "skipped {count} {kind} parameter{}: only station positions are read",
                if count == 1 { "" } else { "s" }
            ),
        });
    }
    Ok(Survey {
        stations,
        measurements: vec![Measurement {
            kind: MeasurementKind::PointCluster,
            elements,
            covariance,
            reference,
            scaling: Scaling::NONE,
            ignored: false,
            source: None,
            id: None,
            origin: Origin {
                file: file.to_owned(),
                line: sites[0].line,
            },
        }],
        notes,
    })
}
