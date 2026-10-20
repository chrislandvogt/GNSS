//! Loop closures and repeat baselines (ARCHITECTURE.md section 10.9): a
//! screen of the vectors before adjusting, and an aid to explaining the
//! adjustment's results. They don't replace its tests.
//!
//! A spanning tree of the marks, built breadth first along the vectors in
//! order, leaves each other vector to close exactly one loop: a fundamental
//! cycle basis. Repeated vectors over the same pair of marks are compared
//! with the first instead. Each misclosure w is tested as wᵀΣ_w⁻¹w/σ₀²
//! against χ²(3), at the vector test's critical value, with Σ_w from the
//! vectors' covariances, including their correlations within a cluster,
//! and the priors of any set-ups whose errors don't cancel.

use std::collections::{BTreeMap, VecDeque};

use nalgebra::{Matrix3, Vector3};

use crate::error::AdjustError;
use crate::network::{Element, Network, Options};
use crate::sparse::spd_inverse;
use crate::stats::BMethod;

/// A vector around a loop: (observation, element, direction), with direction
/// +1 along the vector and −1 against it.
pub type Leg = (usize, usize, f64);

/// The misclosure of one loop.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopClosure {
    /// The vectors around the loop, in order.
    pub legs: Vec<Leg>,
    /// The marks around the loop, in order, from its first.
    pub marks: Vec<usize>,
    /// The misclosure, the sum of the vectors around the loop, in ECEF metres.
    pub misclosure: Vector3<f64>,
    /// The misclosure on local east, north and up axes at the first mark.
    pub misclosure_enu: Vector3<f64>,
    /// The loop's length, the sum of its vectors' lengths, in metres.
    pub length: f64,
    /// The misclosure's size in parts per million of the length.
    pub ppm: f64,
    /// wᵀΣ_w⁻¹w/σ₀², which is χ²(3) if the vectors are right.
    pub statistic: f64,
    /// True if the statistic is within the critical value.
    pub passed: bool,
}

/// A repeated vector compared with the first over the same pair of marks.
#[derive(Debug, Clone, PartialEq)]
pub struct RepeatBaseline {
    /// The first vector over the pair, as (observation, element).
    pub first: (usize, usize),
    /// The repeat, as (observation, element).
    pub repeat: (usize, usize),
    /// +1 if the repeat runs the same way as the first, −1 if reversed.
    pub direction: f64,
    /// The repeat, turned to the first's direction, minus the first, in
    /// ECEF metres.
    pub difference: Vector3<f64>,
    /// The difference on local east, north and up axes at the first
    /// vector's start.
    pub difference_enu: Vector3<f64>,
    /// The first vector's length, in metres.
    pub length: f64,
    /// The difference's size in parts per million of the length.
    pub ppm: f64,
    /// dᵀΣ_d⁻¹d/σ₀², which is χ²(3) if both vectors are right.
    pub statistic: f64,
    /// True if the statistic is within the critical value.
    pub passed: bool,
}

/// Every loop closure and repeat baseline in a network.
#[derive(Debug, Clone, PartialEq)]
pub struct Closures {
    /// The fundamental loops, one for each vector off the spanning tree.
    pub loops: Vec<LoopClosure>,
    /// Each repeat compared with the first vector over its pair of marks.
    pub repeats: Vec<RepeatBaseline>,
    /// The critical value of both tests: the vector test's, by the B-method.
    pub critical: f64,
}

/// A vector, as an edge of the graph of marks.
struct Edge {
    observation: usize,
    element: usize,
    from: usize,
    to: usize,
    vector: Vector3<f64>,
    setups: [Option<usize>; 2],
}

impl Edge {
    /// Adds the edge's set-ups to a combination's coefficients, for the edge
    /// taken `sign` times: its end's error counts positive, its start's
    /// negative.
    fn add_setups(&self, sign: f64, coefficients: &mut BTreeMap<usize, f64>) {
        for (setup, end) in self.setups.iter().zip([-1.0, 1.0]) {
            if let Some(s) = *setup {
                *coefficients.entry(s).or_insert(0.0) += sign * end;
            }
        }
    }
}

