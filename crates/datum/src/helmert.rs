//! Helmert (similarity) transformations: 7 parameters, and 14 with rates of change.

use std::f64::consts::PI;

use nalgebra::{Matrix3, Vector3};

use crate::epoch::CoordinateEpoch;

/// One millimetre, in metres.
pub const MM: f64 = 1e-3;
/// One part per billion, as a dimensionless scale.
pub const PPB: f64 = 1e-9;
/// One milliarcsecond, in radians.
pub const MAS: f64 = PI / 648_000_000.0;

/// How a Helmert transformation's rotations are signed.
///
/// The two conventions differ only in the sign of the rotations, and
/// publishers use both: the IERS (ITRF) uses position vector rotations, and
/// the GDA2020 Technical Manual uses coordinate frame rotations. Applying
/// parameters in the wrong convention gives errors of tens of millimetres, so
/// every set of parameters carries its convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationConvention {
    /// The position vector is rotated (EPSG methods 1033 and 1053; IERS).
    PositionVector,
    /// The coordinate axes are rotated (EPSG methods 1032 and 1056; GDA2020 Technical Manual).
    CoordinateFrame,
}

/// The seven parameters of a similarity transformation, in SI units.
///
/// Also used for their rates of change (per year) and for one-sigma
/// uncertainties, which have the same shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HelmertParameters {
    /// Translation along X, Y and Z, in metres.
    pub translation_m: Vector3<f64>,
    /// Rotation about X, Y and Z, in radians.
    pub rotation_rad: Vector3<f64>,
    /// Scale difference: coordinates are multiplied by `1 + scale`.
    pub scale: f64,
}

impl HelmertParameters {
    /// All seven parameters zero.
    pub fn zero() -> Self {
        Self {
            translation_m: Vector3::zeros(),
            rotation_rad: Vector3::zeros(),
            scale: 0.0,
        }
    }

    /// Parameters from the units transformations are usually published in:
    /// millimetres, milliarcseconds and parts per billion.
    pub fn from_mm_mas_ppb(
        translation_mm: [f64; 3],
        rotation_mas: [f64; 3],
        scale_ppb: f64,
    ) -> Self {
        Self {
            translation_m: Vector3::from(translation_mm) * MM,
            rotation_rad: Vector3::from(rotation_mas) * MAS,
            scale: scale_ppb * PPB,
        }
    }

    fn negated(&self) -> Self {
        Self {
            translation_m: -self.translation_m,
            rotation_rad: -self.rotation_rad,
            scale: -self.scale,
        }
    }
}

/// A 7-parameter similarity (Helmert) transformation of ECEF coordinates.
///
/// `X' = T + (1 + s) R X`, with `R` the small-angle rotation matrix in the
/// given convention. This linearised form is how EPSG, the IERS and the
/// GDA2020 Technical Manual define these transformations; it's also PROJ's
/// default. Rotations of a few milliarcseconds make the omitted terms smaller
/// than a micrometre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Helmert7 {
    /// The parameters.
    pub parameters: HelmertParameters,
    /// One-sigma uncertainties of the parameters, if published.
    pub sigmas: Option<HelmertParameters>,
    /// The sign convention of the rotations.
    pub convention: RotationConvention,
}

impl Helmert7 {
    /// The small-angle rotation matrix `R`.
    fn rotation(&self) -> Matrix3<f64> {
        let r = self.parameters.rotation_rad;
        let coordinate_frame = Matrix3::new(1.0, r.z, -r.y, -r.z, 1.0, r.x, r.y, -r.x, 1.0);
        match self.convention {
            RotationConvention::CoordinateFrame => coordinate_frame,
            RotationConvention::PositionVector => coordinate_frame.transpose(),
        }
    }

    /// Transforms ECEF coordinates, in metres.
    pub fn apply(&self, xyz: &Vector3<f64>) -> Vector3<f64> {
        self.parameters.translation_m + self.rotation() * xyz * (1.0 + self.parameters.scale)
    }

