//! Geodetic coordinates and the local east, north, up frame.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use nalgebra::Matrix3;

use crate::error::DatumError;

/// Latitude, longitude and ellipsoidal height on some ellipsoid.
///
/// Angles are in radians. Longitude is normalised to (-π, π]. The height is
/// above the ellipsoid; orthometric and other heights have their own types.
/// On its own this says nothing about the frame: a [`Position`](crate::Position)
/// carries that, and its geodetic view uses the frame's ellipsoid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geodetic {
    latitude_rad: f64,
    longitude_rad: f64,
    ellipsoidal_height_m: f64,
}

impl Geodetic {
    /// Geodetic coordinates from radians and metres.
    pub fn new(
        latitude_rad: f64,
        longitude_rad: f64,
        ellipsoidal_height_m: f64,
    ) -> Result<Self, DatumError> {
        for (what, value) in [
            ("latitude", latitude_rad),
            ("longitude", longitude_rad),
            ("ellipsoidal height", ellipsoidal_height_m),
        ] {
            if !value.is_finite() {
                return Err(DatumError::NotFinite { what, value });
            }
        }
        if latitude_rad.abs() > FRAC_PI_2 {
            return Err(DatumError::InvalidLatitude {
                latitude_deg: latitude_rad.to_degrees(),
            });
        }
        Ok(Self {
            latitude_rad,
            longitude_rad: normalise_longitude(longitude_rad),
            ellipsoidal_height_m,
        })
    }

    /// Geodetic coordinates from decimal degrees and metres.
    ///
    /// Degrees belong at the edges (CONVENTIONS.md, section 1): parsers and
    /// tests use this, and everything inside works in radians.
    pub fn from_degrees(
        latitude_deg: f64,
        longitude_deg: f64,
        ellipsoidal_height_m: f64,
    ) -> Result<Self, DatumError> {
        Self::new(
            latitude_deg.to_radians(),
            longitude_deg.to_radians(),
            ellipsoidal_height_m,
        )
    }

    /// Latitude, in radians.
    pub const fn latitude_rad(&self) -> f64 {
        self.latitude_rad
    }

    /// Longitude, in radians, in (-π, π].
    pub const fn longitude_rad(&self) -> f64 {
        self.longitude_rad
    }

    /// Height above the ellipsoid, in metres.
    pub const fn ellipsoidal_height_m(&self) -> f64 {
        self.ellipsoidal_height_m
    }

    /// Latitude, in decimal degrees.
    pub fn latitude_deg(&self) -> f64 {
        self.latitude_rad.to_degrees()
    }

    /// Longitude, in decimal degrees.
    pub fn longitude_deg(&self) -> f64 {
        self.longitude_rad.to_degrees()
    }

    /// The rotation from ECEF to local east, north, up axes at this point.
    pub fn enu_rotation(&self) -> Matrix3<f64> {
        enu_rotation(self.latitude_rad, self.longitude_rad)
    }
}

/// The rotation from ECEF to local east, north, up (ENU) axes at a latitude and longitude.
///
/// Rows are the east, north and up unit vectors in ECEF. An ECEF difference
/// `d` is `R * d` in ENU, and an ECEF covariance `C` is `R * C * Rᵀ`.
pub fn enu_rotation(latitude_rad: f64, longitude_rad: f64) -> Matrix3<f64> {
    let (sin_lat, cos_lat) = libm::sincos(latitude_rad);
    let (sin_lon, cos_lon) = libm::sincos(longitude_rad);
    Matrix3::new(
        -sin_lon,
        cos_lon,
        0.0,
        -sin_lat * cos_lon,
        -sin_lat * sin_lon,
        cos_lat,
        cos_lat * cos_lon,
        cos_lat * sin_lon,
        sin_lat,
    )
}

/// A longitude in (-π, π]. IEEE remainder is exact, so this adds no rounding error.
fn normalise_longitude(longitude_rad: f64) -> f64 {
    let wrapped = libm::remainder(longitude_rad, TAU);
    if wrapped <= -PI {
        wrapped + TAU
    } else {
        wrapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn longitude_is_normalised() {
        let g = Geodetic::from_degrees(-30.0, 190.0, 0.0).unwrap();
        assert!((g.longitude_deg() + 170.0).abs() < 1e-12);
        let g = Geodetic::from_degrees(-30.0, -180.0, 0.0).unwrap();
        assert!((g.longitude_deg() - 180.0).abs() < 1e-12);
        let g = Geodetic::from_degrees(-30.0, 147.25, 0.0).unwrap();
        assert_eq!(g.longitude_deg(), 147.25_f64.to_radians().to_degrees());
    }

    #[test]
    fn rejects_bad_latitude_and_non_finite_values() {
        assert!(Geodetic::from_degrees(90.0001, 0.0, 0.0).is_err());
        assert!(Geodetic::from_degrees(-30.0, f64::INFINITY, 0.0).is_err());
        assert!(Geodetic::from_degrees(-30.0, 0.0, f64::NAN).is_err());
    }

    #[test]
    fn enu_rotation_is_orthonormal_and_points_the_right_way() {
        let r = enu_rotation(0.0, 0.0);
        // At (0°, 0°): east is +Y, north is +Z, up is +X.
        assert_eq!(r.row(0).transpose(), nalgebra::Vector3::new(0.0, 1.0, 0.0));
        assert_eq!(r.row(1).transpose(), nalgebra::Vector3::new(0.0, 0.0, 1.0));
        assert_eq!(r.row(2).transpose(), nalgebra::Vector3::new(1.0, 0.0, 0.0));
        let r = enu_rotation(-0.6, 2.6);
        assert!((r * r.transpose() - Matrix3::identity()).abs().max() < 1e-15);
        assert!((r.determinant() - 1.0).abs() < 1e-15);
    }
}
