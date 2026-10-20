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

use gnss_datum::DatumError;
use nalgebra::{DMatrix, DVector, Matrix3, Vector3};

use crate::error::AdjustError;
use crate::network::{Datum, Network, Observation, Options};
use crate::sparse::{Block, BlockMatrix, SelectedInverse, spd_inverse};
use crate::stats::{self, BMethod};

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
    datum: Datum,
    marks: Vec<AdjustedMark>,
    observations: Vec<ObservationResult>,
    statistics: Statistics,
    criteria: Criteria,
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
    /// The datum.
    pub fn datum(&self) -> &Datum {
        &self.datum
    }

    /// The adjusted marks, in the network's order.
    pub fn marks(&self) -> &[AdjustedMark] {
        &self.marks
    }

    /// The results for each observation, in the network's order.
    pub fn observations(&self) -> &[ObservationResult] {
        &self.observations
    }

    /// The overall statistics.
    pub fn statistics(&self) -> &Statistics {
        &self.statistics
    }

    /// The critical values.
    pub fn criteria(&self) -> &Criteria {
        &self.criteria
    }
}

impl Network {
    /// Adjusts the network in `datum`.
    pub fn adjust(&self, datum: &Datum, options: &Options) -> Result<Adjustment, AdjustError> {
        options.check()?;
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

        // A block of unknowns per mark not held.
        let marks = self.marks().len();
        let free: Vec<usize> = (0..marks).filter(|&m| !held[m]).collect();
        let mut unknown = vec![None; marks];
        for (u, &m) in free.iter().enumerate() {
            unknown[m] = Some(u);
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
                for (a, sa) in ep.terms() {
                    if let Some(ua) = unknown[a] {
                        rhs[ua] += pl.fixed_rows::<3>(3 * p) * sa;
                    }
                }
                for (q, eq) in o.elements.iter().enumerate() {
                    let block: Block = w.weight.fixed_view::<3, 3>(3 * p, 3 * q).into_owned();
                    for (a, sa) in ep.terms() {
                        for (b, sb) in eq.terms() {
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

        let factor = normal.factorise().map_err(|s| AdjustError::Singular {
            name: self.name(free[s.index]),
        })?;
        let solution = factor.solve(&rhs);
        let inverse = factor.selected_inverse();
        let mut corrections: Vec<Vector3<f64>> = unknown
            .iter()
            .map(|u| u.map_or_else(Vector3::zeros, |u| solution[u]))
            .collect();

        let inner = if *datum == Datum::Inner {
            // R = Q̃G, with G = [I I … I]ᵀ: three solves.
            let columns: Vec<Vec<Vector3<f64>>> = (0..3)
                .map(|c| {
                    let mut e = Vector3::zeros();
                    e[c] = 1.0;
                    factor.solve(&vec![e; free.len()])
                })
                .collect();
            let mut r = vec![Block::zeros(); marks];
            for (u, &m) in free.iter().enumerate() {
                r[m] = Block::from_columns(&[columns[0][u], columns[1][u], columns[2][u]]);
            }
            let t: Block = r.iter().sum();
            let count = marks as f64;
            let mean = corrections.iter().sum::<Vector3<f64>>() / count;
            for c in &mut corrections {
                *c -= mean;
            }
            Some(Inner { r, t, count })
        } else {
            None
        };
        let cofactors = Cofactors {
            unknown: &unknown,
            inverse: &inverse,
            inner,
        };

        let adjusted_marks = (0..marks)
            .map(|m| AdjustedMark {
                position: approximate[m] + corrections[m],
                cofactor: cofactors.datum(m, m),
                fixed: held[m] && *datum != Datum::Inner,
            })
            .collect();

        // Statistics.
        let components: usize = self.observations().iter().map(|o| o.elements.len()).sum();
        // Never negative once the factorisation succeeds: N = AᵀPA is then
        // positive definite, so A has at least as many rows as columns.
        let degrees_of_freedom = (3 * components).saturating_sub(3 * free.len());
        let sigma0 = libm::sqrt(options.variance_factor);
        let residuals: Vec<DVector<f64>> = self
            .observations()
            .iter()
            .zip(&misclosures)
            .map(|(o, l)| residual(o, l, &corrections))
            .collect();
        // Ω over every observation first: the tau test needs σ̂₀.
        let shares: Vec<f64> = residuals
            .iter()
            .zip(&weights)
            .map(|(v, w)| v.dot(&(&w.weight * v)))
            .collect();
        let sum_of_squares: f64 = shares.iter().sum();
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
            cofactors: &cofactors,
            axes: &axes,
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
            .map(|(((o, w), v), share)| context.observation(o, w, v, share))
            .collect();

        Ok(Adjustment {
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
        })
    }
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

/// Adjusted minus observed, v = A x̂ − l.
fn residual(o: &Observation, l: &DVector<f64>, corrections: &[Vector3<f64>]) -> DVector<f64> {
    let mut v = -l;
    for (p, element) in o.elements.iter().enumerate() {
        let mut rows = v.fixed_rows_mut::<3>(3 * p);
        for (a, sa) in element.terms() {
            rows += corrections[a] * sa;
        }
    }
    v
}

/// The S-transformation to inner constraints from a solution holding one
/// mark: Q = Q̃ − (R_a + R_bᵀ)/m + T/m², with R = Q̃G and T = GᵀQ̃G.
struct Inner {
    r: Vec<Block>,
    t: Block,
    count: f64,
}

/// Blocks of the cofactor matrix of the positions.
struct Cofactors<'a> {
    unknown: &'a [Option<usize>],
    inverse: &'a SelectedInverse,
    inner: Option<Inner>,
}

impl Cofactors<'_> {
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

    /// Block (a, b) in the datum.
    fn datum(&self, a: usize, b: usize) -> Block {
        let q = self.held(a, b);
        match &self.inner {
            None => q,
            Some(s) => q - (s.r[a] + s.r[b].transpose()) / s.count + s.t / (s.count * s.count),
        }
    }
}

/// What every observation's tests need.
struct Context<'a> {
    cofactors: &'a Cofactors<'a>,
    axes: &'a [Matrix3<f64>],
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
        // Qᵥᵥ = Q − AQₓAᵀ, from the solution holding marks, which gives the
        // same residuals whatever the datum.
        let mut aqa = DMatrix::zeros(3 * n, 3 * n);
        for (p, ep) in o.elements.iter().enumerate() {
            for (q, eq) in o.elements.iter().enumerate() {
                let mut block = Block::zeros();
                for (a, sa) in ep.terms() {
                    for (b, sb) in eq.terms() {
                        block += self.cofactors.held(a, b) * (sa * sb);
                    }
                }
                aqa.fixed_view_mut::<3, 3>(3 * p, 3 * q).copy_from(&block);
            }
        }
        let qvv = &w.cofactor - aqa;
        let pv = &w.weight * v;
        let redundancy = &qvv * &w.weight;
        let m = &w.weight * &redundancy;
        let mut marks: Vec<usize> = o
            .elements
            .iter()
            .flat_map(|e| e.terms().map(|(a, _)| a))
            .collect();
        marks.sort_unstable();
        marks.dedup();

        let elements = o
            .elements
            .iter()
            .enumerate()
            .map(|(a, element)| {
                let axes = self.axes[element.first_mark()];
                let m_aa: Block = m.fixed_view::<3, 3>(3 * a, 3 * a).into_owned();
                let m_aa = (m_aa + m_aa.transpose()) * 0.5;
                let m_enu = axes * m_aa * axes.transpose();
                let p_enu = axes * w.weight.fixed_view::<3, 3>(3 * a, 3 * a) * axes.transpose();
                let r_enu = axes * redundancy.fixed_view::<3, 3>(3 * a, 3 * a) * axes.transpose();
                let pv_a: Vector3<f64> = pv.fixed_rows::<3>(3 * a).into_owned();
                let pv_enu = axes * pv_a;
                let v_a: Vector3<f64> = v.fixed_rows::<3>(3 * a).into_owned();
                let v_enu = axes * v_a;
                let components = std::array::from_fn(|k| {
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
                        shift: mdb.map(|mdb| {
                            let direction: Vector3<f64> = axes.row(k).transpose();
                            self.shift(o, w, &marks, a, &direction) * mdb
                        }),
                    }
                });
                let vector_test = components
                    .iter()
                    .all(|c: &ComponentResult| c.w.is_some())
                    .then(|| spd_inverse(&m_aa))
                    .flatten()
                    .map(|m_inverse| pv_a.dot(&(m_inverse * pv_a)) / (self.sigma0 * self.sigma0));
                ElementResult {
                    residual: v_a,
                    components,
                    vector_test,
                }
            })
            .collect();
        ObservationResult {
            elements,
            sum_of_squares,
        }
    }

    /// The largest shift of the observation's own marks, per metre of bias
    /// along `direction` (ECEF) in element `a`: Qₓ AᵀP c.
    fn shift(
        &self,
        o: &Observation,
        w: &Weights,
        marks: &[usize],
        a: usize,
        direction: &Vector3<f64>,
    ) -> f64 {
        let g = w.weight.columns(3 * a, 3) * direction;
        let mut h = vec![Vector3::zeros(); marks.len()];
        for (p, element) in o.elements.iter().enumerate() {
            let g_p: Vector3<f64> = g.fixed_rows::<3>(3 * p).into_owned();
            for (mark, sign) in element.terms() {
                if let Ok(i) = marks.binary_search(&mark) {
                    h[i] += g_p * sign;
                }
            }
        }
        marks
            .iter()
            .map(|&m| {
                let mut moved = Vector3::zeros();
                for (&b, h_b) in marks.iter().zip(&h) {
                    moved += self.cofactors.datum(m, b) * h_b;
                }
                moved.norm()
            })
            .fold(0.0, f64::max)
    }
}
