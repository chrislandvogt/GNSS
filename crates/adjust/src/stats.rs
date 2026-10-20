//! Probability distributions for the adjustment's tests.
//!
//! Critical values decide which observations are flagged, so they're computed
//! here with `libm` rather than taken from a statistics library: the same bits
//! on every platform (D-024). The tests check every function against SciPy.

use std::f64::consts::{PI, SQRT_2};

/// Below this, a continued fraction's denominators are nudged off zero
/// (Lentz's method).
const TINY: f64 = 1e-300;

/// A bound on series and continued fraction terms. They converge in about
/// the square root of the degrees of freedom, so this is never reached in
/// practice.
const MAX_TERMS: usize = 100_000;

/// The standard normal distribution function Φ(x).
pub fn normal_cdf(x: f64) -> f64 {
    0.5 * libm::erfc(-x / SQRT_2)
}

/// The standard normal quantile Φ⁻¹(p), for 0 < p < 1.
///
/// Acklam's rational approximation, accurate to a relative 1.2e-9, refined by
/// one step of Halley's method, which takes it to within rounding.
pub fn normal_quantile(p: f64) -> f64 {
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [
        7.784_695_709_041_462e-3,
        3.224_671_290_700_398e-1,
        2.445_134_137_142_996,
        3.754_408_661_907_416,
    ];
    const LOW: f64 = 0.024_25;

    if p > 0.5 {
        // 1 − p is exact here, and the refinement is accurate in the lower tail.
        return -normal_quantile(1.0 - p);
    }
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    let x = if p < LOW {
        let q = libm::sqrt(-2.0 * libm::log(p));
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    };
    let u = (normal_cdf(x) - p) * libm::sqrt(2.0 * PI) * libm::exp(x * x / 2.0);
    x - u / (1.0 + x * u / 2.0)
}

