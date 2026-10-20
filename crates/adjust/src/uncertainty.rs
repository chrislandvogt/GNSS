//! Uncertainty of adjusted positions: error ellipses and ellipsoids, and the
//! 95% measures of positional and relative uncertainty (PU and RU) from the
//! ICSM *Guideline for Adjustment and Evaluation of Survey Control* (SP1,
//! ARCHITECTURE.md section 10.8).

use nalgebra::{Matrix2, Matrix3, Vector3};

/// √χ²₂(0.95) = √(−2 ln 0.05): the 95% error ellipse's scale.
const ELLIPSE_95: f64 = 2.447_746_830_680_816;

/// √χ²₃(0.95): the 95% error ellipsoid's scale.
const ELLIPSOID_95: f64 = 2.795_483_482_915_107_4;

/// The vertical 95% uncertainty is this many standard deviations (SP1).
const VERTICAL_95: f64 = 1.96;

/// Which variance factor turns cofactors into covariances. The choice is a
/// project policy, recorded with the results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scaling {
    /// The a priori variance factor σ₀², as given in the options.
    APriori,
    /// The a posteriori variance factor σ̂₀², estimated by the adjustment.
    APosteriori,
}

/// An error ellipse on the local horizontal plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ellipse {
    /// The semi-major axis, in metres.
    pub semi_major: f64,
    /// The semi-minor axis, in metres.
    pub semi_minor: f64,
    /// The azimuth of the semi-major axis, in radians clockwise from north,
    /// from 0 to π.
    pub azimuth: f64,
}

impl Ellipse {
    /// The standard ellipse of a covariance on east and north axes.
    pub fn from_covariance(en: &Matrix2<f64>) -> Self {
        let (ee, nn, en) = (en[(0, 0)], en[(1, 1)], 0.5 * (en[(0, 1)] + en[(1, 0)]));
        let mean = 0.5 * (ee + nn);
        let spread = libm::hypot(0.5 * (ee - nn), en);
        let mut azimuth = 0.5 * libm::atan2(2.0 * en, nn - ee);
        if azimuth < 0.0 {
            azimuth += std::f64::consts::PI;
        }
        Ellipse {
            semi_major: libm::sqrt(mean + spread),
            // Rounding can take a degenerate ellipse's minor variance just below zero.
            semi_minor: libm::sqrt((mean - spread).max(0.0)),
            azimuth,
        }
    }

    /// The 95% ellipse: the standard one scaled by √χ²₂(0.95) ≈ 2.4477.
    pub fn at_95(&self) -> Self {
        Ellipse {
            semi_major: self.semi_major * ELLIPSE_95,
            semi_minor: self.semi_minor * ELLIPSE_95,
            azimuth: self.azimuth,
        }
    }

    /// The radius of the circle holding 95% of the probability, by the
    /// approximation in the ICSM guideline: a(1.960790 + 0.004071c +
    /// 0.114276c² + 0.371625c³), with c = b/a. This is SP1's horizontal PU or RU.
    pub fn circular_95(&self) -> f64 {
        if self.semi_major <= 0.0 {
            return 0.0;
        }
        let c = self.semi_minor / self.semi_major;
        self.semi_major * (1.960_790 + c * (0.004_071 + c * (0.114_276 + c * 0.371_625)))
    }
}

/// An error ellipsoid on local east, north and up axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ellipsoid {
    /// The semi-axes, in metres, largest first.
    pub semi_axes: Vector3<f64>,
    /// The axes' directions, as columns of east, north and up components, in
    /// the same order. Each points to whichever side has its largest
    /// component positive.
    pub axes: Matrix3<f64>,
}

impl Ellipsoid {
    /// The standard ellipsoid of a covariance on east, north and up axes.
    pub fn from_covariance(enu: &Matrix3<f64>) -> Self {
        let symmetric = (enu + enu.transpose()) * 0.5;
        let eigen = symmetric.symmetric_eigen();
        let mut order = [0, 1, 2];
        order.sort_by(|&i, &j| eigen.eigenvalues[j].total_cmp(&eigen.eigenvalues[i]));
        let mut semi_axes = Vector3::zeros();
        let mut axes = Matrix3::zeros();
        for (k, &i) in order.iter().enumerate() {
            semi_axes[k] = libm::sqrt(eigen.eigenvalues[i].max(0.0));
            let mut axis: Vector3<f64> = eigen.eigenvectors.column(i).into_owned();
            let largest = axis
                .iter()
                .fold(0.0_f64, |m, &v| if v.abs() > m.abs() { v } else { m });
            if largest < 0.0 {
                axis = -axis;
            }
            axes.set_column(k, &axis);
        }
        Ellipsoid { semi_axes, axes }
    }

    /// The 95% ellipsoid: the standard one scaled by √χ²₃(0.95) ≈ 2.7955.
    pub fn at_95(&self) -> Self {
        Ellipsoid {
            semi_axes: self.semi_axes * ELLIPSOID_95,
            axes: self.axes,
        }
    }
}

