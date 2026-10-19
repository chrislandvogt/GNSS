//! Positions: ECEF coordinates that always carry their frame and epoch.

use std::fmt;

use nalgebra::{Matrix3, Vector3};

use crate::epoch::CoordinateEpoch;
use crate::error::DatumError;
use crate::frame::{FrameId, FrameKind};
use crate::geodetic::Geodetic;

/// Relative tolerance for a covariance matrix's symmetry.
const SYMMETRY_TOLERANCE: f64 = 1e-9;

/// A 3×3 covariance matrix of ECEF coordinates, in m².
///
/// Construction checks that the matrix is finite, symmetric (to a relative
/// 1e-9, after which it's made exactly symmetric) and positive semi-definite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Covariance(Matrix3<f64>);

impl Covariance {
    /// A covariance from an ECEF matrix in m².
    pub fn new(matrix: Matrix3<f64>) -> Result<Self, DatumError> {
        let invalid = |reason: String| Err(DatumError::InvalidCovariance { reason });
        if matrix.iter().any(|v| !v.is_finite()) {
            return invalid("it contains a value that isn't finite".into());
        }
        let scale = matrix.diagonal().max().max(f64::MIN_POSITIVE);
        for i in 0..3 {
            if matrix[(i, i)] < 0.0 {
                return invalid(format!("variance {i} is negative ({})", matrix[(i, i)]));
            }
            for j in 0..i {
                if (matrix[(i, j)] - matrix[(j, i)]).abs() > SYMMETRY_TOLERANCE * scale {
                    return invalid(format!(
                        "it isn't symmetric: element ({i}, {j}) is {} but ({j}, {i}) is {}",
                        matrix[(i, j)],
                        matrix[(j, i)]
                    ));
                }
            }
        }
        let symmetric = (matrix + matrix.transpose()) * 0.5;
        // Positive semi-definite: every principal minor non-negative, allowing for rounding.
        let tolerance = 1e-12 * scale;
        for (i, j) in [(0, 1), (0, 2), (1, 2)] {
            let minor =
                symmetric[(i, i)] * symmetric[(j, j)] - symmetric[(i, j)] * symmetric[(i, j)];
            if minor < -tolerance * scale {
                return invalid(format!(
                    "a 2x2 minor is negative ({minor}): correlation over 1"
                ));
            }
        }
        if symmetric.determinant() < -tolerance * scale * scale {
            return invalid("it isn't positive semi-definite (negative determinant)".into());
        }
        Ok(Self(symmetric))
    }

    /// A covariance from standard deviations along the ECEF axes, uncorrelated.
    pub fn from_sigmas(sigma_x_m: f64, sigma_y_m: f64, sigma_z_m: f64) -> Result<Self, DatumError> {
        Self::new(Matrix3::from_diagonal(&Vector3::new(
            sigma_x_m * sigma_x_m,
            sigma_y_m * sigma_y_m,
            sigma_z_m * sigma_z_m,
        )))
    }

    /// A covariance from an east, north, up matrix (m²) at a geodetic position.
    pub fn from_enu(enu: Matrix3<f64>, at: &Geodetic) -> Result<Self, DatumError> {
        let r = at.enu_rotation();
        Self::new(r.transpose() * enu * r)
    }

    /// The ECEF matrix, in m².
    pub const fn matrix(&self) -> &Matrix3<f64> {
        &self.0
    }

    /// The matrix in local east, north, up axes at a geodetic position, in m².
    pub fn to_enu(&self, at: &Geodetic) -> Matrix3<f64> {
        let r = at.enu_rotation();
        r * self.0 * r.transpose()
    }

    /// `J * C * Jᵀ`: the covariance after a linear(ised) map with Jacobian `J`.
    pub(crate) fn propagate(&self, jacobian: &Matrix3<f64>) -> Self {
        let m = jacobian * self.0 * jacobian.transpose();
        Self((m + m.transpose()) * 0.5)
    }
}

/// A position: ECEF coordinates in a frame at a coordinate epoch, with an optional covariance.
///
/// The frame and epoch can't be separated from the coordinates, so positions
/// in different frames or at different epochs can't be combined by accident
/// (CONVENTIONS.md, section 2).
///
/// - In a dynamic frame, the epoch is the coordinate epoch, and it's required.
/// - In a static frame, the epoch is always the frame's reference epoch.
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    frame: FrameId,
    epoch: CoordinateEpoch,
    xyz: Vector3<f64>,
    covariance: Option<Covariance>,
}

