//! Synthetic networks with known truth, for tests and benchmarks.
//!
//! Marks are scattered over a disc and joined to their nearest neighbours by
//! GNSS vectors, some of them measured together from a common base. Noise is
//! drawn from each observation's own covariance, so an adjustment's
//! statistics should follow theory exactly. Everything comes from seeds, with
//! the same numbers on every platform.

use std::collections::BTreeSet;
use std::f64::consts::PI;

use gnss_datum::{FrameId, GRS80, Geodetic};
use nalgebra::{DMatrix, DVector, Matrix3, Vector3};

use crate::error::AdjustError;
use crate::network::{Network, Observation};

/// The shape of a synthetic network.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    /// The number of marks.
    pub marks: usize,
    /// Each mark is joined by a vector to this many of its nearest marks.
    pub neighbours: usize,
    /// The radius of the disc the marks are scattered over, in metres.
    pub radius_m: f64,
    /// The centre of the disc: latitude and longitude, in degrees.
    pub centre_deg: (f64, f64),
    /// The number of vectors measured together from one base, and
    /// correlated through it. 1 for independent baselines.
    pub cluster: usize,
    /// A vector's horizontal standard deviation is this, in metres…
    pub sigma_m: f64,
    /// …plus this many parts per million of its length.
    pub ppm: f64,
    /// Vertical standard deviations, as a multiple of horizontal ones.
    pub vertical_factor: f64,
    /// The horizontal standard deviation of an error each base shares with
    /// every vector measured from it, in metres.
    pub base_sigma_m: f64,
    /// The number of marks with control: a point observation each.
    pub control: usize,
    /// Control's horizontal standard deviation, in metres.
    pub control_sigma_m: f64,
    /// The standard deviation of each coordinate of the approximate
    /// positions, in metres.
    pub approximate_sigma_m: f64,
    /// The number of groups the vectors are dealt into, in turn, named
    /// "group-1" and so on, for variance component estimation. 0 for none.
    /// Control is in no group.
    pub groups: usize,
    /// The seed for the marks' positions and the choice of control.
    pub seed: u64,
}

impl Default for Spec {
    /// Twenty marks within 10 km of Canberra, each joined to four
    /// neighbours by independent baselines of 3 mm + 1 ppm horizontally,
    /// twice that vertically, with three control marks good to 1 cm.
    fn default() -> Self {
        Spec {
            marks: 20,
            neighbours: 4,
            radius_m: 10_000.0,
            centre_deg: (-35.3, 149.1),
            cluster: 1,
            sigma_m: 0.003,
            ppm: 1.0,
            vertical_factor: 2.0,
            base_sigma_m: 0.002,
            control: 3,
            control_sigma_m: 0.01,
            approximate_sigma_m: 2.0,
            groups: 0,
            seed: 1,
        }
    }
}

/// A synthetic network and its truth.
#[derive(Debug, Clone, PartialEq)]
pub struct Synthetic {
    /// The marks' true positions.
    pub truth: Vec<Vector3<f64>>,
    /// The marks with control.
    pub control: Vec<usize>,
    exact: Network,
}

impl Synthetic {
    /// A network of the given shape, in GDA2020.
    pub fn generate(spec: &Spec) -> Result<Self, AdjustError> {
        let mut rng = Rng::new(spec.seed);
        let lat0 = spec.centre_deg.0.to_radians();
        let lon0 = spec.centre_deg.1.to_radians();
        let meridian = GRS80.meridian_radius_m(lat0);
        let parallel = GRS80.prime_vertical_radius_m(lat0) * libm::cos(lat0);
        let mut truth = Vec::with_capacity(spec.marks);
        let mut axes = Vec::with_capacity(spec.marks);
        for _ in 0..spec.marks {
            let r = spec.radius_m * libm::sqrt(rng.uniform(0.0, 1.0));
            let (sin, cos) = libm::sincos(2.0 * PI * rng.uniform(0.0, 1.0));
            let at = Geodetic::new(
                lat0 + r * sin / meridian,
                lon0 + r * cos / parallel,
                rng.uniform(50.0, 350.0),
            )?;
            truth.push(GRS80.geodetic_to_ecef(&at));
            axes.push(at.enu_rotation());
        }

        let edges = nearest_neighbours(&truth, spec.neighbours);
        let mut network = Network::new(FrameId::Gda2020, None)?;
        for (m, x) in truth.iter().enumerate() {
            let offset = Vector3::from_fn(|_, _| rng.gaussian() * spec.approximate_sigma_m);
            network.add_mark(format!("M{:04}", m + 1), Some(x + offset))?;
        }
        let enu = |sigma: f64| {
            let vertical = sigma * spec.vertical_factor;
            Matrix3::from_diagonal(&Vector3::new(
                sigma * sigma,
                sigma * sigma,
                vertical * vertical,
            ))
        };
        // Clusters: each base's vectors, in groups of `cluster`.
        let mut base = usize::MAX;
        let mut pending: Vec<usize> = Vec::new();
        let edges: Vec<(usize, usize)> = edges.into_iter().collect();
        for (k, &(from, to)) in edges.iter().enumerate() {
            if from != base {
                base = from;
                pending.clear();
            }
            pending.push(to);
            let last = edges.get(k + 1).is_none_or(|&(next, _)| next != from);
            if pending.len() == spec.cluster.max(1) || last {
                let r = axes[from];
                let shared = r.transpose() * enu(spec.base_sigma_m) * r;
                let n = pending.len();
                let mut covariance = DMatrix::zeros(3 * n, 3 * n);
                let mut baselines = Vec::with_capacity(n);
                for (i, &to) in pending.iter().enumerate() {
                    let vector = truth[to] - truth[from];
                    let sigma = spec.sigma_m + spec.ppm * 1e-6 * vector.norm();
                    let own = r.transpose() * enu(sigma) * r;
                    for j in 0..n {
                        let block = if i == j { own + shared } else { shared };
                        covariance
                            .fixed_view_mut::<3, 3>(3 * i, 3 * j)
                            .copy_from(&block);
                    }
                    baselines.push((from, to, vector));
                }
                let index = network.observations().len();
                let id = format!("{}-{}", network.marks()[from].name, index + 1);
                let mut observation = Observation::baseline_cluster(id, &baselines, covariance);
                if spec.groups > 0 {
                    observation =
                        observation.with_group(format!("group-{}", index % spec.groups + 1));
                }
                network.add_observation(observation);
                pending.clear();
            }
        }

        let mut control = Vec::new();
        while control.len() < spec.control.min(spec.marks) {
            let m = rng.below(spec.marks);
            if !control.contains(&m) {
                control.push(m);
            }
        }
        control.sort_unstable();
        for &m in &control {
            let covariance = axes[m].transpose() * enu(spec.control_sigma_m) * axes[m];
            let id = format!("control-{}", network.marks()[m].name);
            network.add_observation(Observation::point(id, m, truth[m], covariance));
        }
        Ok(Synthetic {
            truth,
            control,
            exact: network,
        })
    }