/// The uncertainty of a position, or of one position relative to another,
/// on local east, north and up axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Uncertainty {
    /// The covariance on local east, north and up axes, in m².
    pub enu: Matrix3<f64>,
    /// The standard horizontal error ellipse.
    pub ellipse: Ellipse,
    /// The standard error ellipsoid.
    pub ellipsoid: Ellipsoid,
    /// The vertical standard deviation, in metres.
    pub sigma_up: f64,
    /// The horizontal 95% uncertainty, in metres: [`Ellipse::circular_95`].
    pub horizontal_95: f64,
    /// The vertical 95% uncertainty, in metres: 1.96 σ_up.
    pub vertical_95: f64,
}

impl Uncertainty {
    /// The uncertainty of a covariance on local east, north and up axes.
    pub fn from_enu(enu: Matrix3<f64>) -> Self {
        let enu = (enu + enu.transpose()) * 0.5;
        let ellipse = Ellipse::from_covariance(&enu.fixed_view::<2, 2>(0, 0).into_owned());
        let sigma_up = libm::sqrt(enu[(2, 2)].max(0.0));
        Uncertainty {
            enu,
            ellipse,
            ellipsoid: Ellipsoid::from_covariance(&enu),
            sigma_up,
            horizontal_95: ellipse.circular_95(),
            vertical_95: VERTICAL_95 * sigma_up,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[track_caller]
    fn close(a: f64, b: f64, tolerance: f64) {
        assert!(
            (a - b).abs() <= tolerance * b.abs().max(1e-300),
            "{a} vs {b}"
        );
    }

    #[test]
    fn the_ellipse_matches_geodepy() {
        // GeodePy 0.7.0's error_ellipse (east, north order): a, b and the
        // azimuth of the major axis.
        let en = Matrix2::new(4e-4, 1.5e-4, 1.5e-4, 1e-4);
        let e = Ellipse::from_covariance(&en);
        close(e.semi_major, 0.021_497_256_437_879_794, 1e-14);
        close(e.semi_minor, 0.006_153_695_283_651_585, 1e-13);
        close(e.azimuth.to_degrees(), 67.5, 1e-13);
        // East is the major axis: azimuth 90°. North: 0°.
        let east = Ellipse::from_covariance(&Matrix2::new(4.0, 0.0, 0.0, 1.0));
        assert_eq!((east.semi_major, east.semi_minor), (2.0, 1.0));
        close(east.azimuth, std::f64::consts::FRAC_PI_2, 1e-15);
        assert_eq!(
            Ellipse::from_covariance(&Matrix2::new(1.0, 0.0, 0.0, 4.0)).azimuth,
            0.0
        );
        let at_95 = east.at_95();
        close(at_95.semi_major, 2.0 * 2.447_746_830_680_816, 1e-15);
    }

    #[test]
    fn circular_95_matches_geodepy() {
        // GeodePy 0.7.0's circ_hz_pu.
        for (a, b, expected) in [
            (0.02, 0.01, 0.040_756_952_500_000_006),
            (0.015, 0.015, 0.036_761_429_999_999_99),
            (0.03, 0.0, 0.058_823_7),
            (0.012, 0.004, 0.023_863_298_666_666_668),
        ] {
            let e = Ellipse {
                semi_major: a,
                semi_minor: b,
                azimuth: 0.0,
            };
            close(e.circular_95(), expected, 1e-14);
        }
        let point = Ellipse {
            semi_major: 0.0,
            semi_minor: 0.0,
            azimuth: 0.0,
        };
        assert_eq!(point.circular_95(), 0.0);
    }

    #[test]
    fn the_ellipsoid_has_the_covariance_eigenstructure() {
        let enu = Matrix3::new(4e-4, 1.5e-4, 2e-5, 1.5e-4, 1e-4, -1e-5, 2e-5, -1e-5, 9e-4);
        let e = Ellipsoid::from_covariance(&enu);
        assert!(e.semi_axes[0] >= e.semi_axes[1] && e.semi_axes[1] >= e.semi_axes[2]);
        // Q = V diag(s²) Vᵀ, with V orthonormal.
        let s2 = Matrix3::from_diagonal(&e.semi_axes.component_mul(&e.semi_axes));
        assert!((e.axes * s2 * e.axes.transpose() - enu).abs().max() < 1e-18);
        assert!(
            (e.axes.transpose() * e.axes - Matrix3::identity())
                .abs()
                .max()
                < 1e-14
        );
        // Mostly vertical first.
        assert!(e.axes[(2, 0)] > 0.99);
        close(
            e.at_95().semi_axes[0],
            e.semi_axes[0] * 2.795_483_482_915_107_4,
            1e-15,
        );
    }

    #[test]
    fn uncertainty_gives_sp1_measures() {
        let enu = Matrix3::new(4e-4, 1.5e-4, 0.0, 1.5e-4, 1e-4, 0.0, 0.0, 0.0, 9e-4);
        let u = Uncertainty::from_enu(enu);
        close(u.sigma_up, 0.03, 1e-15);
        close(u.vertical_95, 0.0588, 1e-15);
        close(u.horizontal_95, u.ellipse.circular_95(), 0.0);
    }
}
