//! From imported files to a network ready to adjust: marks placed in the
//! project's frame and epoch, measurements transformed into it, DynAdjust's
//! variance scaling applied, and the datum chosen.
//!
//! Vectors are transformed the way DynAdjust's `dnareftran` does it: both
//! ends are transformed, starting from the approximate position of the first
//! mark, and differenced. Covariances go through the transformation's
//! Jacobian there, which DynAdjust leaves out (it's a rotation of under a
//! microradian).

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use gnss_adjust::{Action, Datum, Edit, Network, Observation};
use gnss_datum::tm::MgaZone;
use gnss_datum::{CoordinateEpoch, FrameId, Geodetic, Position, TransformationGraph};
use gnss_formats::{
    Constraint, Coordinate, CoordinateValue, Element, HeightType, Measurement, MeasurementKind,
    Note, Origin, Reference, Scaling, Station, Survey,
};
use nalgebra::{DMatrix, Matrix3, Vector3};

/// The step for differentiating a transformation, in metres. Transformations
/// are linear or nearly so over this distance, and it's long enough that
/// rounding in ECEF coordinates (about 1e-9 m) doesn't matter.
const STEP_M: f64 = 100.0;

/// How the datum is defined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatumChoice {
    /// By the control imported: stations held fixed, and observed positions
    /// with their covariances.
    Constrained,
    /// Minimally constrained, by inner constraints, to test the vectors on
    /// their own. Observed positions are left out and fixed stations are free.
    Minimal,
    /// Minimally constrained by holding one mark at its imported coordinate.
    FixedMark(String),
}

/// What to build.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// The frame to adjust in.
    pub frame: FrameId,
    /// The coordinate epoch, for a dynamic frame.
    pub epoch: Option<CoordinateEpoch>,
    /// How the datum is defined.
    pub datum: DatumChoice,
}

/// A network built from imports, with what was done to get there.
#[derive(Debug, Clone)]
pub struct Built {
    /// The network.
    pub network: Network,
    /// Its datum.
    pub datum: Datum,
    /// Exclusions of the measurements the files mark to be ignored.
    pub edits: Vec<Edit>,
    /// For each observation in the network, the measurement it came from.
    pub sources: Vec<Source>,
    /// How measurements were transformed into the network's frame and epoch.
    pub transformations: Vec<Transformation>,
    /// What was assumed or left out, including the readers' notes.
    pub notes: Vec<Note>,
}

/// Where an observation came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    /// The file and line.
    pub origin: Origin,
    /// A short label, such as "A to B".
    pub label: String,
    /// What the file called it.
    pub kind: MeasurementKind,
    /// The frame and epoch it was in.
    pub reference: Reference,
    /// The variance scaling applied.
    pub scaling: Scaling,
}

/// The transformation applied to the measurements in one frame and epoch.
#[derive(Debug, Clone, PartialEq)]
pub struct Transformation {
    /// The frame and epoch they were in.
    pub from: Reference,
    /// How many measurements.
    pub measurements: usize,
    /// The steps, described.
    pub steps: Vec<String>,
}

/// A problem building the network, and where it is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}{message}", .origin.as_ref().map(|o| format!("{o}: ")).unwrap_or_default())]
pub struct BuildError {
    /// The file and line, if it's about one.
    pub origin: Option<Origin>,
    /// What's wrong.
    pub message: String,
}

fn fail(origin: Option<&Origin>, message: impl Into<String>) -> BuildError {
    BuildError {
        origin: origin.cloned(),
        message: message.into(),
    }
}

fn note(origin: &Origin, message: String) -> Note {
    Note {
        origin: origin.clone(),
        message,
    }
}

/// Up to ten names, then how many more.
fn some_names<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    let all: Vec<&str> = names.into_iter().collect();
    let shown = all.iter().take(10).copied().collect::<Vec<_>>().join(", ");
    match all.len() {
        n if n > 10 => format!("{shown} and {} more", n - 10),
        _ => shown,
    }
}

