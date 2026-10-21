//! RTKLIB solution files (`.pos`), as rnx2rtkp and RTKPOST write them: one
//! baseline's static solution, from the base position in the header to the
//! rover's solution.
//!
//! All three of RTKLIB's position forms are read: ECEF (`x/y/z-ecef`),
//! latitude, longitude and ellipsoidal height (in degrees, or degrees,
//! minutes and seconds), and east, north and up from the base. RTKLIB writes
//! each covariance term as a signed square root (`sdxy` is the square root of
//! |covariance|, with its sign), which is undone here. It writes standard
//! deviations to 0.1 mm, so precise static baselines' covariances carry that
//! rounding.
//!
//! Export static solutions with `out-solstatic=single`, which gives one line.
//! If a file has more, the last is used, with a note. RTKLIB doesn't record
//! the frame of the orbits it used (broadcast orbits are in WGS 84, precise
//! ones in the IGS frame), so the caller says which; the epoch is the middle
//! of the observations.

use gnss_datum::hifitime::{Epoch, TimeScale};
use gnss_datum::{CoordinateEpoch, FrameId, Geodetic, WGS84};
use nalgebra::{DMatrix, Matrix3, Vector3};

use crate::error::FormatError;
use crate::survey::{
    Constraint, Coordinate, CoordinateValue, Element, Measurement, MeasurementKind, Note, Origin,
    Reference, Scaling, Station, Survey,
};
use crate::text::number;

/// The mark names to use, when the caller knows them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Names {
    /// The base: the reference position in the header.
    pub base: Option<String>,
    /// The rover: the solution.
    pub rover: Option<String>,
}

/// RTKLIB's position forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    Ecef,
    Degrees,
    DegreesMinutesSeconds,
    EastNorthUp,
}

/// RTKLIB's solution quality flag, `Q`.
fn quality(q: u32) -> &'static str {
    match q {
        1 => "fixed",
        2 => "float",
        3 => "SBAS",
        4 => "DGPS",
        5 => "single point",
        6 => "PPP",
        _ => "unknown",
    }
}

/// Undoes RTKLIB's signed square root of a covariance term.
fn signed_square(v: f64) -> f64 {
    if v < 0.0 { -v * v } else { v * v }
}

/// A mark name from an observation file's name: a RINEX 2 short name's or
/// RINEX 3 long name's four-character site, upper case, or otherwise the
/// name without its extensions. `None` for files that hold no observations,
/// such as navigation data.
pub fn mark_from_file(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let mut stem = name;
    for compression in [".gz", ".Z", ".zip", ".bz2"] {
        stem = stem.strip_suffix(compression).unwrap_or(stem);
    }
    let lower = stem.to_ascii_lowercase();
    let site = |s: &str| s.chars().take(4).collect::<String>().to_ascii_uppercase();
    // RINEX 2 short names: ssssdddf.yyt, observations t = o or d (Hatanaka).
    if let Some((base, ext)) = lower.rsplit_once('.') {
        let ext = ext.as_bytes();
        if base.len() == 8 && ext.len() == 3 && ext[..2].iter().all(u8::is_ascii_digit) {
            return matches!(ext[2], b'o' | b'd').then(|| site(stem));
        }
        // RINEX 3 and 4 long names: the content type is the last part, MO for
        // mixed observations, GO, RO and so on; MN and friends are navigation.
        if (ext == b"rnx" || ext == b"crx") && base.contains('_') {
            return base
                .rsplit('_')
                .next()
                .is_some_and(|t| t.ends_with('o'))
                .then(|| site(stem));
        }
        if base.contains("nav") || base.contains("brdc") {
            return None;
        }
        if matches!(
            ext,
            b"nav" | b"sp3" | b"clk" | b"atx" | b"erp" | b"ionex" | b"conf" | b"pos" | b"sbs"
        ) {
            return None;
        }
        return Some(stem[..base.len()].to_owned());
    }
    Some(stem.to_owned())
}

