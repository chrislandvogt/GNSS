//! The network to adjust: marks, the observations between them, the datum
//! that fixes them, and the options for the tests (ARCHITECTURE.md section 10).

use std::collections::{BTreeMap, VecDeque};

use gnss_datum::{CoordinateEpoch, FrameId};
use nalgebra::{DMatrix, Matrix3, Vector3};

use crate::error::AdjustError;

/// A survey mark: a point whose position the adjustment estimates.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    /// The mark's name, unique in the network.
    pub name: String,
    /// An approximate ECEF position, in metres. A mark held fixed is held
    /// here. Marks without one are placed from their neighbours along the
    /// observed vectors, or at their observed position.
    pub approximate: Option<Vector3<f64>>,
}

/// One three-component part of an observation, in ECEF metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Element {
    /// A GNSS vector: the position of `to` minus the position of `from`.
    Difference {
        /// The mark the vector starts at.
        from: usize,
        /// The mark the vector ends at.
        to: usize,
        /// The vector.
        vector: Vector3<f64>,
    },
    /// A mark's position: control, a CORS, or an AUSPOS solution.
    Position {
        /// The mark.
        mark: usize,
        /// The position.
        position: Vector3<f64>,
    },
}

impl Element {
    /// The marks in the element, each with the sign of its position in it.
    pub(crate) fn terms(&self) -> impl Iterator<Item = (usize, f64)> {
        let (first, second) = match *self {
            Element::Difference { from, to, .. } => ((from, -1.0), Some((to, 1.0))),
            Element::Position { mark, .. } => ((mark, 1.0), None),
        };
        std::iter::once(first).chain(second)
    }

    /// The mark whose local east, north and up axes the element's tests use:
    /// where a vector starts, or the mark of a position.
    pub fn first_mark(&self) -> usize {
        match *self {
            Element::Difference { from, .. } => from,
            Element::Position { mark, .. } => mark,
        }
    }

    /// The observed value.
    pub(crate) fn value(&self) -> Vector3<f64> {
        match *self {
            Element::Difference { vector, .. } => vector,
            Element::Position { position, .. } => position,
        }
    }

    /// Adds `delta` to the observed value.
    pub(crate) fn perturb(&mut self, delta: &Vector3<f64>) {
        match self {
            Element::Difference { vector, .. } => *vector += delta,
            Element::Position { position, .. } => *position += delta,
        }
    }
}

/// Elements observed together, with the covariance of all their components.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    /// An identifier for reports and errors, such as the baseline's file name.
    pub id: String,
    /// The group whose scale factor applies to the covariance, such as a
    /// session or an engine. Observations in no group have scale 1.
    pub group: Option<String>,
    /// The elements.
    pub elements: Vec<Element>,
    /// The covariance of the elements' components (X, Y and Z of the first
    /// element, then of the next), 3n × 3n for n elements, in m².
    pub covariance: DMatrix<f64>,
}

impl Observation {
    /// A GNSS baseline from `from` to `to`, with its covariance.
    pub fn baseline(
        id: impl Into<String>,
        from: usize,
        to: usize,
        vector: Vector3<f64>,
        covariance: Matrix3<f64>,
    ) -> Self {
        Observation {
            id: id.into(),
            group: None,
            elements: vec![Element::Difference { from, to, vector }],
            covariance: DMatrix::from_column_slice(3, 3, covariance.as_slice()),
        }
    }

    /// Baselines solved together, such as one session's, as (from, to,
    /// vector), with the covariance of all their components.
    pub fn baseline_cluster(
        id: impl Into<String>,
        baselines: &[(usize, usize, Vector3<f64>)],
        covariance: DMatrix<f64>,
    ) -> Self {
        Observation {
            id: id.into(),
            group: None,
            elements: baselines
                .iter()
                .map(|&(from, to, vector)| Element::Difference { from, to, vector })
                .collect(),
            covariance,
        }
    }

    /// A mark's position, with its covariance.
    pub fn point(
        id: impl Into<String>,
        mark: usize,
        position: Vector3<f64>,
        covariance: Matrix3<f64>,
    ) -> Self {
        Self::point_cluster(
            id,
            &[(mark, position)],
            DMatrix::from_column_slice(3, 3, covariance.as_slice()),
        )
    }