/// Builds a network from what was imported. `graph` holds the
/// transformations available, such as [`TransformationGraph::builtin`] or one
/// that prefers a grid.
pub fn build(
    survey: &Survey,
    settings: &Settings,
    graph: &TransformationGraph,
) -> Result<Built, BuildError> {
    let target = Reference {
        frame: settings.frame,
        epoch: settings.epoch,
    };
    let mut network =
        Network::new(settings.frame, settings.epoch).map_err(|e| fail(None, e.to_string()))?;
    let mut notes = survey.notes.clone();
    let minimal = settings.datum != DatumChoice::Constrained;

    let mut measurements: Vec<Cow<'_, Measurement>> = Vec::new();
    let mut left_out: Vec<&Measurement> = Vec::new();
    // Marks the vectors reach. Observed positions of other marks, such as
    // the CORS an AUSPOS solution includes, tell nothing about the network:
    // they're marginalised out, which for a Gaussian is dropping their rows
    // and columns.
    let mut reached: BTreeSet<&str> = BTreeSet::new();
    for m in &survey.measurements {
        for e in &m.elements {
            if let Element::Vector { from, to, .. } = e {
                reached.insert(from);
                reached.insert(to);
            }
        }
    }
    let mut unreached: Vec<&str> = Vec::new();
    for m in &survey.measurements {
        if minimal && m.kind == MeasurementKind::PointCluster {
            left_out.push(m);
            continue;
        }
        let keep: Vec<usize> = m
            .elements
            .iter()
            .enumerate()
            .filter(|(_, e)| match e {
                Element::Position { station, .. } => {
                    reached.is_empty() || reached.contains(station.as_str())
                }
                Element::Vector { .. } => true,
            })
            .map(|(i, _)| i)
            .collect();
        if keep.len() == m.elements.len() {
            measurements.push(Cow::Borrowed(m));
            continue;
        }
        for (i, e) in m.elements.iter().enumerate() {
            if let (false, Element::Position { station, .. }) = (keep.contains(&i), e) {
                unreached.push(station);
            }
        }
        if keep.is_empty() {
            continue;
        }
        let rows: Vec<usize> = keep.iter().flat_map(|&i| 3 * i..3 * i + 3).collect();
        let mut reduced = m.clone();
        reduced.elements = keep.iter().map(|&i| m.elements[i].clone()).collect();
        reduced.covariance = DMatrix::from_fn(rows.len(), rows.len(), |r, c| {
            m.covariance[(rows[r], rows[c])]
        });
        measurements.push(Cow::Owned(reduced));
    }
    if let Some(first) = survey
        .measurements
        .iter()
        .find(|m| m.kind == MeasurementKind::PointCluster)
        && !unreached.is_empty()
    {
        notes.push(note(
            &first.origin,
            format!(
                "observed positions of {} mark{} no vector reaches were left out: {}",
                unreached.len(),
                if unreached.len() == 1 { "" } else { "s" },
                some_names(unreached.iter().copied())
            ),
        ));
    }
    if let Some(first) = left_out.first() {
        notes.push(note(
            &first.origin,
            format!(
                "{} observed position{} left out of the minimally constrained adjustment, \
                 which tests the vectors on their own",
                left_out.len(),
                if left_out.len() == 1 {
                    " was"
                } else {
                    "s were"
                }
            ),
        ));
    }
    let mut used: BTreeSet<&str> = BTreeSet::new();
    for m in &measurements {
        for e in &m.elements {
            match e {
                Element::Vector { from, to, .. } => {
                    used.insert(from);
                    used.insert(to);
                }
                Element::Position { station, .. } => {
                    used.insert(station);
                }
            }
        }
    }

    // One station per name. Where files repeat a station, one held fixed wins
    // over free ones, and two held differently conflict. Free stations only
    // give starting positions, so the first with a coordinate is used.
    let mut stations: BTreeMap<&str, &Station> = BTreeMap::new();
    let mut ordered: Vec<&str> = Vec::new();
    for s in &survey.stations {
        let held = |x: &Station| x.constraint != Constraint::Free;
        match stations.get(s.name.as_str()) {
            Some(previous) => {
                let replace = match (held(previous), held(s)) {
                    (true, true) => {
                        if previous.coordinate != s.coordinate
                            || previous.constraint != s.constraint
                        {
                            return Err(fail(
                                Some(&s.origin),
                                format!(
                                    "station {} is also held at {}, with a different coordinate \
                                     or constraint",
                                    s.name, previous.origin
                                ),
                            ));
                        }
                        false
                    }
                    (false, true) => true,
                    (false, false) => previous.coordinate.is_none() && s.coordinate.is_some(),
                    (true, false) => false,
                };
                if replace {
                    stations.insert(&s.name, s);
                }
            }
            None => {
                stations.insert(&s.name, s);
                ordered.push(&s.name);
            }
        }
    }
    let ordered: Vec<&Station> = ordered
        .iter()
        .filter_map(|n| stations.get(n).copied())
        .collect();
    let unused: Vec<&Station> = ordered
        .iter()
        .copied()
        .filter(|s| !used.contains(s.name.as_str()))
        .collect();
    if let Some(first) = unused.first() {
        notes.push(note(
            &first.origin,
            format!(
                "{} station{} in no measurement, so left out: {}",
                unused.len(),
                if unused.len() == 1 { " is" } else { "s are" },
                some_names(unused.iter().map(|s| s.name.as_str()))
            ),
        ));
    }

    // Marks: stations first, in file order, then marks only in measurements.
    let mut assumed_heights: Vec<&Station> = Vec::new();
    for s in ordered.iter().filter(|s| used.contains(s.name.as_str())) {
        let approximate = match &s.coordinate {
            Some(c) => {
                if height_type(c) == Some(HeightType::Orthometric) {
                    assumed_heights.push(s);
                }
                Some(
                    place(c, target, graph)
                        .map_err(|m| fail(Some(&s.origin), format!("station {}: {m}", s.name)))?,
                )
            }
            None => None,
        };
        network
            .add_mark(&s.name, approximate)
            .map_err(|e| fail(Some(&s.origin), e.to_string()))?;
    }
    for m in &measurements {
        for e in &m.elements {
            let names: Vec<&str> = match e {
                Element::Vector { from, to, .. } => vec![from, to],
                Element::Position { station, .. } => vec![station],
            };
            for name in names {
                if network.mark(name).is_none() {
                    network
                        .add_mark(name, None)
                        .map_err(|e| fail(Some(&m.origin), e.to_string()))?;
                }
            }
        }
    }

    // The datum.
    let datum = match &settings.datum {
        DatumChoice::Constrained => {
            let mut fixed = Vec::new();
            for s in ordered.iter().filter(|s| used.contains(s.name.as_str())) {
                match &s.constraint {
                    Constraint::Free => {}
                    Constraint::Fixed => {
                        if s.coordinate.as_ref().and_then(height_type)
                            == Some(HeightType::Orthometric)
                        {
                            return Err(fail(
                                Some(&s.origin),
                                format!(
                                    "station {} is held fixed, but its height is orthometric: \
                                     give it an ellipsoidal height (DynaML type LLh or XYZ), or \
                                     hold it free",
                                    s.name
                                ),
                            ));
                        }
                        fixed.extend(network.mark(&s.name));
                    }
                    Constraint::Partial(code) => {
                        return Err(fail(
                            Some(&s.origin),
                            format!(
                                "station {} has constraints {code}: holding some coordinates \
                                 and not others isn't supported yet. Hold it fixed (CCC) or \
                                 free (FFF), or give it as weighted control",
                                s.name
                            ),
                        ));
                    }
                }
            }
            Datum::Constrained { fixed }
        }
        DatumChoice::Minimal | DatumChoice::FixedMark(_) => {
            let held: Vec<&str> = ordered
                .iter()
                .filter(|s| used.contains(s.name.as_str()) && s.constraint != Constraint::Free)
                .map(|s| s.name.as_str())
                .collect();
            if let Some(first) = held.first().and_then(|n| stations.get(n)) {
                notes.push(note(
                    &first.origin,
                    format!(
                        "held free in the minimally constrained adjustment: {}",
                        some_names(held.iter().copied())
                    ),
                ));
            }
            match &settings.datum {
                DatumChoice::FixedMark(name) => {
                    let mark = network.mark(name).ok_or_else(|| {
                        fail(
                            None,
                            format!("the datum holds mark {name}, but no measurement uses it"),
                        )
                    })?;
                    if network.marks()[mark].approximate.is_none() {
                        return Err(fail(
                            None,
                            format!(
                                "the datum holds mark {name}, but no station gives its coordinate"
                            ),
                        ));
                    }
                    Datum::FixedMark(mark)
                }
                _ => Datum::Inner,
            }
        }
    };
    if let Some(first) = assumed_heights.first() {
        notes.push(note(
            &first.origin,
            format!(
                "{} station{} orthometric heights, used as ellipsoidal heights for starting \
                 positions only: {}",
                assumed_heights.len(),
                if assumed_heights.len() == 1 {
                    " has"
                } else {
                    "s have"
                },
                some_names(assumed_heights.iter().map(|s| s.name.as_str()))
            ),
        ));
    }

    // Approximate positions, from the measurements as given: good enough to
    // linearise the transformations and to orient the scaling.
    let mut draft = network.clone();
    for m in &measurements {
        let elements = elements(&network, m)?;
        let k = elements.len() * 3;
        draft.add_observation(Observation {
            id: String::new(),
            group: None,
            elements,
            covariance: DMatrix::identity(k, k),
        });
    }
    let approximate = draft
        .approximate_positions()
        .map_err(|e| fail(None, e.to_string()))?;

    // The observations, in the network's frame and epoch, scaled.
    let mut edits = Vec::new();
    let mut sources = Vec::new();
    let mut transformations: Vec<Transformation> = Vec::new();
    for m in &measurements {
        let id = format!("{}:{}", m.origin.file, m.origin.line);
        let mut elements = elements(&network, m)?;
        let mut covariance = m.covariance.clone();
        let firsts: Vec<Vector3<f64>> = elements
            .iter()
            .map(|e| approximate[e.first_mark()])
            .collect();
        if !same_reference(m.reference, target) {
            let steps = transform(m, &mut elements, &mut covariance, &firsts, target, graph)?;
            match transformations
                .iter_mut()
                .find(|t| same_reference(t.from, m.reference))
            {
                Some(t) => t.measurements += 1,
                None => transformations.push(Transformation {
                    from: m.reference,
                    measurements: 1,
                    steps,
                }),
            }
        }
        scale(&mut covariance, m.scaling, &firsts, settings.frame)
            .map_err(|e| fail(Some(&m.origin), e))?;
        // Products like J C Jᵀ can come out asymmetric in the last bit.
        let covariance = (&covariance + covariance.transpose()) * 0.5;
        network.add_observation(Observation {
            id: id.clone(),
            group: None,
            elements,
            covariance,
        });
        if m.ignored {
            edits.push(Edit {
                observation: id,
                action: Action::Exclude,
                reason: format!("marked to be ignored in {}", m.origin.file),
            });
        }
        sources.push(Source {
            origin: m.origin.clone(),
            label: m.label(),
            kind: m.kind,
            reference: m.reference,
            scaling: m.scaling,
        });
    }
    Ok(Built {
        network,
        datum,
        edits,
        sources,
        transformations,
        notes,
    })
}

