//! Reference ellipsoids and conversions between ECEF and geodetic coordinates.

use nalgebra::{Matrix3, Vector3};

use crate::error::DatumError;
use crate::geodetic::Geodetic;

/// Positions must be at least this far from the geocentre (metres).
const MIN_RADIUS_M: f64 = 5_000_000.0;
/// Positions must be at most this far from the geocentre (metres).
const MAX_RADIUS_M: f64 = 10_000_000.0;

/// A reference ellipsoid of revolution, defined by its semi-major axis and inverse flattening.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ellipsoid {
    name: &'static str,
    a: f64,
    inverse_flattening: f64,
    f: f64,
    b: f64,
    e2: f64,
    n: f64,
}

/// The Geodetic Reference System 1980 ellipsoid (EPSG:7019), used by ITRF, ATRF, GDA2020 and GDA94.
pub const GRS80: Ellipsoid = Ellipsoid::new("GRS80", 6_378_137.0, 298.257_222_101);

/// The World Geodetic System 1984 ellipsoid (EPSG:7030), used by WGS 84 realisations.
pub const WGS84: Ellipsoid = Ellipsoid::new("WGS 84", 6_378_137.0, 298.257_223_563);

impl Ellipsoid {
    /// An ellipsoid from its semi-major axis (metres) and inverse flattening.
    pub const fn new(name: &'static str, semi_major_axis_m: f64, inverse_flattening: f64) -> Self {
        let a = semi_major_axis_m;
        let f = 1.0 / inverse_flattening;
        Self {
            name,
            a,
            inverse_flattening,
            f,
            b: a * (1.0 - f),
            e2: f * (2.0 - f),
            n: f / (2.0 - f),
        }
    }

    /// The ellipsoid's name.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Semi-major axis, in metres.
    pub const fn semi_major_axis_m(&self) -> f64 {
        self.a
    }

    /// Semi-minor axis, in metres.
    pub const fn semi_minor_axis_m(&self) -> f64 {
        self.b
    }

    /// Inverse flattening, 1/f.
    pub const fn inverse_flattening(&self) -> f64 {
        self.inverse_flattening
    }

    /// Flattening, f.
    pub const fn flattening(&self) -> f64 {
        self.f
    }

    /// First eccentricity squared, e².
    pub const fn eccentricity_squared(&self) -> f64 {
        self.e2
    }

    /// Third flattening, n = f / (2 - f), the parameter of the Krüger series.
    pub const fn third_flattening(&self) -> f64 {
        self.n
    }

    /// Radius of curvature in the prime vertical, ν, at a latitude (metres).
    pub fn prime_vertical_radius_m(&self, latitude_rad: f64) -> f64 {
        let s = libm::sin(latitude_rad);
        self.a / libm::sqrt(1.0 - self.e2 * s * s)
    }

    /// Radius of curvature in the meridian, ρ, at a latitude (metres).
    pub fn meridian_radius_m(&self, latitude_rad: f64) -> f64 {
        let s = libm::sin(latitude_rad);
        let w2 = 1.0 - self.e2 * s * s;
        self.a * (1.0 - self.e2) / (w2 * libm::sqrt(w2))
    }

    /// The geometric mean radius of curvature, √(ρν), at a latitude (metres).
    pub fn mean_radius_m(&self, latitude_rad: f64) -> f64 {
        libm::sqrt(
            self.meridian_radius_m(latitude_rad) * self.prime_vertical_radius_m(latitude_rad),
        )
    }

    /// ECEF coordinates (metres) of a geodetic position on this ellipsoid.
    pub fn geodetic_to_ecef(&self, geodetic: &Geodetic) -> Vector3<f64> {
        let (sin_lat, cos_lat) = libm::sincos(geodetic.latitude_rad());
        let (sin_lon, cos_lon) = libm::sincos(geodetic.longitude_rad());
        let nu = self.a / libm::sqrt(1.0 - self.e2 * sin_lat * sin_lat);
        let h = geodetic.ellipsoidal_height_m();
        Vector3::new(
            (nu + h) * cos_lat * cos_lon,
            (nu + h) * cos_lat * sin_lon,
            (nu * (1.0 - self.e2) + h) * sin_lat,
        )
    }