    /// The Jacobian of [`apply`](Self::apply) with respect to the coordinates: `(1 + s) R`.
    pub fn jacobian(&self) -> Matrix3<f64> {
        self.rotation() * (1.0 + self.parameters.scale)
    }

    /// The covariance (m²) that the parameters' uncertainties add to transformed coordinates.
    ///
    /// Zero if no uncertainties are published. The parameters are treated as
    /// uncorrelated, as their publishers give them (after Harvey 1998,
    /// "Practical least squares and statistics for surveyors", section 8.7.2).
    /// This error is common to every point transformed with the same
    /// parameters, so it doesn't affect vectors between them.
    pub fn parameter_covariance(&self, xyz: &Vector3<f64>) -> Matrix3<f64> {
        let Some(sigmas) = self.sigmas else {
            return Matrix3::zeros();
        };
        let k = 1.0 + self.parameters.scale;
        let sign = match self.convention {
            RotationConvention::CoordinateFrame => 1.0,
            RotationConvention::PositionVector => -1.0,
        };
        let (x, y, z) = (xyz.x, xyz.y, xyz.z);
        // Partial derivatives of X' with respect to each parameter, and that parameter's sigma.
        let columns = [
            (Vector3::x(), sigmas.translation_m.x),
            (Vector3::y(), sigmas.translation_m.y),
            (Vector3::z(), sigmas.translation_m.z),
            (self.rotation() * xyz, sigmas.scale),
            (Vector3::new(0.0, z, -y) * (sign * k), sigmas.rotation_rad.x),
            (Vector3::new(-z, 0.0, x) * (sign * k), sigmas.rotation_rad.y),
            (Vector3::new(y, -x, 0.0) * (sign * k), sigmas.rotation_rad.z),
        ];
        columns.iter().fold(Matrix3::zeros(), |acc, (d, sigma)| {
            acc + d * d.transpose() * (sigma * sigma)
        })
    }

    /// The reverse transformation, by changing the sign of every parameter.
    ///
    /// This is how EPSG defines the reverse of these methods. It's exact to
    /// the order of the linearisation: well under a micrometre.
    pub fn inverse(&self) -> Self {
        Self {
            parameters: self.parameters.negated(),
            sigmas: self.sigmas,
            convention: self.convention,
        }
    }
}

/// A 14-parameter (time-dependent) Helmert transformation.
///
/// The parameters at a coordinate epoch `t` are `p(t) = p0 + ṗ (t − t0)`,
/// and their uncertainties `σ(t) = √(σ0² + (σṗ (t − t0))²)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Helmert14 {
    /// The parameters at the reference epoch.
    pub parameters: HelmertParameters,
    /// Rates of change of the parameters, per year.
    pub rates: HelmertParameters,
    /// The reference epoch, t0.
    pub reference_epoch: CoordinateEpoch,
    /// One-sigma uncertainties of the parameters at the reference epoch, if published.
    pub sigmas: Option<HelmertParameters>,
    /// One-sigma uncertainties of the rates, if published.
    pub rate_sigmas: Option<HelmertParameters>,
    /// The sign convention of the rotations.
    pub convention: RotationConvention,
}

impl Helmert14 {
    /// The 7-parameter transformation at a coordinate epoch.
    pub fn at(&self, epoch: CoordinateEpoch) -> Helmert7 {
        let dt = epoch.years_since(self.reference_epoch);
        let p = &self.parameters;
        let r = &self.rates;
        let parameters = HelmertParameters {
            translation_m: p.translation_m + r.translation_m * dt,
            rotation_rad: p.rotation_rad + r.rotation_rad * dt,
            scale: p.scale + r.scale * dt,
        };
        let sigmas = match (self.sigmas, self.rate_sigmas) {
            (None, None) => None,
            (s, rs) => {
                let s = s.unwrap_or_else(HelmertParameters::zero);
                let rs = rs.unwrap_or_else(HelmertParameters::zero);
                let grow = |a: f64, b: f64| libm::sqrt(a * a + (b * dt) * (b * dt));
                Some(HelmertParameters {
                    translation_m: s.translation_m.zip_map(&rs.translation_m, grow),
                    rotation_rad: s.rotation_rad.zip_map(&rs.rotation_rad, grow),
                    scale: grow(s.scale, rs.scale),
                })
            }
        };
        Helmert7 {
            parameters,
            sigmas,
            convention: self.convention,
        }
    }

