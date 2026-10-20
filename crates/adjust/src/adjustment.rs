//! The adjustment itself: normal equations, the solution in the chosen
//! datum, residuals, tests and reliability (ARCHITECTURE.md sections 10.4 to
//! 10.6).
//!
//! The unknowns are corrections to the marks' approximate ECEF positions. A
//! GNSS model is linear in them, so one solution is exact and needs no
//! iteration. Marks held fixed have no unknowns. Inner constraints are solved
//! by holding the first mark and moving the solution to the centroid
//! afterwards (an S-transformation), which leaves residuals and tests as they
//! are: they don't depend on the datum, and the tests check that.

use gnss_datum::{CoordinateEpoch, Covariance, DatumError, FrameId, Position};
use nalgebra::{DMatrix, DVector, Matrix3, Vector3};

use crate::error::AdjustError;
use crate::network::{Action, Datum, Edit, Network, Observation, Options};
use crate::sparse::{Block, BlockMatrix, Factor, SelectedInverse, spd_inverse};
use crate::stats::{self, BMethod};
use crate::uncertainty::{Scaling, Uncertainty};

/// A component is tested only if the other observations check at least this
/// share of it. Below this, its residual is rounding error divided by
/// rounding error.
const MIN_REDUNDANCY: f64 = 1e-8;

/// Relative tolerance for a covariance matrix's symmetry.
const SYMMETRY_TOLERANCE: f64 = 1e-9;

/// A covariance is singular, or near enough, when a component's variance
/// given the others is below this share of its variance: a correlation
/// within 5e-11 of one.
const MIN_CONDITIONAL_VARIANCE: f64 = 1e-10;

/// A network after adjustment.
#[derive(Debug, Clone, PartialEq)]
pub struct Adjustment {
    frame: FrameId,
    epoch: CoordinateEpoch,
    datum: Datum,
    marks: Vec<AdjustedMark>,
    observations: Vec<ObservationResult>,
    statistics: Statistics,
    criteria: Criteria,
    setups: Vec<SetupResult>,
    a_priori: f64,
    edits: Vec<Edit>,
    solution: Solution,
}

/// A mark's adjusted position.
#[derive(Debug, Clone, PartialEq)]
pub struct AdjustedMark {
    /// The adjusted ECEF position, in metres, in the network's frame and epoch.
    pub position: Vector3<f64>,
    /// The position's cofactor matrix: its covariance per unit variance
    /// factor, in m². Zero for a mark held fixed. Multiply by the a priori or
    /// a posteriori variance factor for a covariance.
    pub cofactor: Matrix3<f64>,
    /// True if the datum held the mark fixed.
    pub fixed: bool,
}

/// The results for one observation, by element.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservationResult {
    /// One per element, in order.
    pub elements: Vec<ElementResult>,
    /// The observation's share of the weighted sum of squares, vᵀPv.
    pub sum_of_squares: f64,
    /// False if an edit excluded the observation. Its residuals are then its
    /// misclosures against the network adjusted without it, and each
    /// element's vector test says whether it would fit.
    pub used: bool,
    /// The factor its covariance was scaled by: its group's scale factor
    /// times any re-weighting.
    pub scale: f64,
}

/// The results for one set-up: its estimated centring and height errors,
/// and the tests of its prior.
#[derive(Debug, Clone, PartialEq)]
pub struct SetupResult {
    /// The mark occupied.
    pub mark: usize,
    /// The estimated error, in ECEF metres, which every vector measured from
    /// the set-up shares.
    pub error: Vector3<f64>,
    /// East, north and up, on the mark's local axes: the error as the
    /// residual of the prior that it's zero, and that prior's tests, which
    /// flag a misread height or an antenna off its mark.
    pub components: [ComponentResult; 3],
    /// The test of the whole error against its prior, with 3 degrees of
    /// freedom.
    pub vector_test: Option<f64>,
    /// The prior's share of the weighted sum of squares.
    pub sum_of_squares: f64,
}

/// What a failed test is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Subject {
    /// An element of an observation.
    Element {
        /// The observation.
        observation: usize,
        /// The element.
        element: usize,
    },
    /// A set-up's prior.
    Setup(usize),
}

/// A failed test, for a person to judge (ARCHITECTURE.md section 10.5).
#[derive(Debug, Clone, PartialEq)]
pub struct Suspect {
    /// What failed.
    pub subject: Subject,
    /// The local component (0 east, 1 north, 2 up) whose w-test failed, or
    /// `None` for the vector test.
    pub component: Option<usize>,
    /// The test statistic: w for a component, T for the whole.
    pub statistic: f64,
    /// How far the test failed, on a scale that grows in proportion to the
    /// bias: |w| over its critical value, or √(T over its critical value).
    pub excess: f64,
}

/// The results for one element of an observation.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementResult {
    /// The residual, adjusted minus observed, in ECEF metres.
    pub residual: Vector3<f64>,
    /// East, north and up, on the local axes of the element's first mark
    /// ([`crate::Element::first_mark`]).
    pub components: [ComponentResult; 3],
    /// The test of the whole element, with 3 degrees of freedom:
    /// T = (Pv)ᵀ(PQᵥᵥP)⁻¹(Pv)/σ₀² over the element's components. `None` if a
    /// component isn't checked by the other observations.
    pub vector_test: Option<f64>,
    /// Each ECEF component's residual over its own standard deviation,
    /// vₖ/(σ₀√Qᵥᵥ,ₖₖ), ignoring correlations: DynAdjust's N-stat. It equals
    /// the w-test only for uncorrelated components. `None` as for `w`.
    pub standardised: [Option<f64>; 3],
}