fn time(field: &str, date: &str, clock: &str, scale: TimeScale) -> Result<Epoch, String> {
    let bad = || format!("{field} isn't a time RTKLIB writes: {date} {clock}");
    // Either yyyy/mm/dd hh:mm:ss.sss or GPS week and seconds of week.
    if let Some((y, rest)) = date.split_once('/') {
        let (m, d) = rest.split_once('/').ok_or_else(bad)?;
        let mut hms = clock.splitn(3, ':');
        let (h, mi, s) = (hms.next(), hms.next(), hms.next());
        let (Some(h), Some(mi), Some(s)) = (h, mi, s) else {
            return Err(bad());
        };
        let seconds: f64 = s.parse().map_err(|_| bad())?;
        if !(0.0..61.0).contains(&seconds) {
            return Err(bad());
        }
        let whole = seconds.floor();
        let nanos = ((seconds - whole) * 1e9).round() as u32;
        Epoch::maybe_from_gregorian(
            y.parse().map_err(|_| bad())?,
            m.parse().map_err(|_| bad())?,
            d.parse().map_err(|_| bad())?,
            h.parse().map_err(|_| bad())?,
            mi.parse().map_err(|_| bad())?,
            whole as u8,
            nanos.min(999_999_999),
            scale,
        )
        .map_err(|_| bad())
    } else {
        let week: u32 = date.parse().map_err(|_| bad())?;
        let seconds: f64 = clock.parse().map_err(|_| bad())?;
        if !(0.0..=604_800.0).contains(&seconds) || week > 10_000 {
            return Err(bad());
        }
        let nanos = (seconds * 1e9).round() as u64;
        Ok(Epoch::from_time_of_week(week, nanos, TimeScale::GPST))
    }
}