    /// The network with exact observations.
    pub fn exact(&self) -> &Network {
        &self.exact
    }

    /// The network observed with noise drawn from each observation's
    /// covariance, from `seed`.
    pub fn observe(&self, seed: u64) -> Network {
        self.observe_with(seed, |_| 1.0)
    }

    /// The network observed with noise drawn from each observation's
    /// covariance times `scale(observation)`, from `seed`: observations
    /// whose covariances are too optimistic or too pessimistic.
    pub fn observe_with(&self, seed: u64, scale: impl Fn(&Observation) -> f64) -> Network {
        let mut rng = Rng::new(seed);
        let mut network = self.exact.clone();
        for o in network.observations_mut() {
            let Some(cholesky) = o.covariance.clone().cholesky() else {
                continue;
            };
            let z = DVector::from_fn(o.covariance.nrows(), |_, _| rng.gaussian());
            let noise = cholesky.l() * z * libm::sqrt(scale(o));
            for (p, element) in o.elements.iter_mut().enumerate() {
                element.perturb(&noise.fixed_rows::<3>(3 * p).into_owned());
            }
        }
        network
    }
}

/// Each mark joined to its `k` nearest, as (lower index, higher index), then
/// the parts joined by their closest marks until the network is connected.
fn nearest_neighbours(points: &[Vector3<f64>], k: usize) -> BTreeSet<(usize, usize)> {
    let n = points.len();
    let mut edges = BTreeSet::new();
    for i in 0..n {
        let mut others: Vec<(f64, usize)> = (0..n)
            .filter(|&j| j != i)
            .map(|j| ((points[j] - points[i]).norm(), j))
            .collect();
        others.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for &(_, j) in others.iter().take(k) {
            edges.insert((i.min(j), i.max(j)));
        }
    }
    loop {
        // The part containing mark 0.
        let mut adjacent = vec![Vec::new(); n];
        for &(i, j) in &edges {
            adjacent[i].push(j);
            adjacent[j].push(i);
        }
        let mut joined = vec![false; n];
        let mut stack = vec![0];
        while let Some(a) = stack.pop() {
            if a < n && !joined[a] {
                joined[a] = true;
                stack.extend(adjacent[a].iter().filter(|&&b| !joined[b]));
            }
        }
        let mut closest: Option<(f64, usize, usize)> = None;
        for i in (0..n).filter(|&i| joined[i]) {
            for j in (0..n).filter(|&j| !joined[j]) {
                let d = (points[j] - points[i]).norm();
                if closest.is_none_or(|(best, _, _)| d < best) {
                    closest = Some((d, i, j));
                }
            }
        }
        match closest {
            Some((_, i, j)) => {
                edges.insert((i.min(j), i.max(j)));
            }
            None => return edges,
        }
    }
}

/// SplitMix64, for the same pseudo-random numbers on every platform.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [low, high).
    pub(crate) fn uniform(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * ((self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64)
    }

    /// Uniform in 0..n.
    pub(crate) fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// Standard normal, by the Box–Muller transform.
    pub(crate) fn gaussian(&mut self) -> f64 {
        // In (0, 1], so the logarithm is finite.
        let u = 1.0 - self.uniform(0.0, 1.0);
        let angle = 2.0 * PI * self.uniform(0.0, 1.0);
        libm::sqrt(-2.0 * libm::log(u)) * libm::cos(angle)
    }
}
