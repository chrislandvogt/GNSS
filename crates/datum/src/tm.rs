//! Transverse Mercator by the Krüger n-series, and the Map Grid of Australia.
//!
//! The series are Karney's to eighth order in n: Karney (2011), "Transverse
//! Mercator with an accuracy of a few nanometers", Journal of Geodesy
//! 85:475-485. Within 30° of the central meridian they're accurate to well
//! under a micrometre.

use std::fmt;

use crate::ellipsoid::{Ellipsoid, GRS80};
use crate::error::DatumError;
use crate::frame::FrameId;
use crate::geodetic::Geodetic;
use crate::position::{Covariance, Position};

/// The furthest from the central meridian, in degrees, that coordinates are computed.
const MAX_OFFSET_DEG: f64 = 30.0;

/// Coefficients of the Krüger α series, α1 to α8, in powers n¹ to n⁸ (Karney 2011, eq. 35).
const ALPHA: [[f64; 8]; 8] = [
    [
        1.0 / 2.0,
        -2.0 / 3.0,
        5.0 / 16.0,
        41.0 / 180.0,
        -127.0 / 288.0,
        7891.0 / 37800.0,
        72161.0 / 387072.0,
        -18975107.0 / 50803200.0,
    ],
    [
        0.0,
        13.0 / 48.0,
        -3.0 / 5.0,
        557.0 / 1440.0,
        281.0 / 630.0,
        -1983433.0 / 1935360.0,
        13769.0 / 28800.0,
        148003883.0 / 174182400.0,
    ],
    [
        0.0,
        0.0,
        61.0 / 240.0,
        -103.0 / 140.0,
        15061.0 / 26880.0,
        167603.0 / 181440.0,
        -67102379.0 / 29030400.0,
        79682431.0 / 79833600.0,
    ],
    [
        0.0,
        0.0,
        0.0,
        49561.0 / 161280.0,
        -179.0 / 168.0,
        6601661.0 / 7257600.0,
        97445.0 / 49896.0,
        -40176129013.0 / 7664025600.0,
    ],
    [
        0.0,
        0.0,
        0.0,
        0.0,
        34729.0 / 80640.0,
        -3418889.0 / 1995840.0,
        14644087.0 / 9123840.0,
        2605413599.0 / 622702080.0,
    ],
    [
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        212378941.0 / 319334400.0,
        -30705481.0 / 10378368.0,
        175214326799.0 / 58118860800.0,
    ],
    [
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1522256789.0 / 1383782400.0,
        -16759934899.0 / 3113510400.0,
    ],
    [
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1424729850961.0 / 743921418240.0,
    ],
];

/// Coefficients of the Krüger β series, β1 to β8, in powers n¹ to n⁸ (Karney 2011, eq. 36).
const BETA: [[f64; 8]; 8] = [
    [
        1.0 / 2.0,
        -2.0 / 3.0,
        37.0 / 96.0,
        -1.0 / 360.0,
        -81.0 / 512.0,
        96199.0 / 604800.0,
        -5406467.0 / 38707200.0,
        7944359.0 / 67737600.0,
    ],
    [
        0.0,
        1.0 / 48.0,
        1.0 / 15.0,
        -437.0 / 1440.0,
        46.0 / 105.0,
        -1118711.0 / 3870720.0,
        51841.0 / 1209600.0,
        24749483.0 / 348364800.0,
    ],
    [
        0.0,
        0.0,
        17.0 / 480.0,
        -37.0 / 840.0,
        -209.0 / 4480.0,
        5569.0 / 90720.0,
        9261899.0 / 58060800.0,
        -6457463.0 / 17740800.0,
    ],
    [
        0.0,
        0.0,
        0.0,
        4397.0 / 161280.0,
        -11.0 / 504.0,
        -830251.0 / 7257600.0,
        466511.0 / 2494800.0,
        324154477.0 / 7664025600.0,
    ],
    [
        0.0,
        0.0,
        0.0,
        0.0,
        4583.0 / 161280.0,
        -108847.0 / 3991680.0,
        -8005831.0 / 63866880.0,
        22894433.0 / 124540416.0,
    ],
    [
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        20648693.0 / 638668800.0,
        -16363163.0 / 518918400.0,
        -2204645983.0 / 12915302400.0,
    ],
    [
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        219941297.0 / 5535129600.0,
        -497323811.0 / 12454041600.0,
    ],
    [
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        191773887257.0 / 3719607091200.0,
    ],
];