impl Position {
    /// A position from ECEF coordinates in metres.
    ///
    /// `epoch` is required for a dynamic frame. For a static frame it can be
    /// `None`, or the frame's reference epoch; any other epoch is an error,
    /// because a coordinate at another epoch belongs in a dynamic frame.
    pub fn new(
        frame: FrameId,
        epoch: Option<CoordinateEpoch>,
        xyz: Vector3<f64>,
        covariance: Option<Covariance>,
    ) -> Result<Self, DatumError> {
        let epoch = resolve_epoch(frame, epoch)?;
        // Validates finiteness and that the point is near the Earth's surface.
        frame.ellipsoid().ecef_to_geodetic(&xyz)?;
        Ok(Self {
            frame,
            epoch,
            xyz,
            covariance,
        })
    }

    /// A position from geodetic coordinates on the frame's ellipsoid.
    pub fn from_geodetic(
        frame: FrameId,
        epoch: Option<CoordinateEpoch>,
        geodetic: &Geodetic,
        covariance: Option<Covariance>,
    ) -> Result<Self, DatumError> {
        Self::new(
            frame,
            epoch,
            frame.ellipsoid().geodetic_to_ecef(geodetic),
            covariance,
        )
    }

    /// The frame.
    pub const fn frame(&self) -> FrameId {
        self.frame
    }

    /// The coordinate epoch. For a static frame, its reference epoch.
    pub const fn epoch(&self) -> CoordinateEpoch {
        self.epoch
    }

    /// ECEF coordinates, in metres.
    pub const fn xyz(&self) -> &Vector3<f64> {
        &self.xyz
    }

    /// The ECEF covariance, if known.
    pub const fn covariance(&self) -> Option<&Covariance> {
        self.covariance.as_ref()
    }

    /// Geodetic coordinates on the frame's ellipsoid.
    pub fn geodetic(&self) -> Geodetic {
        // `new` checked that the coordinates convert, and positions are immutable.
        match self.frame.ellipsoid().ecef_to_geodetic(&self.xyz) {
            Ok(g) => g,
            Err(_) => unreachable!("position coordinates were validated on construction"),
        }
    }

    /// The covariance in local east, north, up axes at the position, in m².
    pub fn enu_covariance(&self) -> Option<Matrix3<f64>> {
        let at = self.geodetic();
        self.covariance.map(|c| c.to_enu(&at))
    }

    /// The ECEF vector from this position to `other` (`other − self`), in metres.
    ///
    /// Both must be in the same frame at the same epoch; transform one first otherwise.
    pub fn vector_to(&self, other: &Position) -> Result<Vector3<f64>, DatumError> {
        self.check_same_frame_and_epoch(other)?;
        Ok(other.xyz - self.xyz)
    }

    /// `other` in local east, north, up axes at this position, in metres.
    pub fn enu_to(&self, other: &Position) -> Result<Vector3<f64>, DatumError> {
        Ok(self.geodetic().enu_rotation() * self.vector_to(other)?)
    }

    fn check_same_frame_and_epoch(&self, other: &Position) -> Result<(), DatumError> {
        if self.frame != other.frame || self.epoch != other.epoch {
            return Err(DatumError::FrameMismatch {
                a: self.frame_and_epoch(),
                b: other.frame_and_epoch(),
            });
        }
        Ok(())
    }

    /// "ITRF2014 at 2018.0" or "GDA2020" (a static frame needs no epoch).
    pub fn frame_and_epoch(&self) -> String {
        describe(self.frame, self.epoch)
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({:.4}, {:.4}, {:.4}) m",
            self.frame_and_epoch(),
            self.xyz.x,
            self.xyz.y,
            self.xyz.z
        )
    }
}

/// A frame and epoch in words: "ITRF2014 at 2018.0", or just "GDA2020" for a static frame.
pub(crate) fn describe(frame: FrameId, epoch: CoordinateEpoch) -> String {
    if frame.is_static() {
        frame.name().to_owned()
    } else {
        format!("{frame} at {epoch}")
    }
}