    /// Positions of marks, as (mark, position), with the covariance of all
    /// their components.
    pub fn point_cluster(
        id: impl Into<String>,
        points: &[(usize, Vector3<f64>)],
        covariance: DMatrix<f64>,
    ) -> Self {
        Observation {
            id: id.into(),
            group: None,
            elements: points
                .iter()
                .map(|&(mark, position)| Element::Position { mark, position })
                .collect(),
            covariance,
        }
    }

    /// Puts the observation in a group, for its scale factor.
    pub fn with_group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }
}

/// What fixes the network's position (ARCHITECTURE.md section 10.3). GNSS
/// vectors carry scale and orientation, so only translation is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Datum {
    /// Control fixes the network: point observations enter with their
    /// covariance, and the marks listed are held at their approximate
    /// positions. Every part of the network needs one or the other.
    Constrained {
        /// Marks held fixed.
        fixed: Vec<usize>,
    },
    /// Minimally constrained by holding one mark at its approximate position.
    FixedMark(usize),
    /// Minimally constrained by inner constraints on translation: the
    /// adjusted positions keep the centroid of the approximate ones.
    Inner,
}

/// Settings for an adjustment.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// The a priori variance factor σ₀². An observation's covariance is taken
    /// as σ₀² times its group's scale factor times the matrix given.
    pub variance_factor: f64,
    /// Scale factors for groups of observations. Groups not listed have scale 1.
    pub group_scales: BTreeMap<String, f64>,
    /// The level of the w-test, α₀, from which the B-method sets the other
    /// tests' levels.
    pub alpha0: f64,
    /// The power β₀ with which every test detects a bias the size of its
    /// minimal detectable bias.
    pub beta0: f64,
    /// The level of the global test, which is two-tailed.
    pub global_alpha: f64,
}

impl Default for Options {
    /// σ₀² = 1, no group scales, α₀ = 0.1% and β₀ = 80% (λ₀ ≈ 17.07), and a
    /// 5% global test, as in the ICSM guideline and DynAdjust.
    fn default() -> Self {
        Options {
            variance_factor: 1.0,
            group_scales: BTreeMap::new(),
            alpha0: 0.001,
            beta0: 0.8,
            global_alpha: 0.05,
        }
    }
}

impl Options {
    /// Checks every option is in range.
    pub(crate) fn check(&self) -> Result<(), AdjustError> {
        let invalid = |name, expected, value| {
            Err(AdjustError::InvalidOption {
                name,
                expected,
                value,
            })
        };
        let positive = |v: f64| v.is_finite() && v > 0.0;
        if !positive(self.variance_factor) {
            return invalid(
                "the a priori variance factor",
                "positive",
                self.variance_factor,
            );
        }
        for &scale in self.group_scales.values() {
            if !positive(scale) {
                return invalid("a group scale factor", "positive", scale);
            }
        }
        if !(self.alpha0 > 0.0 && self.alpha0 <= 0.5) {
            return invalid("alpha0", "above 0 and at most 0.5", self.alpha0);
        }
        if !(0.5..1.0).contains(&self.beta0) {
            return invalid("beta0", "at least 0.5 and below 1", self.beta0);
        }
        if !(self.global_alpha > 0.0 && self.global_alpha < 1.0) {
            return invalid("global_alpha", "between 0 and 1", self.global_alpha);
        }
        Ok(())
    }

    /// The scale factor for a group.
    pub(crate) fn scale(&self, group: Option<&String>) -> f64 {
        group
            .and_then(|g| self.group_scales.get(g))
            .copied()
            .unwrap_or(1.0)
    }
}

/// Marks and the observations between them, in one frame at one epoch.
#[derive(Debug, Clone, PartialEq)]
pub struct Network {
    frame: FrameId,
    epoch: CoordinateEpoch,
    marks: Vec<Mark>,
    names: BTreeMap<String, usize>,
    observations: Vec<Observation>,
}

impl Network {
    /// An empty network in `frame` at `epoch`. A static frame needs no epoch;
    /// a dynamic one does.
    pub fn new(frame: FrameId, epoch: Option<CoordinateEpoch>) -> Result<Self, AdjustError> {
        Ok(Network {
            frame,
            epoch: frame.resolve_epoch(epoch)?,
            marks: Vec::new(),
            names: BTreeMap::new(),
            observations: Vec::new(),
        })
    }