/// The results for one component of an element.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentResult {
    /// The residual, adjusted minus observed, in metres.
    pub residual: f64,
    /// The redundancy number: the share of the component that the other
    /// observations check, from 0 (not at all) to 1.
    pub redundancy: f64,
    /// The w-test statistic, the standardised residual with the a priori
    /// variance factor. `None` if the other observations don't check the
    /// component.
    pub w: Option<f64>,
    /// Pope's tau statistic, the standardised residual with the a posteriori
    /// variance factor. `None` as for `w`, or with under 2 degrees of freedom.
    pub tau: Option<f64>,
    /// The minimal detectable bias, in metres: the bias in this component
    /// that the w-test detects with power β₀.
    pub mdb: Option<f64>,
    /// External reliability, in metres: the largest shift that an undetected
    /// bias the size of the MDB causes in the position of the observation's
    /// own marks, relative to the datum.
    pub shift: Option<f64>,
}

/// The adjustment's overall statistics.
#[derive(Debug, Clone, PartialEq)]
pub struct Statistics {
    /// The weighted sum of squared residuals, Ω = vᵀPv, with P the inverse of
    /// the cofactor matrix of the observations.
    pub sum_of_squares: f64,
    /// The degrees of freedom: observation components minus unknowns.
    pub degrees_of_freedom: usize,
    /// The a posteriori variance factor, σ̂₀² = Ω/r. `None` without
    /// redundancy.
    pub variance_factor: Option<f64>,
    /// The global test. `None` without redundancy.
    pub global_test: Option<GlobalTest>,
}

/// The global test of the model: Ω/σ₀² against χ²(r), two-tailed.
#[derive(Debug, Clone, PartialEq)]
pub struct GlobalTest {
    /// Ω/σ₀².
    pub statistic: f64,
    /// The lower critical value, the α/2 quantile of χ²(r).
    pub lower: f64,
    /// The upper critical value, the 1 − α/2 quantile of χ²(r).
    pub upper: f64,
    /// True if the statistic is within the critical values.
    pub passed: bool,
}

/// The critical values of the tests (B-method).
#[derive(Debug, Clone, PartialEq)]
pub struct Criteria {
    /// The w-test's level, α₀.
    pub alpha0: f64,
    /// The power of every test, β₀.
    pub beta0: f64,
    /// The non-centrality that α₀ and β₀ give, λ₀.
    pub lambda0: f64,
    /// A w statistic beyond ± this fails: z(1 − α₀/2).
    pub w: f64,
    /// A vector test statistic above this fails.
    pub vector: f64,
    /// The vector test's level, which the B-method derives from α₀ and β₀.
    pub vector_alpha: f64,
    /// A tau statistic beyond ± this fails (Pope, at level α₀). `None` with
    /// under 2 degrees of freedom.
    pub tau: Option<f64>,
    /// The global test's level.
    pub global_alpha: f64,
}

impl Adjustment {
    /// The frame of the adjusted positions.
    pub fn frame(&self) -> FrameId {
        self.frame
    }

    /// The coordinate epoch of the adjusted positions.
    pub fn epoch(&self) -> CoordinateEpoch {
        self.epoch
    }

    /// The datum.
    pub fn datum(&self) -> &Datum {
        &self.datum
    }

    /// A mark's adjusted position with its covariance, as a datum-crate
    /// [`Position`] in the network's frame and epoch: ready to transform to
    /// another frame or epoch, or to project, with the covariance carried
    /// along.
    pub fn position(&self, mark: usize, scaling: Scaling) -> Result<Position, AdjustError> {
        let covariance = Covariance::new(self.covariance(mark, scaling)?)?;
        Ok(Position::new(
            self.frame,
            Some(self.epoch),
            self.marks[mark].position,
            Some(covariance),
        )?)
    }

    /// The adjusted marks, in the network's order.
    pub fn marks(&self) -> &[AdjustedMark] {
        &self.marks
    }

    /// The results for each observation, in the network's order.
    pub fn observations(&self) -> &[ObservationResult] {
        &self.observations
    }

    /// The results for each set-up, in the network's order.
    pub fn setups(&self) -> &[SetupResult] {
        &self.setups
    }

    /// The overall statistics.
    pub fn statistics(&self) -> &Statistics {
        &self.statistics
    }

    /// The critical values.
    pub fn criteria(&self) -> &Criteria {
        &self.criteria
    }