/// The epoch a position in `frame` has, applying the static and dynamic frame rules.
pub(crate) fn resolve_epoch(
    frame: FrameId,
    epoch: Option<CoordinateEpoch>,
) -> Result<CoordinateEpoch, DatumError> {
    match (frame.kind(), epoch) {
        (FrameKind::Dynamic, Some(e)) => Ok(e),
        (FrameKind::Dynamic, None) => Err(DatumError::MissingEpoch { frame }),
        (FrameKind::Static { reference_epoch }, None) => Ok(reference_epoch),
        (FrameKind::Static { reference_epoch }, Some(e)) if e == reference_epoch => Ok(e),
        (FrameKind::Static { reference_epoch }, Some(given)) => {
            Err(DatumError::EpochOnStaticFrame {
                frame,
                reference: reference_epoch,
                given,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn epoch(y: f64) -> CoordinateEpoch {
        CoordinateEpoch::from_decimal_year(y).unwrap()
    }

    fn alic() -> Vector3<f64> {
        Vector3::new(-4052052.7373, 4212835.9835, -2545104.5867)
    }

    #[test]
    fn dynamic_frames_need_an_epoch() {
        let err = Position::new(FrameId::Itrf2014, None, alic(), None).unwrap_err();
        assert!(matches!(err, DatumError::MissingEpoch { .. }));
        let p = Position::new(FrameId::Itrf2014, Some(epoch(2018.0)), alic(), None).unwrap();
        assert_eq!(p.frame_and_epoch(), "ITRF2014 at 2018.0");
    }

    #[test]
    fn static_frames_are_at_their_reference_epoch() {
        let p = Position::new(FrameId::Gda2020, None, alic(), None).unwrap();
        assert_eq!(p.epoch().decimal_year(), 2020.0);
        assert_eq!(p.frame_and_epoch(), "GDA2020");
        assert!(Position::new(FrameId::Gda2020, Some(epoch(2020.0)), alic(), None).is_ok());
        let err = Position::new(FrameId::Gda2020, Some(epoch(2023.5)), alic(), None).unwrap_err();
        assert!(err.to_string().contains("ATRF2014"), "{err}");
    }

    #[test]
    fn vectors_need_the_same_frame_and_epoch() {
        let a = Position::new(FrameId::Itrf2014, Some(epoch(2018.0)), alic(), None).unwrap();
        let b_xyz = alic() + Vector3::new(10.0, -20.0, 30.0);
        let b = Position::new(FrameId::Itrf2014, Some(epoch(2018.0)), b_xyz, None).unwrap();
        assert_eq!(a.vector_to(&b).unwrap(), Vector3::new(10.0, -20.0, 30.0));
        let c = Position::new(FrameId::Itrf2014, Some(epoch(2019.0)), b_xyz, None).unwrap();
        assert!(matches!(
            a.vector_to(&c),
            Err(DatumError::FrameMismatch { .. })
        ));
        let d = Position::new(FrameId::Gda2020, None, b_xyz, None).unwrap();
        assert!(matches!(
            a.vector_to(&d),
            Err(DatumError::FrameMismatch { .. })
        ));
    }

    #[test]
    fn enu_vector_points_the_right_way() {
        let a = Position::new(FrameId::Gda2020, None, alic(), None).unwrap();
        let g = a.geodetic();
        let north = Geodetic::new(
            g.latitude_rad() + 1e-6,
            g.longitude_rad(),
            g.ellipsoidal_height_m(),
        )
        .unwrap();
        let b = Position::from_geodetic(FrameId::Gda2020, None, &north, None).unwrap();
        let enu = a.enu_to(&b).unwrap();
        assert!(
            enu.y > 6.0 && enu.x.abs() < 1e-6 && enu.z.abs() < 1e-5,
            "{enu}"
        );
    }

    #[test]
    fn covariance_checks() {
        assert!(Covariance::from_sigmas(0.003, 0.003, 0.006).is_ok());
        let mut m = Matrix3::from_diagonal_element(1e-6);
        m[(0, 1)] = 2e-6;
        m[(1, 0)] = 2e-6;
        assert!(Covariance::new(m).is_err(), "correlation over 1");
        m[(1, 0)] = 0.0;
        assert!(Covariance::new(m).is_err(), "not symmetric");
        assert!(Covariance::new(Matrix3::from_diagonal_element(-1.0)).is_err());
    }

    #[test]
    fn covariance_round_trips_through_enu() {
        let at = Geodetic::from_degrees(-23.67, 133.89, 603.0).unwrap();
        let enu = Matrix3::new(9e-6, 1e-6, 0.0, 1e-6, 9e-6, 0.0, 0.0, 0.0, 3.6e-5);
        let c = Covariance::from_enu(enu, &at).unwrap();
        assert!((c.to_enu(&at) - enu).abs().max() < 1e-20);
    }
}