/// Reads an RTKLIB solution file as one baseline from the base to the rover.
///
/// `frame` is the frame of the orbits used; without one, WGS 84 is assumed,
/// with a note. `names` overrides the mark names, which are otherwise taken
/// from the observation files the header lists: as RTKLIB reads them, the
/// first is the rover's and the second the base's.
pub fn read(
    file: &str,
    text: &str,
    frame: Option<FrameId>,
    names: &Names,
) -> Result<Survey, FormatError> {
    let mut notes = Vec::new();
    let note = |notes: &mut Vec<Note>, line: usize, message: String| {
        notes.push(Note {
            origin: Origin {
                file: file.to_owned(),
                line,
            },
            message,
        });
    };
    let mut inputs: Vec<String> = Vec::new();
    let mut start: Option<Epoch> = None;
    let mut end: Option<Epoch> = None;
    let mut reference_text: Option<(usize, String)> = None;
    let mut form: Option<Form> = None;
    let mut scale = TimeScale::GPST;
    let mut ellipsoidal = true;
    let mut static_mode = true;
    let mut solutions: Vec<(usize, Vec<String>)> = Vec::new();

    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let at = |m: String| FormatError::at(file, line, m);
        let content = raw.trim();
        if content.is_empty() {
            continue;
        }
        if let Some(header) = content.strip_prefix('%') {
            let header = header.trim();
            // The legend, such as (lat/lon/height=WGS84/ellipsoidal,Q=1:fix,...).
            if header.starts_with('(') {
                if header.contains("/geodetic") {
                    ellipsoidal = false;
                }
            } else if let Some((key, value)) = header.split_once(':') {
                let value = value.trim();
                match key.trim() {
                    "inp file" => inputs.push(value.to_owned()),
                    "obs start" | "obs end" => {
                        let words: Vec<&str> = value.split_whitespace().collect();
                        if let [date, clock, system, ..] = words[..] {
                            let s = match system {
                                "UTC" => TimeScale::UTC,
                                _ => TimeScale::GPST,
                            };
                            let t = time(key.trim(), date, clock, s).map_err(at)?;
                            if key.trim() == "obs start" {
                                start = Some(t);
                            } else {
                                end = Some(t);
                            }
                        }
                    }
                    "ref pos" => reference_text = Some((line, value.to_owned())),
                    "pos mode" => static_mode = value.eq_ignore_ascii_case("static"),
                    _ => {}
                }
            } else {
                let first = header.split_whitespace().next().unwrap_or("");
                if matches!(first, "GPST" | "UTC" | "JST") {
                    scale = match first {
                        "UTC" => TimeScale::UTC,
                        "GPST" => TimeScale::GPST,
                        _ => {
                            return Err(at("times in JST aren't read: write GPST or UTC".into()));
                        }
                    };
                    form = Some(if header.contains("x-ecef") {
                        Form::Ecef
                    } else if header.contains("e-baseline") {
                        Form::EastNorthUp
                    } else if header.contains("latitude(d'") {
                        Form::DegreesMinutesSeconds
                    } else if header.contains("latitude(deg)") {
                        Form::Degrees
                    } else {
                        return Err(at(format!(
                            "the column heading {header:?} isn't one RTKLIB writes for \
                             positions; NMEA and the other forms aren't read"
                        )));
                    });
                }
            }
            continue;
        }
        let fields: Vec<String> = content
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|f| !f.is_empty())
            .map(str::to_owned)
            .collect();
        solutions.push((line, fields));
    }

    let form = form.ok_or_else(|| {
        FormatError::in_file(
            file,
            "it has no RTKLIB column heading (a % line starting GPST or UTC): write the solution \
             with out-outhead=on",
        )
    })?;
    if !ellipsoidal {
        return Err(FormatError::in_file(
            file,
            "its heights are geodetic (above a geoid): write ellipsoidal heights \
             (out-height=ellipsoidal)",
        ));
    }
    let (line, fields) = solutions
        .last()
        .ok_or_else(|| FormatError::in_file(file, "it has no solution"))?;
    let line = *line;
    let at = |m: String| FormatError::at(file, line, m);
    if solutions.len() > 1 {
        note(
            &mut notes,
            line,
            format!(
                "it has {} solutions; the last was used. For a static baseline, write only the \
                 final solution with out-solstatic=single",
                solutions.len()
            ),
        );
    }
    if !static_mode {
        note(
            &mut notes,
            line,
            "RTKLIB was in a kinematic mode, so this is one epoch's position, not a static \
             solution"
                .into(),
        );
    }

    // Time, then the position, Q, ns and six signed standard deviations.
    let angles = if form == Form::DegreesMinutesSeconds {
        6
    } else {
        2
    };
    let needed = 2 + angles + 1 + 2 + 6;
    if fields.len() < needed {
        return Err(at(format!(
            "a solution needs at least {needed} values, but this has {}",
            fields.len()
        )));
    }
    let solved = time("the solution time", &fields[0], &fields[1], scale).map_err(at)?;
    let values: Vec<f64> = fields[2..needed]
        .iter()
        .map(|f| number("a solution value", f))
        .collect::<Result<_, _>>()
        .map_err(at)?;
    let (position, rest) = values.split_at(angles + 1);
    let q = rest[0];
    let q = if q.fract() == 0.0 && (0.0..100.0).contains(&q) {
        q as u32
    } else {
        return Err(at(format!("Q should be a solution flag, not {q}")));
    };
    let sd = &rest[2..8];
    if !matches!(q, 1 | 2 | 4) {
        return Err(at(format!(
            "the solution is {} (Q={q}), not a baseline from the base: process it in a \
             relative mode",
            quality(q)
        )));
    }
    if q != 1 {
        note(
            &mut notes,
            line,
            format!("the solution is {} (Q={q}), not fixed", quality(q)),
        );
    }

    // The base, from the header.
    let (reference_line, reference_text) = reference_text.ok_or_else(|| {
        FormatError::in_file(
            file,
            "it has no ref pos line, so the base's position is unknown",
        )
    })?;
    let at_reference = |m: String| FormatError::at(file, reference_line, m);
    let reference_values: Vec<f64> = reference_text
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|f| !f.is_empty())
        .map(|f| number("ref pos", f))
        .collect::<Result<_, _>>()
        .map_err(at_reference)?;
    let geodetic_to_ecef = |latitude_deg: f64, longitude_deg: f64, height_m: f64| {
        Geodetic::from_degrees(latitude_deg, longitude_deg, height_m)
            .map(|g| WGS84.geodetic_to_ecef(&g))
            .map_err(|e| e.to_string())
    };
    let dms = |v: &[f64]| {
        let sign = if v[0] < 0.0 || (v[0] == 0.0 && (v[1] < 0.0 || v[2] < 0.0)) {
            -1.0
        } else {
            1.0
        };
        sign * (v[0].abs() + v[1].abs() / 60.0 + v[2].abs() / 3600.0)
    };
    let base = match (form, &reference_values[..]) {
        (Form::Ecef, [x, y, z]) => Vector3::new(*x, *y, *z),
        (Form::Degrees | Form::EastNorthUp, [lat, lon, h]) => {
            geodetic_to_ecef(*lat, *lon, *h).map_err(at_reference)?
        }
        (_, v) if v.len() == 7 => {
            geodetic_to_ecef(dms(&v[0..3]), dms(&v[3..6]), v[6]).map_err(at_reference)?
        }
        _ => {
            return Err(at_reference(format!(
                "ref pos has {} values, which doesn't fit the solution's form",
                reference_values.len()
            )));
        }
    };
    let rotation_at = |x: &Vector3<f64>| -> Result<Matrix3<f64>, FormatError> {
        WGS84
            .ecef_to_geodetic(x)
            .map(|g| g.enu_rotation())
            .map_err(|e| at(e.to_string()))
    };

    // The vector and its covariance, in ECEF.
    let (vector, covariance) = match form {
        Form::Ecef => {
            let rover = Vector3::new(position[0], position[1], position[2]);
            let [sx, sy, sz, sxy, syz, szx] = [sd[0], sd[1], sd[2], sd[3], sd[4], sd[5]];
            let (xy, yz, zx) = (signed_square(sxy), signed_square(syz), signed_square(szx));
            let c = Matrix3::new(sx * sx, xy, zx, xy, sy * sy, yz, zx, yz, sz * sz);
            (rover - base, c)
        }
        Form::Degrees | Form::DegreesMinutesSeconds => {
            let (lat, lon, h) = if form == Form::Degrees {
                (position[0], position[1], position[2])
            } else {
                (dms(&position[0..3]), dms(&position[3..6]), position[6])
            };
            let rover = geodetic_to_ecef(lat, lon, h).map_err(at)?;
            // sdn sde sdu sdne sdeu sdun, at the rover.
            let [sn, se, su, sne, seu, sun] = [sd[0], sd[1], sd[2], sd[3], sd[4], sd[5]];
            let (ne, eu, un) = (signed_square(sne), signed_square(seu), signed_square(sun));
            let enu = Matrix3::new(se * se, ne, eu, ne, sn * sn, un, eu, un, su * su);
            let r = rotation_at(&rover)?;
            (rover - base, r.transpose() * enu * r)
        }
        Form::EastNorthUp => {
            // sde sdn sdu sden sdnu sdue, at the base.
            let [se, sn, su, sen, snu, sue] = [sd[0], sd[1], sd[2], sd[3], sd[4], sd[5]];
            let (en, nu, ue) = (signed_square(sen), signed_square(snu), signed_square(sue));
            let enu = Matrix3::new(se * se, en, ue, en, sn * sn, nu, ue, nu, su * su);
            let r = rotation_at(&base)?;
            let local = Vector3::new(position[0], position[1], position[2]);
            (r.transpose() * local, r.transpose() * enu * r)
        }
    };

    // Names, frame and epoch.
    let observations: Vec<String> = inputs.iter().filter_map(|i| mark_from_file(i)).collect();
    let rover = names
        .rover
        .clone()
        .or_else(|| observations.first().cloned())
        .ok_or_else(|| {
            FormatError::in_file(
                file,
                "the header lists no observation files: name the rover",
            )
        })?;
    let base_name = names
        .base
        .clone()
        .or_else(|| observations.get(1).cloned())
        .ok_or_else(|| {
            FormatError::in_file(file, "the header lists one observation file: name the base")
        })?;
    if rover == base_name {
        return Err(FormatError::in_file(
            file,
            format!("the base and the rover are both called {rover}: name them"),
        ));
    }
    if names.rover.is_none() || names.base.is_none() {
        note(
            &mut notes,
            1,
            format!(
                "the base is taken as {base_name} and the rover as {rover}, from the file names"
            ),
        );
    }
    let frame = frame.unwrap_or_else(|| {
        note(
            &mut notes,
            1,
            "RTKLIB doesn't record the orbits' frame; WGS 84 was assumed, as for broadcast \
             orbits"
                .into(),
        );
        FrameId::Wgs84
    });
    let middle = match (start, end) {
        (Some(s), Some(e)) => s + (e - s) * 0.5,
        _ => solved,
    };
    let epoch = CoordinateEpoch::from_time(middle).map_err(|e| at(e.to_string()))?;
    let reference = Reference {
        frame,
        epoch: (!frame.is_static()).then_some(epoch),
    };
    let origin = Origin {
        file: file.to_owned(),
        line,
    };
    Ok(Survey {
        stations: vec![Station {
            name: base_name.clone(),
            description: None,
            coordinate: Some(Coordinate {
                value: CoordinateValue::Cartesian(base),
                reference,
            }),
            constraint: Constraint::Free,
            origin: Origin {
                file: file.to_owned(),
                line: reference_line,
            },
        }],
        measurements: vec![Measurement {
            kind: MeasurementKind::Baseline,
            elements: vec![Element::Vector {
                from: base_name,
                to: rover,
                vector,
            }],
            covariance: DMatrix::from_column_slice(3, 3, covariance.as_slice()),
            reference,
            scaling: Scaling::NONE,
            ignored: false,
            source: None,
            id: None,
            origin,
        }],
        notes,
    })
}