    /// The edits applied: the audit trail of exclusions and re-weightings.
    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    /// Every failed w-test and vector test of the observations used, worst
    /// first. They're evidence: leaving an observation out or re-weighting
    /// it is a person's [`Edit`].
    pub fn suspects(&self) -> Vec<Suspect> {
        let mut suspects = Vec::new();
        let mut add = |subject: Subject, components: &[ComponentResult; 3], test: Option<f64>| {
            for (k, c) in components.iter().enumerate() {
                if let Some(w) = c.w
                    && w.abs() > self.criteria.w
                {
                    suspects.push(Suspect {
                        subject,
                        component: Some(k),
                        statistic: w,
                        excess: w.abs() / self.criteria.w,
                    });
                }
            }
            if let Some(t) = test
                && t > self.criteria.vector
            {
                suspects.push(Suspect {
                    subject,
                    component: None,
                    statistic: t,
                    excess: libm::sqrt(t / self.criteria.vector),
                });
            }
        };
        for (observation, result) in self.observations.iter().enumerate().filter(|(_, r)| r.used) {
            for (element, e) in result.elements.iter().enumerate() {
                let subject = Subject::Element {
                    observation,
                    element,
                };
                add(subject, &e.components, e.vector_test);
            }
        }
        for (s, setup) in self.setups.iter().enumerate() {
            add(Subject::Setup(s), &setup.components, setup.vector_test);
        }
        suspects.sort_by(|a, b| {
            b.excess
                .total_cmp(&a.excess)
                .then(a.subject.cmp(&b.subject))
                .then(a.component.cmp(&b.component))
        });
        suspects
    }

    /// The variance factor that `scaling` uses. The a posteriori factor
    /// needs redundancy.
    pub fn variance_factor(&self, scaling: Scaling) -> Result<f64, AdjustError> {
        match scaling {
            Scaling::APriori => Ok(self.a_priori),
            Scaling::APosteriori => self
                .statistics
                .variance_factor
                .ok_or(AdjustError::NoRedundancy),
        }
    }

    /// The covariance of a mark's position, in ECEF m². Zero for a mark held
    /// fixed.
    pub fn covariance(&self, mark: usize, scaling: Scaling) -> Result<Matrix3<f64>, AdjustError> {
        self.cross_covariance(mark, mark, scaling)
    }

    /// The covariance between two marks' positions, block (a, b), in ECEF
    /// m². Pairs that share an observation come from the selected inverse;
    /// others cost three solves.
    pub fn cross_covariance(
        &self,
        a: usize,
        b: usize,
        scaling: Scaling,
    ) -> Result<Matrix3<f64>, AdjustError> {
        self.check_mark(a)?;
        self.check_mark(b)?;
        Ok(self.solution.datum_any(a, b) * self.variance_factor(scaling)?)
    }

    /// A mark's positional uncertainty, relative to the datum, on its local
    /// axes: SP1's PU, with control uncertainty included.
    pub fn positional_uncertainty(
        &self,
        mark: usize,
        scaling: Scaling,
    ) -> Result<Uncertainty, AdjustError> {
        let covariance = self.covariance(mark, scaling)?;
        let axes = self.solution.axes[mark];
        Ok(Uncertainty::from_enu(axes * covariance * axes.transpose()))
    }

    /// The uncertainty of mark `b`'s position relative to mark `a`'s, on
    /// `a`'s local axes: SP1's RU, which leaves out what the two share,
    /// such as the datum's own uncertainty.
    pub fn relative_uncertainty(
        &self,
        a: usize,
        b: usize,
        scaling: Scaling,
    ) -> Result<Uncertainty, AdjustError> {
        let ab = self.cross_covariance(a, b, scaling)?;
        let relative =
            self.covariance(a, scaling)? + self.covariance(b, scaling)? - ab - ab.transpose();
        let axes = self.solution.axes[a];
        Ok(Uncertainty::from_enu(axes * relative * axes.transpose()))
    }

    /// An excluded observation compared with the adjusted network: its
    /// misclosures d, each element's test dᵀΣ⁻¹d/σ₀² with Σ = Q + AQₓAᵀ, and
    /// each component's standardised misclosure.
    fn excluded(&self, o: &Observation, w: &Weights, scale: f64) -> ObservationResult {
        let sigma0 = libm::sqrt(self.a_priori);
        let elements = o
            .elements
            .iter()
            .enumerate()
            .map(|(p, element)| {
                let marks = self.marks.len();
                let mut adjusted = Vector3::zeros();
                for (a, sa) in element.terms() {
                    adjusted += self.marks[a].position * sa;
                }
                for (setup, sign) in element.setup_terms() {
                    adjusted += self.setups[setup].error * sign;
                }
                let mut aqa = Block::zeros();
                for (a, sa) in element.nodes(marks) {
                    for (b, sb) in element.nodes(marks) {
                        aqa += self.solution.datum_any(a, b) * (sa * sb);
                    }
                }
                let misclosure = adjusted - element.value();
                let own: Block = w.cofactor.fixed_view::<3, 3>(3 * p, 3 * p).into_owned();
                let spread = own + aqa;
                let axes = self.solution.axes[element.first_mark()];
                let enu = axes * misclosure;
                ElementResult {
                    residual: misclosure,
                    components: std::array::from_fn(|k| ComponentResult {
                        residual: enu[k],
                        redundancy: 0.0,
                        w: None,
                        tau: None,
                        mdb: None,
                        shift: None,
                    }),
                    vector_test: spd_inverse(&spread)
                        .map(|inverse| misclosure.dot(&(inverse * misclosure)) / self.a_priori),
                    standardised: std::array::from_fn(|k| {
                        Some(misclosure[k] / (sigma0 * libm::sqrt(spread[(k, k)])))
                    }),
                }
            })
            .collect();
        ObservationResult {
            elements,
            sum_of_squares: 0.0,
            used: false,
            scale,
        }
    }