/// Evaluates each row of `table` as a polynomial c₁n + c₂n² + ... + c₈n⁸, by Horner's rule.
fn series(table: &[[f64; 8]; 8], n: f64) -> [f64; 8] {
    table.map(|row| row.iter().rev().fold(0.0, |acc, c| (acc + c) * n))
}

/// Grid coordinates, with the point scale factor and grid convergence there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridPoint {
    /// Easting, in metres, including the false easting.
    pub easting_m: f64,
    /// Northing, in metres, including the false northing.
    pub northing_m: f64,
    /// Point scale factor, k.
    pub point_scale_factor: f64,
    /// Grid convergence, γ, in radians, such that grid bearing = azimuth + γ.
    ///
    /// This is the sign convention of the GDA2020 Technical Manual: in the
    /// southern hemisphere γ is negative west of the central meridian. Some
    /// software (GeographicLib, for one) uses the opposite sign.
    pub grid_convergence_rad: f64,
}

/// A Transverse Mercator projection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransverseMercator {
    ellipsoid: Ellipsoid,
    central_meridian_rad: f64,
    scale_factor: f64,
    false_easting_m: f64,
    false_northing_m: f64,
    e: f64,
    rectifying_radius_m: f64,
    alpha: [f64; 8],
    beta: [f64; 8],
}

impl TransverseMercator {
    /// A projection from its ellipsoid, central meridian (radians), central
    /// scale factor k0, and false easting and northing (metres).
    pub fn new(
        ellipsoid: Ellipsoid,
        central_meridian_rad: f64,
        scale_factor: f64,
        false_easting_m: f64,
        false_northing_m: f64,
    ) -> Self {
        let n = ellipsoid.third_flattening();
        let n2 = n * n;
        let a = ellipsoid.semi_major_axis_m();
        Self {
            ellipsoid,
            central_meridian_rad,
            scale_factor,
            false_easting_m,
            false_northing_m,
            e: libm::sqrt(ellipsoid.eccentricity_squared()),
            rectifying_radius_m: a / (1.0 + n)
                * (1.0
                    + n2 * (1.0 / 4.0
                        + n2 * (1.0 / 64.0 + n2 * (1.0 / 256.0 + n2 * 25.0 / 16384.0)))),
            alpha: series(&ALPHA, n),
            beta: series(&BETA, n),
        }
    }

    /// The central meridian, in radians.
    pub const fn central_meridian_rad(&self) -> f64 {
        self.central_meridian_rad
    }

    /// The ellipsoid.
    pub const fn ellipsoid(&self) -> Ellipsoid {
        self.ellipsoid
    }