fn height_type(c: &Coordinate) -> Option<HeightType> {
    match c.value {
        CoordinateValue::Cartesian(_) => None,
        CoordinateValue::Geographic { height, .. } | CoordinateValue::Grid { height, .. } => {
            Some(height)
        }
    }
}

/// A coordinate as a position in its own frame. Orthometric heights are
/// taken as ellipsoidal: callers that need better check first.
fn position(c: &Coordinate) -> Result<Position, String> {
    let r = c.reference;
    let geodetic = |latitude_rad: f64, longitude_rad: f64, height_m: f64| {
        Geodetic::new(latitude_rad, longitude_rad, height_m)
            .and_then(|g| Position::from_geodetic(r.frame, r.epoch, &g, None))
    };
    match c.value {
        CoordinateValue::Cartesian(xyz) => Position::new(r.frame, r.epoch, xyz, None),
        CoordinateValue::Geographic {
            latitude_rad,
            longitude_rad,
            height_m,
            ..
        } => geodetic(latitude_rad, longitude_rad, height_m),
        CoordinateValue::Grid {
            easting_m,
            northing_m,
            zone,
            height_m,
            ..
        } => MgaZone::new(zone)
            .and_then(|z| z.projection().inverse(easting_m, northing_m))
            .and_then(|(g, _)| geodetic(g.latitude_rad(), g.longitude_rad(), height_m)),
    }
    .map_err(|e| e.to_string())
}