    fn check_mark(&self, index: usize) -> Result<(), AdjustError> {
        if index < self.marks.len() {
            Ok(())
        } else {
            Err(AdjustError::NoSuchMark {
                index,
                count: self.marks.len(),
            })
        }
    }
}

impl Network {
    /// Adjusts the network in `datum`, with the edits in `options`.
    pub fn adjust(&self, datum: &Datum, options: &Options) -> Result<Adjustment, AdjustError> {
        options.check()?;
        self.check_observations()?;
        // Each observation's re-weighting factor, or None if it's excluded.
        let mut usage: Vec<Option<f64>> = vec![Some(1.0); self.observations().len()];
        for edit in &options.edits {
            let mut matches = self
                .observations()
                .iter()
                .enumerate()
                .filter(|(_, o)| o.id == edit.observation)
                .map(|(i, _)| i);
            let (Some(i), None) = (matches.next(), matches.next()) else {
                let count = self
                    .observations()
                    .iter()
                    .filter(|o| o.id == edit.observation)
                    .count();
                let id = edit.observation.clone();
                return Err(if count == 0 {
                    AdjustError::NoSuchObservation { id }
                } else {
                    AdjustError::AmbiguousObservation { id, count }
                });
            };
            usage[i] = match (usage[i], edit.action) {
                (Some(factor), Action::Reweight(more)) => Some(factor * more),
                _ => None,
            };
        }

        let used: Vec<Observation> = self
            .observations()
            .iter()
            .zip(&usage)
            .filter_map(|(o, u)| {
                u.map(|factor| {
                    let mut o = o.clone();
                    o.covariance *= factor;
                    o
                })
            })
            .collect();
        let mut adjustment = self
            .with_observations(used)
            .adjust_used(datum, options)
            .map_err(|e| match e {
                AdjustError::Unobserved { name } => AdjustError::OnlyExcluded { name },
                other => other,
            })?;
        adjustment.edits = options.edits.clone();

        // Back in the network's order, with the excluded observations
        // compared with the network adjusted without them.
        let mut results = std::mem::take(&mut adjustment.observations).into_iter();
        let mut observations = Vec::with_capacity(usage.len());
        for (o, u) in self.observations().iter().zip(&usage) {
            let scale = options.scale(o.group.as_ref());
            // The used observations' results come in order, one each.
            let used = u.and_then(|factor| results.next().map(|result| (factor, result)));
            let result = match used {
                Some((factor, mut result)) => {
                    result.scale = scale * factor;
                    result
                }
                None => adjustment.excluded(o, &Weights::new(o, scale)?, scale),
            };
            observations.push(result);
        }
        adjustment.observations = observations;
        Ok(adjustment)
    }