    /// The reverse transformation, by changing the sign of every parameter and rate.
    pub fn inverse(&self) -> Self {
        Self {
            parameters: self.parameters.negated(),
            rates: self.rates.negated(),
            ..*self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn gda94_to_gda2020() -> Helmert7 {
        Helmert7 {
            parameters: HelmertParameters::from_mm_mas_ppb(
                [61.55, -10.87, -40.19],
                [-39.4924, -32.7221, -32.8979],
                -9.994,
            ),
            sigmas: None,
            convention: RotationConvention::CoordinateFrame,
        }
    }

    #[test]
    fn conventions_differ_only_in_rotation_sign() {
        let cf = gda94_to_gda2020();
        let mut pv = cf;
        pv.convention = RotationConvention::PositionVector;
        pv.parameters.rotation_rad = -pv.parameters.rotation_rad;
        let x = Vector3::new(-4052051.7643, 4212836.2017, -2545106.0245);
        assert!((cf.apply(&x) - pv.apply(&x)).norm() < 1e-9);
    }

    #[test]
    fn inverse_undoes_the_transformation() {
        let h = gda94_to_gda2020();
        let x = Vector3::new(-4052051.7643, 4212836.2017, -2545106.0245);
        // The small-angle form is exact only to second order: about 1e-7 m here.
        assert!((h.inverse().apply(&h.apply(&x)) - x).norm() < 1e-6);
    }

    #[test]
    fn jacobian_matches_finite_differences() {
        let h = gda94_to_gda2020();
        let x = Vector3::new(-4052051.7643, 4212836.2017, -2545106.0245);
        let j = h.jacobian();
        for col in 0..3 {
            let mut moved = x;
            moved[col] += 1.0;
            let numeric = h.apply(&moved) - h.apply(&x);
            assert!((numeric - j.column(col)).norm() < 1e-8);
        }
    }

    #[test]
    fn rates_move_parameters_and_grow_sigmas() {
        let h = Helmert14 {
            parameters: HelmertParameters::zero(),
            rates: HelmertParameters::from_mm_mas_ppb([0.0; 3], [1.50379, 1.18346, 1.20716], 0.0),
            reference_epoch: CoordinateEpoch::from_const(2020.0),
            sigmas: None,
            rate_sigmas: Some(HelmertParameters::from_mm_mas_ppb(
                [0.0; 3],
                [0.00417, 0.00401, 0.00370],
                0.0,
            )),
            convention: RotationConvention::CoordinateFrame,
        };
        let at = h.at(CoordinateEpoch::from_const(2018.0));
        assert!((at.parameters.rotation_rad.x / MAS + 2.0 * 1.50379).abs() < 1e-12);
        let sigma = at.sigmas.unwrap().rotation_rad.x / MAS;
        assert!((sigma - 2.0 * 0.00417).abs() < 1e-12);
        let back = h.inverse().at(CoordinateEpoch::from_const(2018.0));
        assert_eq!(back.parameters.rotation_rad, -at.parameters.rotation_rad);
    }

    #[test]
    fn parameter_covariance_of_a_pure_translation_uncertainty() {
        let mut h = gda94_to_gda2020();
        h.sigmas = Some(HelmertParameters::from_mm_mas_ppb(
            [1.0, 2.0, 3.0],
            [0.0; 3],
            0.0,
        ));
        let c = h.parameter_covariance(&Vector3::new(1.0, 2.0, 3.0));
        let expected = Matrix3::from_diagonal(&Vector3::new(1e-6, 4e-6, 9e-6));
        assert!((c - expected).abs().max() < 1e-18);
    }
}