    /// Geodetic coordinates of an ECEF position, by Vermeille's closed form.
    ///
    /// Vermeille (2004), "Direct transformation from geocentric coordinates to
    /// geodetic coordinates", Journal of Geodesy 78:94-95. It needs no
    /// iteration and is exact to rounding for positions near the Earth's
    /// surface, which is all this accepts: 5,000 to 10,000 km from the geocentre.
    pub fn ecef_to_geodetic(&self, xyz: &Vector3<f64>) -> Result<Geodetic, DatumError> {
        let (x, y, z) = (xyz.x, xyz.y, xyz.z);
        for (what, value) in [
            ("X coordinate", x),
            ("Y coordinate", y),
            ("Z coordinate", z),
        ] {
            if !value.is_finite() {
                return Err(DatumError::NotFinite { what, value });
            }
        }
        let radius = libm::sqrt(x * x + y * y + z * z);
        if !(MIN_RADIUS_M..=MAX_RADIUS_M).contains(&radius) {
            return Err(DatumError::NotNearSurface {
                x_m: x,
                y_m: y,
                z_m: z,
                radius_km: radius / 1000.0,
            });
        }

        let a2 = self.a * self.a;
        let e4 = self.e2 * self.e2;
        let rho2 = x * x + y * y;
        let rho = libm::sqrt(rho2);
        let p = rho2 / a2;
        let q = (1.0 - self.e2) / a2 * z * z;
        let r = (p + q - e4) / 6.0;
        let s = e4 * p * q / (4.0 * r * r * r);
        let t = libm::cbrt(1.0 + s + libm::sqrt(s * (2.0 + s)));
        let u = r * (1.0 + t + 1.0 / t);
        let v = libm::sqrt(u * u + e4 * q);
        let w = self.e2 * (u + v - q) / (2.0 * v);
        let k = libm::sqrt(u + v + w * w) - w;
        let d = k * rho / (k + self.e2);
        let dz = libm::sqrt(d * d + z * z);

        let latitude = 2.0 * libm::atan2(z, d + dz);
        let longitude = libm::atan2(y, x);
        let height = (k + self.e2 - 1.0) / k * dz;
        Geodetic::new(latitude, longitude, height)
    }

    /// The Jacobian ∂(X, Y, Z)/∂(φ, λ, h) at a geodetic position.
    ///
    /// Columns are latitude (m/rad), longitude (m/rad) and height (m/m).
    pub fn geodetic_to_ecef_jacobian(&self, geodetic: &Geodetic) -> Matrix3<f64> {
        let lat = geodetic.latitude_rad();
        let h = geodetic.ellipsoidal_height_m();
        let (sin_lat, cos_lat) = libm::sincos(lat);
        let (sin_lon, cos_lon) = libm::sincos(geodetic.longitude_rad());
        let rho_h = self.meridian_radius_m(lat) + h;
        let nu_h = self.prime_vertical_radius_m(lat) + h;
        Matrix3::new(
            -rho_h * sin_lat * cos_lon,
            -nu_h * cos_lat * sin_lon,
            cos_lat * cos_lon,
            -rho_h * sin_lat * sin_lon,
            nu_h * cos_lat * cos_lon,
            cos_lat * sin_lon,
            rho_h * cos_lat,
            0.0,
            sin_lat,
        )
    }