    /// Projects a latitude and longitude (the height is ignored).
    pub fn forward(&self, geodetic: &Geodetic) -> Result<GridPoint, DatumError> {
        let lambda = self.offset_from_central_meridian(geodetic.longitude_rad())?;
        let tau = libm::tan(geodetic.latitude_rad());
        let tau_p = self.conformal_tan(tau);
        let (sin_l, cos_l) = libm::sincos(lambda);
        let xi_p = libm::atan2(tau_p, cos_l);
        let eta_p = libm::asinh(sin_l / libm::hypot(tau_p, cos_l));

        let (mut xi, mut eta) = (xi_p, eta_p);
        for (j, alpha) in self.alpha.iter().enumerate() {
            let k = 2.0 * (j as f64 + 1.0);
            let (s, c) = libm::sincos(k * xi_p);
            xi += alpha * s * libm::cosh(k * eta_p);
            eta += alpha * c * libm::sinh(k * eta_p);
        }
        let (k, gamma) =
            self.scale_and_convergence(geodetic.latitude_rad(), tau, tau_p, lambda, xi_p, eta_p);
        Ok(GridPoint {
            easting_m: self.false_easting_m + self.scale_factor * self.rectifying_radius_m * eta,
            northing_m: self.false_northing_m + self.scale_factor * self.rectifying_radius_m * xi,
            point_scale_factor: k,
            grid_convergence_rad: gamma,
        })
    }

    /// Latitude and longitude (radians) of grid coordinates, with the factors there.
    pub fn inverse(
        &self,
        easting_m: f64,
        northing_m: f64,
    ) -> Result<(Geodetic, GridPoint), DatumError> {
        for (what, value) in [("easting", easting_m), ("northing", northing_m)] {
            if !value.is_finite() {
                return Err(DatumError::NotFinite { what, value });
            }
        }
        let scale = self.scale_factor * self.rectifying_radius_m;
        let xi = (northing_m - self.false_northing_m) / scale;
        let eta = (easting_m - self.false_easting_m) / scale;
        let (mut xi_p, mut eta_p) = (xi, eta);
        for (j, beta) in self.beta.iter().enumerate() {
            let k = 2.0 * (j as f64 + 1.0);
            let (s, c) = libm::sincos(k * xi);
            xi_p -= beta * s * libm::cosh(k * eta);
            eta_p -= beta * c * libm::sinh(k * eta);
        }
        let sinh_eta = libm::sinh(eta_p);
        let (sin_xi, cos_xi) = libm::sincos(xi_p);
        let tau_p = sin_xi / libm::hypot(sinh_eta, cos_xi);
        let lambda = libm::atan2(sinh_eta, cos_xi);
        if lambda.abs() > MAX_OFFSET_DEG.to_radians() {
            return Err(DatumError::TooFarFromCentralMeridian {
                offset_deg: lambda.to_degrees(),
                limit_deg: MAX_OFFSET_DEG,
            });
        }
        let latitude = libm::atan(self.geographic_tan(tau_p));
        let geodetic = Geodetic::new(latitude, self.central_meridian_rad + lambda, 0.0)?;
        let factors = self.forward(&geodetic)?;
        Ok((
            geodetic,
            GridPoint {
                easting_m,
                northing_m,
                ..factors
            },
        ))
    }

    /// The longitude difference from the central meridian, in (-π, π], within the limit.
    fn offset_from_central_meridian(&self, longitude_rad: f64) -> Result<f64, DatumError> {
        let lambda = libm::remainder(
            longitude_rad - self.central_meridian_rad,
            std::f64::consts::TAU,
        );
        if lambda.abs() > MAX_OFFSET_DEG.to_radians() {
            return Err(DatumError::TooFarFromCentralMeridian {
                offset_deg: lambda.to_degrees(),
                limit_deg: MAX_OFFSET_DEG,
            });
        }
        Ok(lambda)
    }

    /// tan of the conformal latitude, τ', from tan of the geodetic latitude, τ (Karney 2011, eq. 7).
    fn conformal_tan(&self, tau: f64) -> f64 {
        let sigma = libm::sinh(self.e * libm::atanh(self.e * tau / libm::hypot(1.0, tau)));
        tau * libm::hypot(1.0, sigma) - sigma * libm::hypot(1.0, tau)
    }