/// The regularised incomplete gamma functions P(a, x) and Q(a, x) = 1 − P(a, x).
///
/// Each comes from whichever of the series for P and the continued fraction
/// for Q converges faster, and the other by subtraction, so the smaller tail
/// keeps its relative precision (Numerical Recipes, section 6.2).
fn incomplete_gamma(a: f64, x: f64) -> (f64, f64) {
    if x <= 0.0 {
        return (0.0, 1.0);
    }
    let front = libm::exp(a * libm::log(x) - x - libm::lgamma(a));
    if x < a + 1.0 {
        let mut term = 1.0 / a;
        let mut sum = term;
        let mut n = a;
        for _ in 0..MAX_TERMS {
            n += 1.0;
            term *= x / n;
            sum += term;
            if term < sum * f64::EPSILON {
                break;
            }
        }
        let p = sum * front;
        (p, 1.0 - p)
    } else {
        // Modified Lentz's method.
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / TINY;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..MAX_TERMS {
            let i = i as f64;
            let an = -i * (i - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < TINY {
                d = TINY;
            }
            c = b + an / c;
            if c.abs() < TINY {
                c = TINY;
            }
            d = 1.0 / d;
            let delta = d * c;
            h *= delta;
            if (delta - 1.0).abs() < f64::EPSILON {
                break;
            }
        }
        let q = front * h;
        (1.0 - q, q)
    }
}

/// P(χ²(k) ≤ x).
pub fn chi2_cdf(x: f64, k: f64) -> f64 {
    incomplete_gamma(k / 2.0, x / 2.0).0
}

/// P(χ²(k) > x), accurate in the upper tail.
pub fn chi2_sf(x: f64, k: f64) -> f64 {
    incomplete_gamma(k / 2.0, x / 2.0).1
}

/// The lower quantile of χ²(k): x with P(χ²(k) ≤ x) = p.
pub fn chi2_quantile(p: f64, k: f64) -> f64 {
    let hi = bracket(|x| chi2_cdf(x, k) < p, k);
    bisect(0.0, hi, |x| chi2_cdf(x, k) < p)
}

/// The upper quantile of χ²(k): x with P(χ²(k) > x) = q.
pub fn chi2_upper_quantile(q: f64, k: f64) -> f64 {
    let hi = bracket(|x| chi2_sf(x, k) > q, k);
    bisect(0.0, hi, |x| chi2_sf(x, k) > q)
}

/// P(χ²(k, λ) > x) for the non-central χ² distribution with non-centrality
/// λ: a Poisson mixture of central χ² distributions with k + 2j degrees of
/// freedom.
pub fn noncentral_chi2_sf(x: f64, k: f64, lambda: f64) -> f64 {
    if lambda <= 0.0 {
        return chi2_sf(x, k);
    }
    let half = lambda / 2.0;
    let log_half = libm::log(half);
    let mut sum = 0.0;
    for j in 0..MAX_TERMS {
        let j = j as f64;
        let weight = libm::exp(j * log_half - half - libm::lgamma(j + 1.0));
        sum += weight * chi2_sf(x, k + 2.0 * j);
        // Past the mode the weights fall faster than geometrically.
        if j > half && weight < 1e-18 {
            break;
        }
    }
    sum
}

/// The regularised incomplete beta function I_x(a, b) (Numerical Recipes,
/// section 6.4).
fn incomplete_beta(x: f64, a: f64, b: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let front = libm::exp(
        libm::lgamma(a + b) - libm::lgamma(a) - libm::lgamma(b)
            + a * libm::log(x)
            + b * libm::log1p(-x),
    );
    if x < (a + 1.0) / (a + b + 2.0) {
        front * beta_fraction(x, a, b) / a
    } else {
        1.0 - front * beta_fraction(1.0 - x, b, a) / b
    }
}

/// The continued fraction for the incomplete beta function, by the modified
/// Lentz method.
fn beta_fraction(x: f64, a: f64, b: f64) -> f64 {
    let nudge = |v: f64| if v.abs() < TINY { TINY } else { v };
    let mut c = 1.0;
    let mut d = 1.0 / nudge(1.0 - (a + b) * x / (a + 1.0));
    let mut h = d;
    for m in 1..MAX_TERMS {
        let m = m as f64;
        let even = m * (b - m) * x / ((a - 1.0 + 2.0 * m) * (a + 2.0 * m));
        d = 1.0 / nudge(1.0 + even * d);
        c = nudge(1.0 + even / c);
        h *= d * c;
        let odd = -(a + m) * (a + b + m) * x / ((a + 2.0 * m) * (a + 1.0 + 2.0 * m));
        d = 1.0 / nudge(1.0 + odd * d);
        c = nudge(1.0 + odd / c);
        let delta = d * c;
        h *= delta;
        if (delta - 1.0).abs() < f64::EPSILON {
            break;
        }
    }
    h
}

/// P(T > t) for Student's t distribution with ν degrees of freedom, t ≥ 0.
pub fn student_t_sf(t: f64, nu: f64) -> f64 {
    0.5 * incomplete_beta(nu / (nu + t * t), nu / 2.0, 0.5)
}

/// The upper quantile of Student's t distribution: t with P(T > t) = q, for
/// 0 < q ≤ 0.5.
pub fn student_t_upper_quantile(q: f64, nu: f64) -> f64 {
    let hi = bracket(|t| student_t_sf(t, nu) > q, 1.0);
    bisect(0.0, hi, |t| student_t_sf(t, nu) > q)
}

/// A point at or above `start` where `below` is false, by doubling.
fn bracket(below: impl Fn(f64) -> bool, start: f64) -> f64 {
    let mut hi = start.max(1.0);
    while below(hi) && hi < f64::MAX / 2.0 {
        hi *= 2.0;
    }
    hi
}

/// The boundary between `lo`, where `below` is true, and `hi`, where it's
/// false, by bisection to adjacent floating-point numbers. The functions
/// here are monotonic, so this always converges, in a fixed number of steps
/// for the same inputs.
fn bisect(mut lo: f64, mut hi: f64, below: impl Fn(f64) -> bool) -> f64 {
    loop {
        let mid = lo + (hi - lo) / 2.0;
        if mid <= lo || mid >= hi {
            return mid;
        }
        if below(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
}

/// Critical values by Baarda's B-method: every test is set to detect a bias
/// of the same size, the non-centrality λ₀, with the same power β₀.
///
/// The one-dimensional w-test has level α₀, so λ₀ = (z(1 − α₀/2) + z(β₀))². A
/// test with k degrees of freedom then gets the critical value c with
/// P(χ²(k, λ₀) > c) = β₀, and its level is P(χ²(k) > c).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BMethod {
    alpha0: f64,
    beta0: f64,
    lambda0: f64,
}

impl BMethod {
    /// The B-method for a w-test level α₀ and power β₀.
    pub fn new(alpha0: f64, beta0: f64) -> Self {
        let z = -normal_quantile(alpha0 / 2.0) + normal_quantile(beta0);
        BMethod {
            alpha0,
            beta0,
            lambda0: z * z,
        }
    }

    /// The level of the w-test, α₀.
    pub fn alpha0(&self) -> f64 {
        self.alpha0
    }

    /// The power, β₀.
    pub fn beta0(&self) -> f64 {
        self.beta0
    }

    /// The non-centrality parameter, λ₀.
    pub fn lambda0(&self) -> f64 {
        self.lambda0
    }

    /// The w-test's critical value, z(1 − α₀/2).
    pub fn w_critical(&self) -> f64 {
        -normal_quantile(self.alpha0 / 2.0)
    }

    /// The critical value and level of a test with `k` degrees of freedom.
    pub fn critical(&self, k: f64) -> (f64, f64) {
        let sf = |c| noncentral_chi2_sf(c, k, self.lambda0);
        let hi = bracket(|c| sf(c) > self.beta0, k + self.lambda0);
        let critical = bisect(0.0, hi, |c| sf(c) > self.beta0);
        (critical, chi2_sf(critical, k))
    }
}

/// The critical value of Pope's tau test, for redundancy `r` and level `alpha`
/// (two-tailed): τ = √r t / √(r − 1 + t²), with t from Student's t
/// distribution with r − 1 degrees of freedom. `None` for r < 2.
pub fn tau_critical(alpha: f64, r: f64) -> Option<f64> {
    if r < 2.0 {
        return None;
    }
    let t = student_t_upper_quantile(alpha / 2.0, r - 1.0);
    Some(libm::sqrt(r) * t / libm::sqrt(r - 1.0 + t * t))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[track_caller]
    fn close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance * expected.abs(),
            "{actual} vs {expected}"
        );
    }

    // Reference values from SciPy 1.16 (scipy.stats).

    #[test]
    fn normal_quantiles_match_scipy() {
        close(normal_quantile(0.9995), 3.290_526_731_491_925_5, 1e-14);
        close(normal_quantile(0.8), 0.841_621_233_572_914_3, 1e-14);
        close(normal_quantile(1e-10), -6.361_340_902_404_056, 1e-14);
        assert_eq!(normal_quantile(0.5), 0.0);
        for p in [1e-200, 1e-20, 0.01, 0.2, 0.4999, 0.6, 0.99] {
            close(normal_cdf(normal_quantile(p)), p, 1e-13);
        }
    }

    #[test]
    fn chi2_matches_scipy() {
        close(chi2_cdf(3.0, 3.0), 0.608_374_823_728_910_9, 1e-13);
        close(chi2_cdf(0.5, 1.0), 0.520_499_877_813_046_6, 1e-13);
        close(chi2_cdf(150.0, 120.0), 0.966_926_519_088_695_3, 1e-13);
        close(chi2_cdf(2.0, 10.0), 0.003_659_846_827_343_713, 1e-12);
        close(chi2_upper_quantile(0.05, 1.0), 3.841_458_820_694_124, 1e-12);
        close(
            chi2_upper_quantile(0.001, 1.0),
            10.827_566_170_662_733,
            1e-12,
        );
        close(chi2_upper_quantile(0.05, 3.0), 7.814_727_903_251_179, 1e-12);
        close(
            chi2_upper_quantile(0.025, 10.0),
            20.483_177_350_807_388,
            1e-12,
        );
        close(chi2_quantile(0.025, 10.0), 3.246_972_780_236_841_3, 1e-12);
        close(chi2_quantile(0.5, 2.0), 1.386_294_361_119_891, 1e-12);
        close(chi2_quantile(0.025, 100.0), 74.221_927_474_923_73, 1e-12);
        close(
            chi2_upper_quantile(0.025, 100.0),
            129.561_197_185_836_6,
            1e-12,
        );
        close(
            chi2_upper_quantile(0.025, 1000.0),
            1_089.530_912_774_913_5,
            1e-12,
        );
    }

    #[test]
    fn b_method_matches_scipy() {
        let b = BMethod::new(0.001, 0.8);
        close(b.lambda0(), 17.074_646_805_189_502, 1e-13);
        close(b.w_critical(), 3.290_526_731_491_925_5, 1e-14);
        close(
            noncentral_chi2_sf(10.0, 3.0, b.lambda0()),
            0.894_266_325_858_296_6,
            1e-12,
        );
        for (k, critical, alpha) in [
            (1.0, 10.827_566_170_664_29, 0.001),
            (3.0, 12.633_477_721_224_287, 0.005_500_158_038_997_691),
            (10.0, 18.987_146_895_498_27, 0.040_426_240_301_637_03),
            (100.0, 103.114_107_790_531_88, 0.395_505_659_175_619_7),
        ] {
            let (c, a) = b.critical(k);
            close(c, critical, 1e-10);
            close(a, alpha, 1e-9);
        }
    }

    #[test]
    fn student_t_matches_scipy() {
        close(
            student_t_upper_quantile(0.025, 10.0),
            2.228_138_851_986_274,
            1e-12,
        );
        close(
            student_t_upper_quantile(0.0005, 5.0),
            6.868_826_625_881_276,
            1e-12,
        );
        close(
            student_t_upper_quantile(0.005, 100.0),
            2.625_890_521_438_017_3,
            1e-12,
        );
        // Pope's tau: √r t / √(r − 1 + t²), whose square is below r.
        let tau = tau_critical(0.05, 11.0).unwrap();
        let t = 2.228_138_851_986_274;
        close(tau, libm::sqrt(11.0) * t / libm::sqrt(10.0 + t * t), 1e-12);
        assert!(tau_critical(0.05, 1.0).is_none());
    }
}