    /// Adjusts every observation of the network in `datum`.
    fn adjust_used(&self, datum: &Datum, options: &Options) -> Result<Adjustment, AdjustError> {
        // Again without the excluded observations: a mark may have nothing left.
        self.check_observations()?;
        let held = self.check_datum(datum)?;
        let approximate = self.approximate_positions()?;
        let weights = self
            .observations()
            .iter()
            .map(|o| Weights::new(o, options.scale(o.group.as_ref())))
            .collect::<Result<Vec<_>, _>>()?;
        // Local axes at each mark's approximate position, the same whatever
        // the datum: within a microradian of the adjusted position's.
        let ellipsoid = self.frame().ellipsoid();
        let axes = approximate
            .iter()
            .enumerate()
            .map(|(m, x)| {
                ellipsoid
                    .ecef_to_geodetic(x)
                    .map(|g| g.enu_rotation())
                    .map_err(|source: DatumError| AdjustError::MarkPosition {
                        name: self.name(m),
                        source,
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Each set-up's prior: its centring and height errors on its mark's
        // local axes, in ECEF.
        let marks = self.marks().len();
        let priors: Vec<Block> = self
            .setups()
            .iter()
            .map(|setup| {
                let (h, v) = (setup.sigma_horizontal, setup.sigma_vertical);
                let enu = Block::from_diagonal(&Vector3::new(h * h, h * h, v * v));
                axes[setup.mark].transpose() * enu * axes[setup.mark]
            })
            .collect();

        // A block of unknowns per mark not held, then one per set-up: the
        // nodes, numbered with the set-ups after the marks.
        let nodes = marks + priors.len();
        let free: Vec<usize> = (0..nodes).filter(|&n| n >= marks || !held[n]).collect();
        let mut unknown = vec![None; nodes];
        for (u, &n) in free.iter().enumerate() {
            unknown[n] = Some(u);
        }

        let misclosures: Vec<DVector<f64>> = self
            .observations()
            .iter()
            .map(|o| misclosure(o, &approximate))
            .collect();
        let mut normal = BlockMatrix::new(free.len());
        let mut rhs = vec![Vector3::zeros(); free.len()];
        for ((o, w), l) in self.observations().iter().zip(&weights).zip(&misclosures) {
            let pl = &w.weight * l;
            for (p, ep) in o.elements.iter().enumerate() {
                for (a, sa) in ep.nodes(marks) {
                    if let Some(ua) = unknown[a] {
                        rhs[ua] += pl.fixed_rows::<3>(3 * p) * sa;
                    }
                }
                for (q, eq) in o.elements.iter().enumerate() {
                    let block: Block = w.weight.fixed_view::<3, 3>(3 * p, 3 * q).into_owned();
                    for (a, sa) in ep.nodes(marks) {
                        for (b, sb) in eq.nodes(marks) {
                            let (Some(ua), Some(ub)) = (unknown[a], unknown[b]) else {
                                continue;
                            };
                            // Each term of the upper triangle once; add_off_diagonal
                            // fills in the lower.
                            if ua == ub {
                                normal.add_diagonal(ua, &(block * (sa * sb)));
                            } else if ua < ub {
                                normal.add_off_diagonal(ua, ub, &(block * (sa * sb)));
                            }
                        }
                    }
                }
            }
        }

        // The priors: pseudo-observations that each set-up's error is zero.
        let prior_weights: Vec<Block> = priors
            .iter()
            .map(|prior| spd_inverse(prior).unwrap_or_else(Block::zeros))
            .collect();
        for (s, weight) in prior_weights.iter().enumerate() {
            if let Some(u) = unknown[marks + s] {
                normal.add_diagonal(u, weight);
            }
        }

        let factor = normal.factorise().map_err(|s| {
            let node = free[s.index];
            AdjustError::Singular {
                name: match node.checked_sub(marks) {
                    Some(setup) => format!(
                        "{} (its set-up {setup})",
                        self.name(self.setups()[setup].mark)
                    ),
                    None => self.name(node),
                },
            }
        })?;
        let solution = factor.solve(&rhs);
        let inverse = factor.selected_inverse();
        let mut corrections: Vec<Vector3<f64>> = unknown
            .iter()
            .map(|u| u.map_or_else(Vector3::zeros, |u| solution[u]))
            .collect();

        let inner = if *datum == Datum::Inner {
            // R = Q̃G, with G an identity block for each mark and zero for
            // each set-up, whose errors don't move with the network: three
            // solves.
            let columns: Vec<Vec<Vector3<f64>>> = (0..3)
                .map(|c| {
                    let mut e = Vector3::zeros();
                    e[c] = 1.0;
                    let b: Vec<Vector3<f64>> = free
                        .iter()
                        .map(|&n| if n < marks { e } else { Vector3::zeros() })
                        .collect();
                    factor.solve(&b)
                })
                .collect();
            let mut r = vec![Block::zeros(); nodes];
            for (u, &n) in free.iter().enumerate() {
                r[n] = Block::from_columns(&[columns[0][u], columns[1][u], columns[2][u]]);
            }
            let t: Block = r[..marks].iter().sum();
            let count = marks as f64;
            let mean = corrections[..marks].iter().sum::<Vector3<f64>>() / count;
            for c in &mut corrections[..marks] {
                *c -= mean;
            }
            Some(Inner { r, t, count })
        } else {
            None
        };
        let solution = Solution {
            unknown,
            marks,
            factor,
            inverse,
            inner,
            axes,
        };

        let adjusted_marks = (0..marks)
            .map(|m| AdjustedMark {
                position: approximate[m] + corrections[m],
                cofactor: solution.datum(m, m),
                fixed: held[m] && *datum != Datum::Inner,
            })
            .collect();

        // Statistics.
        let components: usize = self.observations().iter().map(|o| o.elements.len()).sum();
        // Never negative once the factorisation succeeds: N = AᵀPA is then
        // positive definite, so A has at least as many rows as columns. Each
        // set-up's prior adds as many rows as unknowns.
        let free_marks = free.len() - priors.len();
        let degrees_of_freedom = (3 * components).saturating_sub(3 * free_marks);
        let sigma0 = libm::sqrt(options.variance_factor);
        let residuals: Vec<DVector<f64>> = self
            .observations()
            .iter()
            .zip(&misclosures)
            .map(|(o, l)| residual(o, l, &corrections, marks))
            .collect();
        // Ω over every observation and prior first: the tau test needs σ̂₀.
        let shares: Vec<f64> = residuals
            .iter()
            .zip(&weights)
            .map(|(v, w)| v.dot(&(&w.weight * v)))
            .collect();
        let prior_shares: Vec<f64> = prior_weights
            .iter()
            .enumerate()
            .map(|(s, weight)| {
                let error = corrections[marks + s];
                error.dot(&(weight * error))
            })
            .collect();
        let sum_of_squares: f64 = shares.iter().sum::<f64>() + prior_shares.iter().sum::<f64>();
        let variance_factor =
            (degrees_of_freedom > 0).then(|| sum_of_squares / degrees_of_freedom as f64);
        let sigma0_hat = variance_factor
            .filter(|&f| degrees_of_freedom >= 2 && f > 0.0)
            .map(libm::sqrt);

        let b_method = BMethod::new(options.alpha0, options.beta0);
        let (vector_critical, vector_alpha) = b_method.critical(3.0);
        let criteria = Criteria {
            alpha0: options.alpha0,
            beta0: options.beta0,
            lambda0: b_method.lambda0(),
            w: b_method.w_critical(),
            vector: vector_critical,
            vector_alpha,
            tau: stats::tau_critical(options.alpha0, degrees_of_freedom as f64),
            global_alpha: options.global_alpha,
        };
        let global_test = (degrees_of_freedom > 0).then(|| {
            let r = degrees_of_freedom as f64;
            let statistic = sum_of_squares / options.variance_factor;
            let lower = stats::chi2_quantile(options.global_alpha / 2.0, r);
            let upper = stats::chi2_upper_quantile(options.global_alpha / 2.0, r);
            GlobalTest {
                statistic,
                lower,
                upper,
                passed: (lower..=upper).contains(&statistic),
            }
        });

        let context = Context {
            solution: &solution,
            sigma0,
            sigma0_hat,
            lambda0: b_method.lambda0(),
        };
        let observations = self
            .observations()
            .iter()
            .zip(&weights)
            .zip(&residuals)
            .zip(shares)
            .map(|(((o, w), v), share)| ObservationResult {
                scale: options.scale(o.group.as_ref()),
                ..context.observation(o, w, v, share)
            })
            .collect();
        let setups = self
            .setups()
            .iter()
            .enumerate()
            .map(|(s, setup)| {
                context.setup(
                    setup.mark,
                    marks + s,
                    &corrections[marks + s],
                    &priors[s],
                    &prior_weights[s],
                    prior_shares[s],
                )
            })
            .collect();

        Ok(Adjustment {
            frame: self.frame(),
            epoch: self.epoch(),
            datum: datum.clone(),
            marks: adjusted_marks,
            observations,
            statistics: Statistics {
                sum_of_squares,
                degrees_of_freedom,
                variance_factor,
                global_test,
            },
            criteria,
            setups,
            a_priori: options.variance_factor,
            edits: Vec::new(),
            solution,
        })
    }
}

/// An observation's cofactor matrix k C, checked.
pub(crate) fn cofactor(o: &Observation, scale: f64) -> Result<DMatrix<f64>, AdjustError> {
    Weights::new(o, scale).map(|w| w.cofactor)
}

/// An observation's weights: its cofactor matrix Q = k C and P = Q⁻¹.
struct Weights {
    cofactor: DMatrix<f64>,
    weight: DMatrix<f64>,
}

impl Weights {
    /// Checks the covariance and inverts it, scaled by `scale`.
    fn new(o: &Observation, scale: f64) -> Result<Self, AdjustError> {
        let invalid = |reason: String| AdjustError::InvalidObservation {
            id: o.id.clone(),
            reason,
        };
        let size = 3 * o.elements.len();
        let c = &o.covariance;
        if c.nrows() != size || c.ncols() != size {
            return Err(invalid(format!(
                "its covariance is {} × {}, but should be {size} × {size}: three rows and \
                 columns for each element",
                c.nrows(),
                c.ncols(),
            )));
        }
        if c.iter().any(|v| !v.is_finite()) {
            return Err(invalid(
                "its covariance has a value that isn't finite".into(),
            ));
        }
        let largest = (0..size).map(|i| c[(i, i)]).fold(0.0, f64::max);
        for i in 0..size {
            for j in 0..i {
                if (c[(i, j)] - c[(j, i)]).abs() > SYMMETRY_TOLERANCE * largest {
                    return Err(invalid(format!(
                        "its covariance isn't symmetric: ({i}, {j}) is {} but ({j}, {i}) is {}",
                        c[(i, j)],
                        c[(j, i)]
                    )));
                }
            }
        }
        let cofactor = (c + c.transpose()) * (0.5 * scale);
        let Some(cholesky) = cofactor.clone().cholesky() else {
            return Err(invalid("its covariance isn't positive definite".into()));
        };
        let l = cholesky.l_dirty();
        for i in 0..size {
            if l[(i, i)] * l[(i, i)] < MIN_CONDITIONAL_VARIANCE * cofactor[(i, i)] {
                return Err(invalid(format!(
                    "its covariance is singular or nearly so: the {} component of element {} \
                     is fixed by the others",
                    ["X", "Y", "Z"][i % 3],
                    i / 3 + 1
                )));
            }
        }
        let inverse = cholesky.inverse();
        let weight = (&inverse + inverse.transpose()) * 0.5;
        Ok(Weights { cofactor, weight })
    }
}

/// Observed minus computed from the approximate positions, l.
fn misclosure(o: &Observation, approximate: &[Vector3<f64>]) -> DVector<f64> {
    let mut l = DVector::zeros(3 * o.elements.len());
    for (p, element) in o.elements.iter().enumerate() {
        let mut computed = Vector3::zeros();
        for (a, sa) in element.terms() {
            computed += approximate[a] * sa;
        }
        l.fixed_rows_mut::<3>(3 * p)
            .copy_from(&(element.value() - computed));
    }
    l
}

/// Adjusted minus observed, v = A x̂ − l, with the corrections by node.
fn residual(
    o: &Observation,
    l: &DVector<f64>,
    corrections: &[Vector3<f64>],
    marks: usize,
) -> DVector<f64> {
    let mut v = -l;
    for (p, element) in o.elements.iter().enumerate() {
        let mut rows = v.fixed_rows_mut::<3>(3 * p);
        for (a, sa) in element.nodes(marks) {
            rows += corrections[a] * sa;
        }
    }
    v
}

/// The S-transformation to inner constraints from a solution holding one
/// mark: Q = Q̃ − (R_a + R_bᵀ)/m + T/m², with R = Q̃G and T = GᵀQ̃G.
#[derive(Debug, Clone, PartialEq)]
struct Inner {
    r: Vec<Block>,
    t: Block,
    count: f64,
}

/// What the adjustment keeps to give the cofactor matrix of the unknowns,
/// Qₓ, for any pair of nodes: marks, then set-ups.
#[derive(Debug, Clone, PartialEq)]
struct Solution {
    /// Each node's block of unknowns, or `None` for a mark held fixed.
    unknown: Vec<Option<usize>>,
    /// The number of marks: nodes from here on are set-ups.
    marks: usize,
    factor: Factor,
    inverse: SelectedInverse,
    inner: Option<Inner>,
    /// Local east, north and up axes at each mark's approximate position.
    axes: Vec<Matrix3<f64>>,
}

impl Solution {
    /// Block (a, b) with the held marks fixed, for marks that share an
    /// observation, which puts the block on the selected inverse's pattern.
    fn held(&self, a: usize, b: usize) -> Block {
        match (self.unknown[a], self.unknown[b]) {
            (Some(i), Some(j)) => {
                let block = self.inverse.block(i, j);
                debug_assert!(block.is_some(), "({a}, {b}) isn't on the pattern");
                block.unwrap_or_else(Block::zeros)
            }
            _ => Block::zeros(),
        }
    }

    /// Block (a, b) in the datum, for marks that share an observation.
    fn datum(&self, a: usize, b: usize) -> Block {
        self.to_datum(self.held(a, b), a, b)
    }

    /// Block (a, b) in the datum, for any pair of marks.
    fn datum_any(&self, a: usize, b: usize) -> Block {
        let held = match (self.unknown[a], self.unknown[b]) {
            (Some(i), Some(j)) => self
                .inverse
                .block(i, j)
                .unwrap_or_else(|| self.solve_block(i, j)),
            _ => Block::zeros(),
        };
        self.to_datum(held, a, b)
    }

    /// Block (i, j) of N⁻¹, by solving for its column block j.
    fn solve_block(&self, i: usize, j: usize) -> Block {
        let columns: [Vector3<f64>; 3] = std::array::from_fn(|c| {
            let mut unit = vec![Vector3::zeros(); self.factor.size()];
            unit[j][c] = 1.0;
            self.factor.solve(&unit)[i]
        });
        Block::from_columns(&columns)
    }

    /// A block with the held marks fixed, moved to the datum:
    /// Q̃ − (g_b R_a + g_a R_bᵀ)/m + g_a g_b T/m², where g is 1 for a mark and
    /// 0 for a set-up.
    fn to_datum(&self, held: Block, a: usize, b: usize) -> Block {
        let Some(s) = &self.inner else {
            return held;
        };
        match (a < self.marks, b < self.marks) {
            (true, true) => {
                held - (s.r[a] + s.r[b].transpose()) / s.count + s.t / (s.count * s.count)
            }
            (true, false) => held - s.r[b].transpose() / s.count,
            (false, true) => held - s.r[a] / s.count,
            (false, false) => held,
        }
    }
}

/// What every observation's tests need.
struct Context<'a> {
    solution: &'a Solution,
    sigma0: f64,
    sigma0_hat: Option<f64>,
    lambda0: f64,
}

impl Context<'_> {
    /// Tests and reliability for one observation, with residuals v.
    fn observation(
        &self,
        o: &Observation,
        w: &Weights,
        v: &DVector<f64>,
        sum_of_squares: f64,
    ) -> ObservationResult {
        let n = o.elements.len();
        let marks = self.solution.marks;
        // Qᵥᵥ = Q − AQₓAᵀ, from the solution holding marks, which gives the
        // same residuals whatever the datum.
        let mut aqa = DMatrix::zeros(3 * n, 3 * n);
        for (p, ep) in o.elements.iter().enumerate() {
            for (q, eq) in o.elements.iter().enumerate() {
                let mut block = Block::zeros();
                for (a, sa) in ep.nodes(marks) {
                    for (b, sb) in eq.nodes(marks) {
                        block += self.solution.held(a, b) * (sa * sb);
                    }
                }
                aqa.fixed_view_mut::<3, 3>(3 * p, 3 * q).copy_from(&block);
            }
        }
        let qvv = &w.cofactor - aqa;
        let pv = &w.weight * v;
        let redundancy = &qvv * &w.weight;
        let m = &w.weight * &redundancy;
        let mut nodes: Vec<usize> = o
            .elements
            .iter()
            .flat_map(|e| e.nodes(marks).map(|(a, _)| a))
            .collect();
        nodes.sort_unstable();
        nodes.dedup();

        let elements = o
            .elements
            .iter()
            .enumerate()
            .map(|(a, element)| {
                let block = |matrix: &DMatrix<f64>| -> Block {
                    matrix.fixed_view::<3, 3>(3 * a, 3 * a).into_owned()
                };
                let v_a: Vector3<f64> = v.fixed_rows::<3>(3 * a).into_owned();
                let pv_a: Vector3<f64> = pv.fixed_rows::<3>(3 * a).into_owned();
                let (components, vector_test) = self.tests(
                    &self.solution.axes[element.first_mark()],
                    &v_a,
                    &pv_a,
                    &block(&m),
                    &block(&w.weight),
                    &block(&redundancy),
                    |direction| self.shift(o, w, &nodes, a, direction),
                );
                let standardised = std::array::from_fn(|k| {
                    let i = 3 * a + k;
                    (qvv[(i, i)] > MIN_REDUNDANCY * w.cofactor[(i, i)])
                        .then(|| v_a[k] / (self.sigma0 * libm::sqrt(qvv[(i, i)])))
                });
                ElementResult {
                    residual: v_a,
                    components,
                    vector_test,
                    standardised,
                }
            })
            .collect();
        ObservationResult {
            elements,
            sum_of_squares,
            used: true,
            scale: 1.0,
        }
    }

    /// Tests and reliability for a set-up's prior: the pseudo-observation
    /// that its error is zero, with covariance `prior`.
    fn setup(
        &self,
        mark: usize,
        node: usize,
        error: &Vector3<f64>,
        prior: &Block,
        weight: &Block,
        sum_of_squares: f64,
    ) -> SetupResult {
        // The residual is the error itself, and Qᵥᵥ = Σ − Q_δδ.
        let qvv = prior - self.solution.held(node, node);
        let redundancy = qvv * weight;
        let m = weight * redundancy;
        let (components, vector_test) = self.tests(
            &self.solution.axes[mark],
            error,
            &(weight * error),
            &m,
            weight,
            &redundancy,
            // An unused set-up shares no observation with its mark.
            |direction| (self.solution.datum_any(mark, node) * (weight * direction)).norm(),
        );
        SetupResult {
            mark,
            error: *error,
            components,
            vector_test,
            sum_of_squares,
        }
    }

    /// The tests of one three-component part, from its residual v, Pv, and
    /// its blocks of PQᵥᵥP, P and QᵥᵥP, on the local `axes`. `shift` gives
    /// the largest shift of a mark per metre of bias in an ECEF direction.
    #[allow(clippy::too_many_arguments)]
    fn tests(
        &self,
        axes: &Matrix3<f64>,
        v: &Vector3<f64>,
        pv: &Vector3<f64>,
        m: &Block,
        p: &Block,
        redundancy: &Block,
        shift: impl Fn(&Vector3<f64>) -> f64,
    ) -> ([ComponentResult; 3], Option<f64>) {
        let m = (m + m.transpose()) * 0.5;
        let m_enu = axes * m * axes.transpose();
        let p_enu = axes * p * axes.transpose();
        let r_enu = axes * redundancy * axes.transpose();
        let pv_enu = axes * pv;
        let v_enu = axes * v;
        let components: [ComponentResult; 3] = std::array::from_fn(|k| {
            let mkk = m_enu[(k, k)];
            let testable = mkk > MIN_REDUNDANCY * p_enu[(k, k)];
            let spread = libm::sqrt(mkk);
            let mdb = testable.then(|| self.sigma0 * libm::sqrt(self.lambda0 / mkk));
            ComponentResult {
                residual: v_enu[k],
                redundancy: r_enu[(k, k)],
                w: testable.then(|| pv_enu[k] / (self.sigma0 * spread)),
                tau: self
                    .sigma0_hat
                    .filter(|_| testable)
                    .map(|s| pv_enu[k] / (s * spread)),
                mdb,
                shift: mdb.map(|mdb| shift(&axes.row(k).transpose()) * mdb),
            }
        });
        let vector_test = components
            .iter()
            .all(|c| c.w.is_some())
            .then(|| spd_inverse(&m))
            .flatten()
            .map(|m_inverse| pv.dot(&(m_inverse * pv)) / (self.sigma0 * self.sigma0));
        (components, vector_test)
    }

    /// The largest shift of the observation's own marks, per metre of bias
    /// along `direction` (ECEF) in element `a`: Qₓ AᵀP c, over the
    /// observation's nodes.
    fn shift(
        &self,
        o: &Observation,
        w: &Weights,
        nodes: &[usize],
        a: usize,
        direction: &Vector3<f64>,
    ) -> f64 {
        let marks = self.solution.marks;
        let g = w.weight.columns(3 * a, 3) * direction;
        let mut h = vec![Vector3::zeros(); nodes.len()];
        for (p, element) in o.elements.iter().enumerate() {
            let g_p: Vector3<f64> = g.fixed_rows::<3>(3 * p).into_owned();
            for (node, sign) in element.nodes(marks) {
                if let Ok(i) = nodes.binary_search(&node) {
                    h[i] += g_p * sign;
                }
            }
        }
        nodes
            .iter()
            .filter(|&&m| m < marks)
            .map(|&m| {
                let mut moved = Vector3::zeros();
                for (&b, h_b) in nodes.iter().zip(&h) {
                    moved += self.solution.datum(m, b) * h_b;
                }
                moved.norm()
            })
            .fold(0.0, f64::max)
    }
}