    /// tan of the geodetic latitude from tan of the conformal latitude, by
    /// Newton's method (Karney 2011, eqs. 19-21). Converges in two or three steps.
    fn geographic_tan(&self, tau_p: f64) -> f64 {
        let e2m = 1.0 - self.ellipsoid.eccentricity_squared();
        let mut tau = tau_p / e2m;
        let tolerance = f64::EPSILON * 0.1 * tau.abs().max(1.0);
        for _ in 0..8 {
            let tau_i = self.conformal_tan(tau);
            let delta = (tau_p - tau_i) / libm::hypot(1.0, tau_i) * (1.0 + e2m * tau * tau)
                / (e2m * libm::hypot(1.0, tau));
            tau += delta;
            if delta.abs() <= tolerance {
                break;
            }
        }
        tau
    }

    /// Point scale factor and grid convergence (Karney 2011, eqs. 10, 11, 25 and 26).
    fn scale_and_convergence(
        &self,
        latitude_rad: f64,
        tau: f64,
        tau_p: f64,
        lambda: f64,
        xi_p: f64,
        eta_p: f64,
    ) -> (f64, f64) {
        let (mut p, mut q) = (1.0, 0.0);
        for (j, alpha) in self.alpha.iter().enumerate() {
            let k = 2.0 * (j as f64 + 1.0);
            let (s, c) = libm::sincos(k * xi_p);
            p += k * alpha * c * libm::cosh(k * eta_p);
            q += k * alpha * s * libm::sinh(k * eta_p);
        }
        let (sin_l, cos_l) = libm::sincos(lambda);
        let sin_lat = libm::sin(latitude_rad);
        // Convergence of the conformal sphere's projection, and the series' rotation of it.
        let gamma_sphere = libm::atan2(tau_p * sin_l, libm::hypot(1.0, tau_p) * cos_l);
        let gamma_series = libm::atan2(q, p);
        let k_sphere = libm::sqrt(1.0 - self.ellipsoid.eccentricity_squared() * sin_lat * sin_lat)
            * libm::hypot(1.0, tau)
            / libm::hypot(tau_p, cos_l);
        let k_series =
            self.rectifying_radius_m / self.ellipsoid.semi_major_axis_m() * libm::hypot(p, q);
        // Karney's γ is the bearing of grid north from true north; ours is its negative.
        (
            self.scale_factor * k_sphere * k_series,
            -(gamma_sphere + gamma_series),
        )
    }
}

/// A zone of the Map Grid of Australia, numbered like UTM zones (1 to 60).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MgaZone(u8);

impl MgaZone {
    /// A zone from its number. Australia's mainland and territories use 46 to 59.
    pub fn new(number: u8) -> Result<Self, DatumError> {
        if (1..=60).contains(&number) {
            Ok(Self(number))
        } else {
            Err(DatumError::InvalidZone { zone: number })
        }
    }

    /// The zone a longitude falls in. A longitude on a zone boundary belongs to the zone east of it.
    pub fn containing(longitude_rad: f64) -> Self {
        let degrees = libm::remainder(longitude_rad.to_degrees(), 360.0);
        let number = libm::floor((degrees + 180.0) / 6.0) as i32 + 1;
        Self(number.clamp(1, 60) as u8)
    }

    /// The zone number.
    pub const fn number(self) -> u8 {
        self.0
    }

    /// The zone's central meridian, in degrees.
    pub fn central_meridian_deg(self) -> f64 {
        f64::from(self.0) * 6.0 - 183.0
    }

    /// The zone's Transverse Mercator projection: GRS80, k0 = 0.9996,
    /// false easting 500 000 m and false northing 10 000 000 m.
    pub fn projection(self) -> TransverseMercator {
        TransverseMercator::new(
            GRS80,
            self.central_meridian_deg().to_radians(),
            0.9996,
            500_000.0,
            10_000_000.0,
        )
    }
}

impl fmt::Display for MgaZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Which Map Grid of Australia: each is defined on one datum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MgaGrid {
    /// MGA2020, on GDA2020.
    Mga2020,
    /// MGA94, on GDA94.
    Mga94,
}