impl Network {
    /// Loop closures and repeat baselines, with each observation's
    /// covariance scaled as in `options`.
    pub fn closures(&self, options: &Options) -> Result<Closures, AdjustError> {
        options.check()?;
        self.check_observations()?;
        let cofactors = self
            .observations()
            .iter()
            .map(|o| crate::adjustment::cofactor(o, options.scale(o.group.as_ref())))
            .collect::<Result<Vec<_>, _>>()?;
        let approximate = self.approximate_positions()?;
        let ellipsoid = self.frame().ellipsoid();
        let axes = |mark: usize| -> Result<Matrix3<f64>, AdjustError> {
            ellipsoid
                .ecef_to_geodetic(&approximate[mark])
                .map(|g| g.enu_rotation())
                .map_err(|source| AdjustError::MarkPosition {
                    name: self.name(mark),
                    source,
                })
        };
        let critical = BMethod::new(options.alpha0, options.beta0).critical(3.0).0;
        // Each set-up's prior covariance, in ECEF.
        let priors = self
            .setups()
            .iter()
            .map(|setup| {
                let (h, v) = (setup.sigma_horizontal, setup.sigma_vertical);
                let enu = Matrix3::from_diagonal(&Vector3::new(h * h, h * h, v * v));
                axes(setup.mark).map(|r| r.transpose() * enu * r)
            })
            .collect::<Result<Vec<_>, _>>()?;
        // The covariance a combination of set-up errors adds.
        let setup_spread = |coefficients: &BTreeMap<usize, f64>| -> Matrix3<f64> {
            coefficients
                .iter()
                .map(|(&s, &c)| priors[s] * (c * c))
                .sum()
        };

        let edges: Vec<Edge> = self
            .observations()
            .iter()
            .enumerate()
            .flat_map(|(observation, o)| {
                o.elements
                    .iter()
                    .enumerate()
                    .filter_map(move |(element, e)| match *e {
                        Element::Difference {
                            from,
                            to,
                            vector,
                            setups,
                        } => Some(Edge {
                            observation,
                            element,
                            from,
                            to,
                            vector,
                            setups,
                        }),
                        Element::Position { .. } => None,
                    })
            })
            .collect();
        // The covariance between two vectors: their block of the
        // observation's covariance if they're in the same one, else zero.
        let covariance = |a: &Edge, b: &Edge| -> Matrix3<f64> {
            if a.observation == b.observation {
                cofactors[a.observation]
                    .fixed_view::<3, 3>(3 * a.element, 3 * b.element)
                    .into_owned()
            } else {
                Matrix3::zeros()
            }
        };
        let test = |w: &Vector3<f64>, spread: &Matrix3<f64>| {
            let statistic = spd_inverse(spread).map_or(f64::INFINITY, |inverse| {
                w.dot(&(inverse * w)) / options.variance_factor
            });
            (statistic, statistic <= critical)
        };

        // Repeats: every later vector over a pair of marks, against the first.
        let mut pairs: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
        for (i, e) in edges.iter().enumerate() {
            pairs
                .entry((e.from.min(e.to), e.from.max(e.to)))
                .or_default()
                .push(i);
        }
        let mut repeats = Vec::new();
        for indices in pairs.values() {
            let first = &edges[indices[0]];
            for &j in &indices[1..] {
                let repeat = &edges[j];
                let direction = if repeat.from == first.from { 1.0 } else { -1.0 };
                let difference = repeat.vector * direction - first.vector;
                let mut coefficients = BTreeMap::new();
                repeat.add_setups(direction, &mut coefficients);
                first.add_setups(-1.0, &mut coefficients);
                let spread = covariance(repeat, repeat) + covariance(first, first)
                    - (covariance(repeat, first) + covariance(first, repeat)) * direction
                    + setup_spread(&coefficients);
                let (statistic, passed) = test(&difference, &spread);
                let length = first.vector.norm();
                repeats.push(RepeatBaseline {
                    first: (first.observation, first.element),
                    repeat: (repeat.observation, repeat.element),
                    direction,
                    difference,
                    difference_enu: axes(first.from)? * difference,
                    length,
                    ppm: difference.norm() / length * 1e6,
                    statistic,
                    passed,
                });
            }
        }

        // The spanning tree, over the first vector of each pair: each mark's
        // parent, depth, and the vector joining them with its direction
        // from parent to child.
        let marks = self.marks().len();
        let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); marks];
        for indices in pairs.values() {
            let e = &edges[indices[0]];
            neighbours[e.from].push(indices[0]);
            neighbours[e.to].push(indices[0]);
        }
        for list in &mut neighbours {
            list.sort_unstable();
        }
        let mut parent: Vec<Option<(usize, usize, f64)>> = vec![None; marks];
        let mut depth = vec![0_usize; marks];
        let mut visited = vec![false; marks];
        let mut tree_edge = vec![false; edges.len()];
        for root in 0..marks {
            if visited[root] {
                continue;
            }
            visited[root] = true;
            let mut queue = VecDeque::from([root]);
            while let Some(m) = queue.pop_front() {
                for &i in &neighbours[m] {
                    let e = &edges[i];
                    let (other, direction) = if e.from == m {
                        (e.to, 1.0)
                    } else {
                        (e.from, -1.0)
                    };
                    if !visited[other] {
                        visited[other] = true;
                        parent[other] = Some((m, i, direction));
                        depth[other] = depth[m] + 1;
                        tree_edge[i] = true;
                        queue.push_back(other);
                    }
                }
            }
        }

        // Each vector off the tree closes a loop: along it from a to b, then
        // back through the tree from b to a.
        let mut loops = Vec::new();
        for indices in pairs.values() {
            let i = indices[0];
            if tree_edge[i] {
                continue;
            }
            let e = &edges[i];
            let (mut up, mut down) = (e.to, e.from);
            let mut b_side = Vec::new();
            let mut a_side = Vec::new();
            while up != down {
                let climb_b = depth[up] >= depth[down];
                let node = if climb_b { up } else { down };
                let Some((p, t, direction)) = parent[node] else {
                    break;
                };
                if climb_b {
                    // From child to parent, against the tree direction.
                    b_side.push((t, -direction, p));
                    up = p;
                } else {
                    a_side.push((t, direction, node));
                    down = p;
                }
            }
            let mut legs = vec![(i, 1.0)];
            let mut loop_marks = vec![e.from, e.to];
            for &(t, direction, p) in &b_side {
                legs.push((t, direction));
                loop_marks.push(p);
            }
            for &(t, direction, child) in a_side.iter().rev() {
                legs.push((t, direction));
                loop_marks.push(child);
            }
            // The walk ends where it started.
            loop_marks.pop();

            let misclosure: Vector3<f64> = legs.iter().map(|&(k, s)| edges[k].vector * s).sum();
            let mut spread = Matrix3::zeros();
            let mut coefficients = BTreeMap::new();
            for &(k, sk) in &legs {
                for &(l, sl) in &legs {
                    spread += covariance(&edges[k], &edges[l]) * (sk * sl);
                }
                edges[k].add_setups(sk, &mut coefficients);
            }
            // A set-up's error cancels around the loop when the loop arrives
            // and leaves through it.
            spread += setup_spread(&coefficients);
            let (statistic, passed) = test(&misclosure, &spread);
            let length: f64 = legs.iter().map(|&(k, _)| edges[k].vector.norm()).sum();
            loops.push(LoopClosure {
                legs: legs
                    .iter()
                    .map(|&(k, s)| (edges[k].observation, edges[k].element, s))
                    .collect(),
                marks: loop_marks,
                misclosure,
                misclosure_enu: axes(e.from)? * misclosure,
                length,
                ppm: misclosure.norm() / length * 1e6,
                statistic,
                passed,
            });
        }
        Ok(Closures {
            loops,
            repeats,
            critical,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::Observation;
    use crate::synthetic::{Spec, Synthetic};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn clustered() -> Synthetic {
        Synthetic::generate(&Spec {
            marks: 25,
            neighbours: 4,
            cluster: 3,
            base_sigma_m: 0.004,
            seed: 61,
            ..Spec::default()
        })
        .unwrap()
    }

    #[test]
    fn exact_vectors_close_and_every_independent_loop_is_found() {
        let synthetic = clustered();
        let network = synthetic.exact();
        let closures = network.closures(&Options::default()).unwrap();
        // The cyclomatic number: vectors − marks + 1 for a connected network.
        let vectors: usize = network
            .observations()
            .iter()
            .map(|o| {
                o.elements
                    .iter()
                    .filter(|e| matches!(e, Element::Difference { .. }))
                    .count()
            })
            .sum();
        assert_eq!(closures.loops.len(), vectors - network.marks().len() + 1);
        assert!(closures.repeats.is_empty());
        for l in &closures.loops {
            assert!(l.misclosure.norm() < 1e-8, "{l:?}");
            assert!(l.passed);
            // The legs join up: each starts where the last ended.
            assert_eq!(l.legs.len(), l.marks.len());
            assert!(l.length > 0.0);
        }
    }

    #[test]
    fn loop_tests_are_chi_squared_with_cluster_correlations() {
        // Clusters share their base's error, which cancels in some loops; the
        // test is χ²(3) only if their correlations are used.
        let synthetic = clustered();
        let draws = 100;
        let mut means = Vec::new();
        for seed in 0..draws {
            let closures = synthetic
                .observe(300 + seed)
                .closures(&Options::default())
                .unwrap();
            let total: f64 = closures.loops.iter().map(|l| l.statistic).sum();
            means.push(total / closures.loops.len() as f64);
        }
        let n = means.len() as f64;
        let mean = means.iter().sum::<f64>() / n;
        let variance = means.iter().map(|m| (m - mean) * (m - mean)).sum::<f64>() / (n - 1.0);
        let standard_error = libm::sqrt(variance / n);
        assert!(
            (mean - 3.0).abs() < 4.0 * standard_error,
            "mean {mean} ± {standard_error}"
        );
    }

    #[test]
    fn a_blunder_fails_the_loops_through_it() {
        let mut network = clustered().observe(5);
        let up = Vector3::new(0.0, 0.0, 0.08);
        let from = match network.observations()[4].elements[0] {
            Element::Difference { from, .. } => from,
            Element::Position { .. } => unreachable!(),
        };
        let approximate = network.approximate_positions().unwrap();
        let axes = network
            .frame()
            .ellipsoid()
            .ecef_to_geodetic(&approximate[from])
            .unwrap()
            .enu_rotation();
        if let Element::Difference { vector, .. } = &mut network.observations_mut()[4].elements[0] {
            *vector += axes.transpose() * up;
        }
        let closures = network.closures(&Options::default()).unwrap();
        let through: Vec<&LoopClosure> = closures
            .loops
            .iter()
            .filter(|l| l.legs.iter().any(|&(o, e, _)| (o, e) == (4, 0)))
            .collect();
        assert!(!through.is_empty());
        for l in &through {
            assert!(!l.passed, "{l:?}");
            // About 8 cm up, either way round.
            assert!((l.misclosure_enu.z.abs() - 0.08).abs() < 0.04, "{l:?}");
        }
    }

    #[test]
    fn repeats_are_compared_either_way_round() {
        let synthetic = clustered();
        let mut network = synthetic.observe(6);
        // The pair of marks of an existing vector.
        let Element::Difference { from: a, to: b, .. } = network.observations()[0].elements[0]
        else {
            unreachable!()
        };
        let covariance = Matrix3::from_diagonal(&Vector3::new(4e-5, 4e-5, 1.6e-4));
        let truth = synthetic.truth[b] - synthetic.truth[a];
        // Two repeats: one reversed and good, one forward with 10 cm in it.
        let good = -truth + Vector3::new(0.002, -0.001, 0.003);
        network.add_observation(Observation::baseline("again", b, a, good, covariance));
        let bad = truth + Vector3::new(0.1, 0.0, 0.0);
        network.add_observation(Observation::baseline("blunder", a, b, bad, covariance));
        let closures = network.closures(&Options::default()).unwrap();
        let find = |id: &str| {
            closures
                .repeats
                .iter()
                .find(|r| network.observations()[r.repeat.0].id == id)
                .unwrap()
        };
        let (again, blunder) = (find("again"), find("blunder"));
        assert_eq!(again.first, (0, 0));
        assert!(again.passed && !blunder.passed, "{again:?} {blunder:?}");
        assert_eq!((again.direction, blunder.direction), (-1.0, 1.0));
        assert!((blunder.difference.norm() - 0.1).abs() < 0.03);
        assert!(blunder.ppm > 1.0);
    }
}