    /// The frame of every position and vector in the network.
    pub fn frame(&self) -> FrameId {
        self.frame
    }

    /// The coordinate epoch of every position and vector in the network.
    pub fn epoch(&self) -> CoordinateEpoch {
        self.epoch
    }

    /// The marks, by index.
    pub fn marks(&self) -> &[Mark] {
        &self.marks
    }

    /// The observations, by index.
    pub fn observations(&self) -> &[Observation] {
        &self.observations
    }

    /// Adds a mark and returns its index.
    pub fn add_mark(
        &mut self,
        name: impl Into<String>,
        approximate: Option<Vector3<f64>>,
    ) -> Result<usize, AdjustError> {
        let name = name.into();
        if self.names.contains_key(&name) {
            return Err(AdjustError::DuplicateMark { name });
        }
        let index = self.marks.len();
        self.names.insert(name.clone(), index);
        self.marks.push(Mark { name, approximate });
        Ok(index)
    }

    /// Sets a mark's approximate position: where a fixed mark is held, and
    /// where the adjustment starts from.
    pub fn set_approximate(
        &mut self,
        mark: usize,
        approximate: Option<Vector3<f64>>,
    ) -> Result<(), AdjustError> {
        let count = self.marks.len();
        let m = self
            .marks
            .get_mut(mark)
            .ok_or(AdjustError::NoSuchMark { index: mark, count })?;
        m.approximate = approximate;
        Ok(())
    }

    /// The index of the mark with this name.
    pub fn mark(&self, name: &str) -> Option<usize> {
        self.names.get(name).copied()
    }

    /// The observations, to change. They're checked when the network is adjusted.
    pub fn observations_mut(&mut self) -> &mut [Observation] {
        &mut self.observations
    }

    /// Adds an observation and returns its index. It's checked when the
    /// network is adjusted.
    pub fn add_observation(&mut self, observation: Observation) -> usize {
        self.observations.push(observation);
        self.observations.len() - 1
    }

    pub(crate) fn name(&self, mark: usize) -> String {
        self.marks[mark].name.clone()
    }

    /// Checks each observation's marks and values, and that every mark is observed.
    pub(crate) fn check_observations(&self) -> Result<(), AdjustError> {
        if self.marks.is_empty() {
            return Err(AdjustError::Empty);
        }
        let mut observed = vec![false; self.marks.len()];
        for o in &self.observations {
            let invalid = |reason: String| {
                Err(AdjustError::InvalidObservation {
                    id: o.id.clone(),
                    reason,
                })
            };
            if o.elements.is_empty() {
                return invalid("it has no elements".into());
            }
            for element in &o.elements {
                for (mark, _) in element.terms() {
                    if mark >= self.marks.len() {
                        return invalid(format!(
                            "it refers to mark {mark}, but the network has {} marks",
                            self.marks.len()
                        ));
                    }
                    observed[mark] = true;
                }
                if let Element::Difference { from, to, .. } = element
                    && from == to
                {
                    return invalid(format!(
                        "it has a vector from mark {} to itself",
                        self.marks[*from].name
                    ));
                }
                if element.value().iter().any(|v| !v.is_finite()) {
                    return invalid("it has a value that isn't finite".into());
                }
            }
        }
        if let Some(mark) = observed.iter().position(|o| !o) {
            return Err(AdjustError::Unobserved {
                name: self.name(mark),
            });
        }
        Ok(())
    }

    /// The marks in each connected part of the network, joined by vectors.
    /// Parts are in order of their first mark, and marks in index order.
    pub(crate) fn parts(&self) -> Vec<Vec<usize>> {
        let mut parent: Vec<usize> = (0..self.marks.len()).collect();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for o in &self.observations {
            for element in &o.elements {
                if let Element::Difference { from, to, .. } = *element {
                    let (a, b) = (root(&mut parent, from), root(&mut parent, to));
                    // The lower index becomes the root, so parts are ordered by first mark.
                    parent[a.max(b)] = a.min(b);
                }
            }
        }
        let mut parts: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for mark in 0..self.marks.len() {
            let r = root(&mut parent, mark);
            parts.entry(r).or_default().push(mark);
        }
        parts.into_values().collect()
    }