/// A coordinate's ECEF position in the target frame and epoch.
fn place(
    c: &Coordinate,
    target: Reference,
    graph: &TransformationGraph,
) -> Result<Vector3<f64>, String> {
    let p = position(c)?;
    if same_reference(c.reference, target) {
        return Ok(*p.xyz());
    }
    graph
        .transform(&p, target.frame, target.epoch)
        .map(|t| *t.position.xyz())
        .map_err(|e| e.to_string())
}

/// True if two references are the same frame at the same epoch, treating a
/// static frame's missing epoch as its reference epoch.
fn same_reference(a: Reference, b: Reference) -> bool {
    let epoch = |r: Reference| r.epoch.or(r.frame.reference_epoch());
    a.frame == b.frame && epoch(a) == epoch(b)
}

/// A measurement's elements in terms of the network's marks.
fn elements(network: &Network, m: &Measurement) -> Result<Vec<gnss_adjust::Element>, BuildError> {
    let mark = |name: &str| {
        network
            .mark(name)
            .ok_or_else(|| fail(Some(&m.origin), format!("mark {name} isn't in the network")))
    };
    m.elements
        .iter()
        .map(|e| {
            Ok(match e {
                Element::Vector { from, to, vector } => gnss_adjust::Element::Difference {
                    from: mark(from)?,
                    to: mark(to)?,
                    vector: *vector,
                    setups: [None; 2],
                },
                Element::Position { station, position } => gnss_adjust::Element::Position {
                    mark: mark(station)?,
                    position: *position,
                    setup: None,
                },
            })
        })
        .collect()
}