impl MgaGrid {
    /// The frame the grid is defined on.
    pub const fn frame(self) -> FrameId {
        match self {
            MgaGrid::Mga2020 => FrameId::Gda2020,
            MgaGrid::Mga94 => FrameId::Gda94,
        }
    }

    /// The grid's name.
    pub const fn name(self) -> &'static str {
        match self {
            MgaGrid::Mga2020 => "MGA2020",
            MgaGrid::Mga94 => "MGA94",
        }
    }

    /// The grid defined on a frame, if there is one.
    pub fn for_frame(frame: FrameId) -> Option<Self> {
        match frame {
            FrameId::Gda2020 => Some(MgaGrid::Mga2020),
            FrameId::Gda94 => Some(MgaGrid::Mga94),
            _ => None,
        }
    }
}

/// A Map Grid of Australia coordinate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MgaCoordinate {
    /// MGA2020 or MGA94.
    pub grid: MgaGrid,
    /// The zone.
    pub zone: MgaZone,
    /// Easting, in metres.
    pub easting_m: f64,
    /// Northing, in metres.
    pub northing_m: f64,
}

impl fmt::Display for MgaCoordinate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} zone {} E {:.4} N {:.4}",
            self.grid.name(),
            self.zone,
            self.easting_m,
            self.northing_m
        )
    }
}

/// A position's MGA coordinate, with the point scale factor and grid convergence.
///
/// The position must be in GDA2020 (giving MGA2020) or GDA94 (giving MGA94).
/// The zone defaults to the one the position is in; give one to compute in a
/// neighbouring zone.
pub fn to_mga(
    position: &Position,
    zone: Option<MgaZone>,
) -> Result<(MgaCoordinate, GridPoint), DatumError> {
    let grid = MgaGrid::for_frame(position.frame()).ok_or(DatumError::WrongFrameForProjection {
        projection: "MGA2020",
        expected: FrameId::Gda2020,
        frame: position.frame(),
    })?;
    let geodetic = position.geodetic();
    let zone = zone.unwrap_or_else(|| MgaZone::containing(geodetic.longitude_rad()));
    let point = zone.projection().forward(&geodetic)?;
    Ok((
        MgaCoordinate {
            grid,
            zone,
            easting_m: point.easting_m,
            northing_m: point.northing_m,
        },
        point,
    ))
}

/// The position of an MGA coordinate with an ellipsoidal height, in the grid's frame.
pub fn from_mga(
    coordinate: &MgaCoordinate,
    ellipsoidal_height_m: f64,
    covariance: Option<Covariance>,
) -> Result<Position, DatumError> {
    let (geodetic, _) = coordinate
        .zone
        .projection()
        .inverse(coordinate.easting_m, coordinate.northing_m)?;
    let geodetic = Geodetic::new(
        geodetic.latitude_rad(),
        geodetic.longitude_rad(),
        ellipsoidal_height_m,
    )?;
    Position::from_geodetic(coordinate.grid.frame(), None, &geodetic, covariance)
}