    /// The Jacobian ∂(φ, λ, h)/∂(X, Y, Z) at a geodetic position.
    ///
    /// Rows are latitude (rad/m), longitude (rad/m) and height (m/m). Not
    /// defined at the poles, where longitude is undefined.
    pub fn ecef_to_geodetic_jacobian(&self, geodetic: &Geodetic) -> Matrix3<f64> {
        let lat = geodetic.latitude_rad();
        let h = geodetic.ellipsoidal_height_m();
        let (sin_lat, cos_lat) = libm::sincos(lat);
        let (sin_lon, cos_lon) = libm::sincos(geodetic.longitude_rad());
        let rho_h = self.meridian_radius_m(lat) + h;
        let nu_cos = (self.prime_vertical_radius_m(lat) + h) * cos_lat;
        Matrix3::new(
            -sin_lat * cos_lon / rho_h,
            -sin_lat * sin_lon / rho_h,
            cos_lat / rho_h,
            -sin_lon / nu_cos,
            cos_lon / nu_cos,
            0.0,
            cos_lat * cos_lon,
            cos_lat * sin_lon,
            sin_lat,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::SplitMix64;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const DEG: f64 = std::f64::consts::PI / 180.0;

    #[test]
    fn grs80_derived_constants() {
        assert_eq!(GRS80.semi_major_axis_m(), 6_378_137.0);
        // From a and the published (rounded) 1/f, so 9 nm off GRS80's defining value.
        assert!((GRS80.semi_minor_axis_m() - 6_356_752.314_140_356).abs() < 1e-9);
        assert!((GRS80.eccentricity_squared() - 0.006_694_380_022_900_787).abs() < 1e-18);
    }

    #[test]
    fn round_trips_across_the_globe() {
        let mut rng = SplitMix64::new(1);
        for _ in 0..20_000 {
            let lat = rng.uniform(-90.0, 90.0) * DEG;
            let lon = rng.uniform(-180.0, 180.0) * DEG;
            let h = rng.uniform(-500.0, 9_000.0);
            let g = Geodetic::new(lat, lon, h).unwrap();
            let back = GRS80.ecef_to_geodetic(&GRS80.geodetic_to_ecef(&g)).unwrap();
            // Compare on the ground, in metres. Double precision of a
            // 6,400 km coordinate is about a nanometre, so allow ten.
            let north_m = (back.latitude_rad() - lat) * 6_371_000.0;
            let east_m = (back.longitude_rad() - lon) * 6_371_000.0 * libm::cos(lat);
            assert!(north_m.abs() < 1e-8, "latitude {lat} {h}: {north_m}");
            assert!(east_m.abs() < 1e-8, "longitude {lat} {lon}: {east_m}");
            assert!(
                (back.ellipsoidal_height_m() - h).abs() < 1e-8,
                "height {lat} {h}"
            );
        }
    }

    #[test]
    fn poles_and_equator() {
        for (lat, lon) in [
            (90.0, 0.0),
            (-90.0, 0.0),
            (0.0, 0.0),
            (0.0, 180.0),
            (0.0, -90.0),
        ] {
            let g = Geodetic::new(lat * DEG, lon * DEG, 100.0).unwrap();
            let back = GRS80.ecef_to_geodetic(&GRS80.geodetic_to_ecef(&g)).unwrap();
            assert!((back.latitude_rad() - g.latitude_rad()).abs() < 1e-15);
            assert!((back.ellipsoidal_height_m() - 100.0).abs() < 1e-9);
        }
    }

    #[test]
    fn rejects_points_far_from_the_surface() {
        assert!(
            GRS80
                .ecef_to_geodetic(&Vector3::new(0.0, 0.0, 0.0))
                .is_err()
        );
        assert!(
            GRS80
                .ecef_to_geodetic(&Vector3::new(26_000_000.0, 0.0, 0.0))
                .is_err()
        );
        assert!(
            GRS80
                .ecef_to_geodetic(&Vector3::new(f64::NAN, 0.0, 0.0))
                .is_err()
        );
    }

    #[test]
    fn jacobians_match_finite_differences_and_invert() {
        let g = Geodetic::new(-35.3 * DEG, 149.1 * DEG, 600.0).unwrap();
        let j = GRS80.geodetic_to_ecef_jacobian(&g);
        let base = GRS80.geodetic_to_ecef(&g);
        let steps = [1e-7, 1e-7, 1e-2];
        for (col, step) in steps.iter().enumerate() {
            let mut p = [
                g.latitude_rad(),
                g.longitude_rad(),
                g.ellipsoidal_height_m(),
            ];
            p[col] += step;
            let moved = GRS80.geodetic_to_ecef(&Geodetic::new(p[0], p[1], p[2]).unwrap());
            let numeric = (moved - base) / *step;
            for row in 0..3 {
                let tol = 1e-6 * j[(row, col)].abs().max(1.0);
                assert!((numeric[row] - j[(row, col)]).abs() < tol, "({row}, {col})");
            }
        }
        // Mixed units (rad/m against m/rad) cancel terms of several million metres.
        let identity = GRS80.ecef_to_geodetic_jacobian(&g) * j;
        assert!((identity - Matrix3::identity()).abs().max() < 1e-9);
    }
}