/// Transforms a measurement's elements and covariance from its frame and
/// epoch to `target`, and returns the steps taken. `firsts` holds the
/// approximate position of each element's first mark.
fn transform(
    m: &Measurement,
    elements: &mut [gnss_adjust::Element],
    covariance: &mut DMatrix<f64>,
    firsts: &[Vector3<f64>],
    target: Reference,
    graph: &TransformationGraph,
) -> Result<Vec<String>, BuildError> {
    let err = |e: gnss_datum::DatumError| {
        fail(
            Some(&m.origin),
            format!(
                "can't transform it from {} to {}: {e}",
                describe(m.reference),
                describe(target)
            ),
        )
    };
    let apply = |x: &Vector3<f64>| -> Result<(Vector3<f64>, Vec<String>), BuildError> {
        let p = Position::new(m.reference.frame, m.reference.epoch, *x, None).map_err(err)?;
        let t = graph
            .transform(&p, target.frame, target.epoch)
            .map_err(err)?;
        Ok((
            *t.position.xyz(),
            t.steps.iter().map(ToString::to_string).collect(),
        ))
    };
    let jacobian = |at: &Vector3<f64>, origin: &Vector3<f64>| -> Result<Matrix3<f64>, BuildError> {
        let mut j = Matrix3::zeros();
        for axis in 0..3 {
            let mut x = *at;
            x[axis] += STEP_M;
            j.set_column(axis, &((apply(&x)?.0 - origin) / STEP_M));
        }
        Ok(j)
    };
    let mut steps = Vec::new();
    let mut jacobians = Vec::with_capacity(elements.len());
    for (element, first) in elements.iter_mut().zip(firsts) {
        match element {
            gnss_adjust::Element::Difference { vector, .. } => {
                let (start, s) = apply(first)?;
                let (end, _) = apply(&(first + *vector))?;
                *vector = end - start;
                jacobians.push(jacobian(first, &start)?);
                steps = s;
            }
            gnss_adjust::Element::Position { position, .. } => {
                let (moved, s) = apply(position)?;
                jacobians.push(jacobian(position, &moved)?);
                *position = moved;
                steps = s;
            }
        }
    }
    let n = jacobians.len();
    let mut transformed = DMatrix::zeros(3 * n, 3 * n);
    for i in 0..n {
        for j in 0..n {
            let block: Matrix3<f64> = covariance.fixed_view::<3, 3>(3 * i, 3 * j).into_owned();
            transformed
                .fixed_view_mut::<3, 3>(3 * i, 3 * j)
                .copy_from(&(jacobians[i] * block * jacobians[j].transpose()));
        }
    }
    *covariance = transformed;
    Ok(steps)
}

/// Applies DynAdjust's variance scaling: `h`, `p` and `l` scale the variances
/// up, north and east at each element's first mark (`firsts`), then `v`
/// scales the whole matrix.
///
/// DynAdjust 1.4.1 multiplies the partial scales by `v` after it has already
/// scaled the matrix by `v`, so a measurement with both gets `v` twice. This
/// applies it once, as DynAdjust's user guide describes (section 7.3.2.1).
fn scale(
    covariance: &mut DMatrix<f64>,
    scaling: Scaling,
    firsts: &[Vector3<f64>],
    frame: FrameId,
) -> Result<(), String> {
    if scaling.p != 1.0 || scaling.l != 1.0 || scaling.h != 1.0 {
        let s = Matrix3::from_diagonal(&Vector3::new(
            scaling.l.sqrt(),
            scaling.p.sqrt(),
            scaling.h.sqrt(),
        ));
        let mut factors = Vec::with_capacity(firsts.len());
        for x in firsts {
            let r = frame
                .ellipsoid()
                .ecef_to_geodetic(x)
                .map_err(|e| e.to_string())?
                .enu_rotation();
            factors.push(r.transpose() * s * r);
        }
        let n = factors.len();
        for i in 0..n {
            for j in 0..n {
                let block: Matrix3<f64> = covariance.fixed_view::<3, 3>(3 * i, 3 * j).into_owned();
                covariance
                    .fixed_view_mut::<3, 3>(3 * i, 3 * j)
                    .copy_from(&(factors[i] * block * factors[j].transpose()));
            }
        }
    }
    if scaling.v != 1.0 {
        *covariance *= scaling.v;
    }
    Ok(())
}

fn describe(r: Reference) -> String {
    match (r.frame.is_static(), r.epoch) {
        (false, Some(e)) => format!("{} at {e}", r.frame),
        _ => r.frame.to_string(),
    }
}