/// The line scale factor between two grid points, by Simpson's rule over
/// the point scale factors at each end and the midpoint.
///
/// Grid distance = ellipsoidal distance × line scale factor. Simpson's rule is
/// accurate to better than 1e-9 for lines up to about 20 km.
pub fn line_scale_factor(
    projection: &TransverseMercator,
    from: (f64, f64),
    to: (f64, f64),
) -> Result<f64, DatumError> {
    let k1 = projection.inverse(from.0, from.1)?.1.point_scale_factor;
    let k2 = projection.inverse(to.0, to.1)?.1.point_scale_factor;
    let km = projection
        .inverse((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0)?
        .1
        .point_scale_factor;
    Ok((k1 + 4.0 * km + k2) / 6.0)
}

/// The height scale factor R / (R + h): ellipsoidal distance = horizontal distance × this.
///
/// R is the geometric mean radius of curvature at the latitude, and h the
/// ellipsoidal height of the line (its mean, for a line).
pub fn height_scale_factor(
    ellipsoid: &Ellipsoid,
    latitude_rad: f64,
    ellipsoidal_height_m: f64,
) -> f64 {
    let r = ellipsoid.mean_radius_m(latitude_rad);
    r / (r + ellipsoidal_height_m)
}

/// The combined scale factor between two points: line scale factor × height scale factor.
///
/// Grid distance = horizontal ground distance × combined scale factor. Each
/// point is easting, northing and ellipsoidal height, in metres.
pub fn combined_scale_factor(
    projection: &TransverseMercator,
    from: (f64, f64, f64),
    to: (f64, f64, f64),
) -> Result<f64, DatumError> {
    let line = line_scale_factor(projection, (from.0, from.1), (to.0, to.1))?;
    let (mid, _) = projection.inverse((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0)?;
    let height = height_scale_factor(
        &projection.ellipsoid(),
        mid.latitude_rad(),
        (from.2 + to.2) / 2.0,
    );
    Ok(line * height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::SplitMix64;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn dms(d: f64, m: f64, s: f64) -> f64 {
        d.signum() * (d.abs() + m / 60.0 + s / 3600.0)
    }

    #[test]
    fn geodepy_reference_point() {
        // GeodePy's test point (Melbourne): zone 55, k 0.99999287, γ -1.2439811331°.
        // E and N as PROJ 9.8.1 gives them (GeodePy agrees to its 0.1 mm rounding).
        let g = Geodetic::from_degrees(dms(-37.0, 48.0, 26.67598), dms(144.0, 58.0, 16.44114), 0.0)
            .unwrap();
        let zone = MgaZone::containing(g.longitude_rad());
        assert_eq!(zone.number(), 55);
        let p = zone.projection().forward(&g).unwrap();
        assert!(
            (p.easting_m - 321_405.559_109).abs() < 1e-6,
            "{}",
            p.easting_m
        );
        assert!(
            (p.northing_m - 5_813_614.161_319).abs() < 1e-6,
            "{}",
            p.northing_m
        );
        assert!((p.point_scale_factor - 0.999_992_87).abs() < 5e-9);
        assert!((p.grid_convergence_rad.to_degrees() + 1.243_981_133_1).abs() < 1e-9);
    }

    #[test]
    fn inverse_round_trips() {
        let mut rng = SplitMix64::new(7);
        let zone = MgaZone::new(55).unwrap();
        let tm = zone.projection();
        for _ in 0..5_000 {
            let lat = rng.uniform(-60.0, 10.0).to_radians();
            let lon = (147.0 + rng.uniform(-12.0, 12.0)).to_radians();
            let g = Geodetic::new(lat, lon, 0.0).unwrap();
            let p = tm.forward(&g).unwrap();
            let (back, q) = tm.inverse(p.easting_m, p.northing_m).unwrap();
            let north_m = (back.latitude_rad() - lat) * 6.4e6;
            let east_m = (back.longitude_rad() - lon) * 6.4e6 * libm::cos(lat);
            assert!(
                north_m.abs() < 1e-8 && east_m.abs() < 1e-8,
                "{lat} {lon}: {north_m} {east_m}"
            );
            assert!((q.point_scale_factor - p.point_scale_factor).abs() < 1e-15);
        }
    }

    #[test]
    fn convergence_matches_the_bearing_of_a_meridian() {
        // Step north along a meridian; its grid bearing is the azimuth (0) plus γ.
        let tm = MgaZone::new(55).unwrap().projection();
        for (lat, lon) in [(-37.0, 144.0), (-37.0, 150.0), (-12.0, 145.5), (5.0, 149.0)] {
            let a = tm
                .forward(&Geodetic::from_degrees(lat, lon, 0.0).unwrap())
                .unwrap();
            let b = tm
                .forward(&Geodetic::from_degrees(lat + 1e-6, lon, 0.0).unwrap())
                .unwrap();
            let bearing = libm::atan2(b.easting_m - a.easting_m, b.northing_m - a.northing_m);
            assert!(
                (bearing - a.grid_convergence_rad).abs() < 1e-8,
                "{lat} {lon}"
            );
        }
    }

    #[test]
    fn point_scale_matches_distances() {
        // Over a short line, grid distance / ellipsoidal distance ≈ k.
        let tm = MgaZone::new(55).unwrap().projection();
        let g = Geodetic::from_degrees(-33.0, 150.2, 0.0).unwrap();
        let a = tm.forward(&g).unwrap();
        let b = tm
            .forward(&Geodetic::from_degrees(-33.0 + 1e-6, 150.2, 0.0).unwrap())
            .unwrap();
        let grid = libm::hypot(b.easting_m - a.easting_m, b.northing_m - a.northing_m);
        let ellipsoidal = GRS80.meridian_radius_m(g.latitude_rad()) * 1e-6_f64.to_radians();
        assert!((grid / ellipsoidal - a.point_scale_factor).abs() < 1e-8);
    }

    #[test]
    fn zones() {
        assert_eq!(MgaZone::containing(147.0_f64.to_radians()).number(), 55);
        assert_eq!(MgaZone::containing(150.0_f64.to_radians()).number(), 56);
        assert_eq!(MgaZone::containing(113.0_f64.to_radians()).number(), 49);
        assert_eq!(MgaZone::new(56).unwrap().central_meridian_deg(), 153.0);
        assert!(MgaZone::new(0).is_err() && MgaZone::new(61).is_err());
    }

    #[test]
    fn refuses_points_far_from_the_central_meridian() {
        let tm = MgaZone::new(55).unwrap().projection();
        assert!(
            tm.forward(&Geodetic::from_degrees(-30.0, 100.0, 0.0).unwrap())
                .is_err()
        );
    }

    #[test]
    fn mga_only_on_gda() {
        let g = Geodetic::from_degrees(-23.67, 133.88, 600.0).unwrap();
        let itrf = Position::from_geodetic(
            FrameId::Itrf2014,
            Some(crate::CoordinateEpoch::from_decimal_year(2018.0).unwrap()),
            &g,
            None,
        )
        .unwrap();
        assert!(matches!(
            to_mga(&itrf, None),
            Err(DatumError::WrongFrameForProjection { .. })
        ));
        let gda = Position::from_geodetic(FrameId::Gda2020, None, &g, None).unwrap();
        let (mga, _) = to_mga(&gda, None).unwrap();
        assert_eq!((mga.grid, mga.zone.number()), (MgaGrid::Mga2020, 53));
        let back = from_mga(&mga, 600.0, None).unwrap();
        assert!((back.xyz() - gda.xyz()).norm() < 1e-8);
    }

    #[test]
    fn scale_factors() {
        let tm = MgaZone::new(56).unwrap().projection();
        // A line near the central meridian: k close to 0.9996.
        let k = line_scale_factor(&tm, (499_000.0, 6_250_000.0), (501_000.0, 6_252_000.0)).unwrap();
        assert!((k - 0.9996).abs() < 1e-7, "{k}");
        // At 1000 m: height factor about 1 - 1000/6.37e6, at the line's mid latitude.
        let (mid, _) = tm.inverse(500_000.0, 6_251_000.0).unwrap();
        let h = height_scale_factor(&GRS80, mid.latitude_rad(), 1000.0);
        assert!((h - (1.0 - 1000.0 / 6_371_000.0)).abs() < 1e-6, "{h}");
        let c = combined_scale_factor(
            &tm,
            (499_000.0, 6_250_000.0, 1000.0),
            (501_000.0, 6_252_000.0, 1000.0),
        )
        .unwrap();
        assert!((c - k * h).abs() < 1e-9);
    }
}