    /// Checks that the datum fixes every part of the network, and returns
    /// the marks held fixed while solving: the datum's fixed marks, or for
    /// inner constraints the first mark, whose solution is then transformed.
    pub(crate) fn check_datum(&self, datum: &Datum) -> Result<Vec<bool>, AdjustError> {
        let n = self.marks.len();
        let mut fixed = vec![false; n];
        let parts = self.parts();
        let mut controlled = vec![false; n];
        let mut control_observation = None;
        for o in &self.observations {
            for element in &o.elements {
                if let Element::Position { mark, .. } = *element {
                    controlled[mark] = true;
                    control_observation.get_or_insert((o.id.clone(), mark));
                }
            }
        }
        let hold = |mark: usize, fixed: &mut Vec<bool>| {
            if mark >= n {
                return Err(AdjustError::InvalidDatum {
                    reason: format!(
                        "the datum holds mark {mark} fixed, but the network has {n} marks"
                    ),
                });
            }
            if self.marks[mark].approximate.is_none() {
                return Err(AdjustError::NoApproximatePosition {
                    name: self.name(mark),
                    reason: "it's held fixed there",
                });
            }
            fixed[mark] = true;
            Ok(())
        };
        match datum {
            Datum::Constrained { fixed: marks } => {
                for &mark in marks {
                    hold(mark, &mut fixed)?;
                }
                for part in &parts {
                    if !part.iter().any(|&m| fixed[m] || controlled[m]) {
                        return Err(AdjustError::UndefinedDatum {
                            marks: part.iter().map(|&m| self.name(m)).collect(),
                        });
                    }
                }
            }
            Datum::FixedMark(_) | Datum::Inner => {
                if let Some((id, mark)) = control_observation {
                    return Err(AdjustError::InvalidDatum {
                        reason: format!(
                            "observation {id} observes the position of mark {}, but a minimal \
                             datum can't have control: use a constrained datum, or leave the \
                             observation out",
                            self.name(mark)
                        ),
                    });
                }
                if parts.len() > 1 {
                    return Err(AdjustError::Disconnected {
                        parts: parts
                            .iter()
                            .map(|p| p.iter().map(|&m| self.name(m)).collect())
                            .collect(),
                    });
                }
                match datum {
                    Datum::FixedMark(mark) => hold(*mark, &mut fixed)?,
                    _ => {
                        if !self.marks.iter().any(|m| m.approximate.is_some()) {
                            return Err(AdjustError::NoApproximatePosition {
                                name: self.name(0),
                                reason: "inner constraints keep the centroid of the approximate \
                                         positions, so at least one mark needs one",
                            });
                        }
                        fixed[0] = true;
                    }
                }
            }
        }
        Ok(fixed)
    }

    /// Approximate positions for every mark: those given, then observed
    /// positions, then the rest along the observed vectors, breadth first.
    pub(crate) fn approximate_positions(&self) -> Result<Vec<Vector3<f64>>, AdjustError> {
        let n = self.marks.len();
        let mut positions: Vec<Option<Vector3<f64>>> =
            self.marks.iter().map(|m| m.approximate).collect();
        let mut neighbours: Vec<Vec<(usize, Vector3<f64>)>> = vec![Vec::new(); n];
        for o in &self.observations {
            for element in &o.elements {
                match *element {
                    Element::Difference { from, to, vector } => {
                        neighbours[from].push((to, vector));
                        neighbours[to].push((from, -vector));
                    }
                    Element::Position { mark, position } => {
                        positions[mark].get_or_insert(position);
                    }
                }
            }
        }
        let mut queue: VecDeque<usize> = (0..n).filter(|&m| positions[m].is_some()).collect();
        while let Some(a) = queue.pop_front() {
            let Some(x) = positions[a] else { continue };
            for &(b, vector) in &neighbours[a] {
                if positions[b].is_none() {
                    positions[b] = Some(x + vector);
                    queue.push_back(b);
                }
            }
        }
        positions
            .into_iter()
            .enumerate()
            .map(|(m, p)| {
                p.ok_or_else(|| AdjustError::NoApproximatePosition {
                    name: self.name(m),
                    reason: "nothing joins it to a mark with a known position",
                })
            })
            .collect()
    }
}
