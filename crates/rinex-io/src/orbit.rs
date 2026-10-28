//! Satellite positions from broadcast ephemerides, and where a satellite
//! is in a receiver's sky.
//!
//! The algorithms are the systems' interface documents': IS-GPS-200 for
//! the Keplerian ephemerides of GPS, Galileo, BeiDou (with its GEO
//! satellites' rotation), QZSS and NavIC; the GLONASS ICD's equations of
//! motion, integrated by fourth-order Runge-Kutta in 60 s steps; and the
//! SBAS message's quadratic. Positions are ECEF in metres, in each
//! system's own frame (WGS 84, GTRF, CGCS2000, PZ-90.11), which agree to a
//! few centimetres. They're good to a few metres, which is plenty for
//! elevations: they are not precise orbits.
//!
//! Times are GPS time throughout.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use crate::obs::Sv;
use crate::signal::SPEED_OF_LIGHT;
use crate::time::GnssTime;

/// A Keplerian broadcast ephemeris (GPS, Galileo, BeiDou, QZSS, NavIC).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kepler {
    /// The satellite.
    pub sv: Sv,
    /// The clock's reference time, GPS time.
    pub toc: GnssTime,
    /// The ephemeris's reference time, GPS time.
    pub toe: GnssTime,
    /// The reference time as the system's own seconds of week, which the
    /// longitude of the node is reckoned from.
    pub toe_seconds: f64,
    /// Clock offset (s), drift (s/s) and drift rate (s/s²).
    pub af: [f64; 3],
    /// Square root of the semi-major axis, √m.
    pub sqrt_a: f64,
    /// Eccentricity.
    pub e: f64,
    /// Inclination at the reference time, radians.
    pub i0: f64,
    /// Rate of inclination, rad/s.
    pub idot: f64,
    /// Longitude of the ascending node at the week's start, radians.
    pub omega0: f64,
    /// Rate of right ascension, rad/s.
    pub omega_dot: f64,
    /// Argument of perigee, radians.
    pub omega: f64,
    /// Mean anomaly at the reference time, radians.
    pub m0: f64,
    /// Mean motion difference, rad/s.
    pub delta_n: f64,
    /// Harmonic corrections: latitude (rad), radius (m), inclination (rad),
    /// cosine then sine.
    pub cuc: f64,
    /// See `cuc`.
    pub cus: f64,
    /// See `cuc`.
    pub crc: f64,
    /// See `cuc`.
    pub crs: f64,
    /// See `cuc`.
    pub cic: f64,
    /// See `cuc`.
    pub cis: f64,
    /// The health field; 0 is healthy (for QZSS, ignoring bit 0).
    pub health: u32,
    /// When the message was sent, GPS time.
    pub transmitted: GnssTime,
    /// Galileo: from the F/NAV message, whose clock is for the E1-E5a
    /// combination; I/NAV's is for E1-E5b.
    pub fnav: bool,
}

/// A position-velocity ephemeris (GLONASS, SBAS), in ECEF metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StateVector {
    /// The satellite.
    pub sv: Sv,
    /// The reference time, GPS time.
    pub toe: GnssTime,
    /// Clock offset (s) and drift (s/s). For GLONASS, −τn and γn.
    pub clock: [f64; 2],
    /// Position, m.
    pub position: [f64; 3],
    /// Velocity, m/s.
    pub velocity: [f64; 3],
    /// Acceleration from the Moon and Sun (GLONASS) or in all (SBAS), m/s².
    pub acceleration: [f64; 3],
    /// The health field; 0 is healthy.
    pub health: u32,
    /// GLONASS: the frequency channel, −7 to 6.
    pub channel: Option<i8>,
}

/// A broadcast ephemeris.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ephemeris {
    /// GPS, Galileo, BeiDou, QZSS, NavIC.
    Kepler(Kepler),
    /// GLONASS.
    Glonass(StateVector),
    /// SBAS.
    Sbas(StateVector),
}

/// Where a satellite is, and its clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SatPosition {
    /// ECEF, metres.
    pub position: [f64; 3],
    /// The clock's offset from system time, seconds, with the relativistic
    /// correction for a Keplerian orbit's eccentricity.
    pub clock: f64,
    /// Whether the ephemeris says the satellite is healthy.
    pub healthy: bool,
}

/// GPS's (and QZSS's and NavIC's) gravitational constant, m³/s².
const MU_GPS: f64 = 3.986_005e14;
/// Galileo's and BeiDou's.
const MU_GAL: f64 = 3.986_004_418e14;
/// GPS's and Galileo's Earth rotation rate, rad/s.
const OMEGA_E_GPS: f64 = 7.292_115_146_7e-5;
/// BeiDou's.
const OMEGA_E_BDS: f64 = 7.292_115e-5;
/// GLONASS (PZ-90): gravitational constant, J2, equatorial radius and
/// rotation rate.
const MU_GLO: f64 = 3.986_004_4e14;
const J2_GLO: f64 = 1.082_625_7e-3;
const RE_GLO: f64 = 6_378_136.0;
const OMEGA_E_GLO: f64 = 7.292_115e-5;
/// The GLONASS integration step, seconds.
const GLONASS_STEP: f64 = 60.0;

impl Ephemeris {
    /// The satellite.
    pub fn sv(&self) -> Sv {
        match self {
            Self::Kepler(k) => k.sv,
            Self::Glonass(s) | Self::Sbas(s) => s.sv,
        }
    }

    /// The reference time, GPS time.
    pub fn toe(&self) -> GnssTime {
        match self {
            Self::Kepler(k) => k.toe,
            Self::Glonass(s) | Self::Sbas(s) => s.toe,
        }
    }

    /// How far from its reference time, in seconds, the ephemeris is used.
    pub fn validity(&self) -> f64 {
        match self.sv().system {
            'E' => 14_400.0,
            'C' => 21_600.0,
            'R' => 1_800.0,
            'S' => 360.0,
            _ => 7_200.0,
        }
    }

    /// The satellite's position and clock at `t`, GPS time, if that's
    /// within the ephemeris's validity.
    pub fn position(&self, t: GnssTime) -> Option<SatPosition> {
        if t.seconds_since(self.toe()).abs() > self.validity() {
            return None;
        }
        Some(match self {
            Self::Kepler(k) => k.position(t),
            Self::Glonass(s) => s.glonass(t),
            Self::Sbas(s) => s.sbas(t),
        })
    }
}

impl Kepler {
    fn position(&self, t: GnssTime) -> SatPosition {
        let (mu, omega_e) = match self.sv.system {
            'E' => (MU_GAL, OMEGA_E_GPS),
            'C' => (MU_GAL, OMEGA_E_BDS),
            _ => (MU_GPS, OMEGA_E_GPS),
        };
        let a = self.sqrt_a * self.sqrt_a;
        let tk = t.seconds_since(self.toe);
        let n = libm::sqrt(mu / (a * a * a)) + self.delta_n;
        let m = self.m0 + n * tk;
        // Kepler's equation, by Newton's method.
        let mut big_e = m;
        for _ in 0..30 {
            let step = (big_e - self.e * libm::sin(big_e) - m) / (1.0 - self.e * libm::cos(big_e));
            big_e -= step;
            if step.abs() < 1e-13 {
                break;
            }
        }
        let (sin_e, cos_e) = (libm::sin(big_e), libm::cos(big_e));
        let nu = libm::atan2(libm::sqrt(1.0 - self.e * self.e) * sin_e, cos_e - self.e);
        let phi = nu + self.omega;
        let (sin2, cos2) = (libm::sin(2.0 * phi), libm::cos(2.0 * phi));
        let u = phi + self.cus * sin2 + self.cuc * cos2;
        let r = a * (1.0 - self.e * cos_e) + self.crs * sin2 + self.crc * cos2;
        let i = self.i0 + self.idot * tk + self.cis * sin2 + self.cic * cos2;
        let (x, y) = (r * libm::cos(u), r * libm::sin(u));
        let (sin_i, cos_i) = (libm::sin(i), libm::cos(i));
        let geo = self.sv.system == 'C' && (self.sv.prn <= 5 || self.sv.prn >= 59);
        let position = if geo {
            // BeiDou GEO: the orbit in an inertial frame, then rotated by
            // the Earth's turn since the reference time and by −5° about x.
            let node = self.omega0 + self.omega_dot * tk - omega_e * self.toe_seconds;
            let (sin_o, cos_o) = (libm::sin(node), libm::cos(node));
            let xg = x * cos_o - y * cos_i * sin_o;
            let yg = x * sin_o + y * cos_i * cos_o;
            let zg = y * sin_i;
            let (sin_t, cos_t) = (libm::sin(omega_e * tk), libm::cos(omega_e * tk));
            let (sin5, cos5) = (libm::sin(-5f64.to_radians()), libm::cos(-5f64.to_radians()));
            [
                xg * cos_t + yg * sin_t * cos5 + zg * sin_t * sin5,
                -xg * sin_t + yg * cos_t * cos5 + zg * cos_t * sin5,
                -yg * sin5 + zg * cos5,
            ]
        } else {
            let node = self.omega0 + (self.omega_dot - omega_e) * tk - omega_e * self.toe_seconds;
            let (sin_o, cos_o) = (libm::sin(node), libm::cos(node));
            [
                x * cos_o - y * cos_i * sin_o,
                x * sin_o + y * cos_i * cos_o,
                y * sin_i,
            ]
        };
        let dt = t.seconds_since(self.toc);
        let clock = self.af[0] + self.af[1] * dt + self.af[2] * dt * dt
            - 2.0 * libm::sqrt(mu * a) * self.e * sin_e / (SPEED_OF_LIGHT * SPEED_OF_LIGHT);
        // QZSS's lowest bit is its L6 signal's health, not used here.
        let health = if self.sv.system == 'J' {
            self.health & !1
        } else {
            self.health
        };
        SatPosition {
            position,
            clock,
            healthy: health == 0,
        }
    }
}

impl StateVector {
    fn glonass(&self, t: GnssTime) -> SatPosition {
        let mut dt = t.seconds_since(self.toe);
        let clock = self.clock[0] + self.clock[1] * dt;
        let mut x = [
            self.position[0],
            self.position[1],
            self.position[2],
            self.velocity[0],
            self.velocity[1],
            self.velocity[2],
        ];
        let step = if dt < 0.0 {
            -GLONASS_STEP
        } else {
            GLONASS_STEP
        };
        while dt.abs() > 1e-9 {
            let h = if dt.abs() < GLONASS_STEP { dt } else { step };
            runge_kutta(&mut x, h, &self.acceleration);
            dt -= h;
        }
        SatPosition {
            position: [x[0], x[1], x[2]],
            clock,
            healthy: self.health == 0,
        }
    }

    fn sbas(&self, t: GnssTime) -> SatPosition {
        let dt = t.seconds_since(self.toe);
        let mut position = self.position;
        for ((p, v), a) in position
            .iter_mut()
            .zip(self.velocity)
            .zip(self.acceleration)
        {
            *p += v * dt + a * dt * dt / 2.0;
        }
        SatPosition {
            position,
            clock: self.clock[0] + self.clock[1] * dt,
            healthy: self.health == 0,
        }
    }
}

/// The GLONASS equations of motion: the state's rate of change.
fn glonass_rate(x: &[f64; 6], acc: &[f64; 3]) -> [f64; 6] {
    let r2 = x[0] * x[0] + x[1] * x[1] + x[2] * x[2];
    let r3 = r2 * libm::sqrt(r2);
    let w2 = OMEGA_E_GLO * OMEGA_E_GLO;
    let a = 1.5 * J2_GLO * MU_GLO * RE_GLO * RE_GLO / r2 / r3;
    let b = 5.0 * x[2] * x[2] / r2;
    let c = -MU_GLO / r3 - a * (1.0 - b);
    [
        x[3],
        x[4],
        x[5],
        (c + w2) * x[0] + 2.0 * OMEGA_E_GLO * x[4] + acc[0],
        (c + w2) * x[1] - 2.0 * OMEGA_E_GLO * x[3] + acc[1],
        (c - 2.0 * a) * x[2] + acc[2],
    ]
}

/// One fourth-order Runge-Kutta step of `h` seconds.
fn runge_kutta(x: &mut [f64; 6], h: f64, acc: &[f64; 3]) {
    let along = |k: &[f64; 6], f: f64| {
        let mut w = *x;
        for (wi, ki) in w.iter_mut().zip(k) {
            *wi += ki * f;
        }
        w
    };
    let k1 = glonass_rate(x, acc);
    let k2 = glonass_rate(&along(&k1, h / 2.0), acc);
    let k3 = glonass_rate(&along(&k2, h / 2.0), acc);
    let k4 = glonass_rate(&along(&k3, h), acc);
    let ks = k1.iter().zip(k2).zip(k3).zip(k4);
    for (xi, (((a, b), c), d)) in x.iter_mut().zip(ks) {
        *xi += (a + 2.0 * b + 2.0 * c + d) * h / 6.0;
    }
}

/// Ephemerides by satellite, to find the right one for a time.
#[derive(Debug, Clone, Default)]
pub struct Orbits {
    by_sv: BTreeMap<Sv, Vec<Ephemeris>>,
}

impl Orbits {
    /// Indexes ephemerides, from any number of navigation files. Of
    /// several for a satellite with the same reference time, one is kept:
    /// Galileo's I/NAV before its F/NAV, then the last sent (or, for
    /// GLONASS and SBAS, the last read).
    pub fn new(ephemerides: impl IntoIterator<Item = Ephemeris>) -> Self {
        let mut by_sv: BTreeMap<Sv, Vec<(usize, Ephemeris)>> = BTreeMap::new();
        for (i, e) in ephemerides.into_iter().enumerate() {
            by_sv.entry(e.sv()).or_default().push((i, e));
        }
        let by_sv = by_sv
            .into_iter()
            .map(|(sv, mut list)| {
                list.sort_by_key(|(i, e)| {
                    let preference = match e {
                        Ephemeris::Kepler(k) => (k.fnav, Reverse(k.transmitted.ticks())),
                        _ => (false, Reverse(0)),
                    };
                    (e.toe(), preference, Reverse(*i))
                });
                list.dedup_by_key(|(_, e)| e.toe());
                (sv, list.into_iter().map(|(_, e)| e).collect())
            })
            .collect();
        Self { by_sv }
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.by_sv.is_empty()
    }

    /// The GLONASS satellites' frequency channels, by slot.
    pub fn glonass_channels(&self) -> BTreeMap<u8, i8> {
        self.by_sv
            .values()
            .flatten()
            .filter_map(|e| match e {
                Ephemeris::Glonass(s) => Some((s.sv.prn, s.channel?)),
                _ => None,
            })
            .collect()
    }

    /// The satellites with ephemerides.
    pub fn satellites(&self) -> impl Iterator<Item = Sv> + '_ {
        self.by_sv.keys().copied()
    }

    /// The ephemeris to use for a satellite at `t`, if one is valid then:
    /// the one whose reference time is nearest, the later of two equally
    /// near. For Galileo, whose ephemerides are only good after their
    /// reference time, the latest at or before `t`.
    pub fn ephemeris(&self, sv: Sv, t: GnssTime) -> Option<&Ephemeris> {
        let list = self.by_sv.get(&sv)?;
        let chosen = if sv.system == 'E' {
            list.get(list.partition_point(|e| e.toe() <= t).checked_sub(1)?)?
        } else {
            // The first at or after t, and the last before it.
            let i = list.partition_point(|e| e.toe() < t);
            let before = i.checked_sub(1).and_then(|j| list.get(j));
            match (before, list.get(i)) {
                (Some(a), Some(b)) => {
                    if t.seconds_since(a.toe()) < b.toe().seconds_since(t) {
                        a
                    } else {
                        b
                    }
                }
                (Some(a), None) => a,
                (None, Some(b)) => b,
                (None, None) => return None,
            }
        };
        (t.seconds_since(chosen.toe()).abs() <= chosen.validity()).then_some(chosen)
    }

    /// A satellite's position at `t`, GPS time, from the ephemeris to use
    /// then.
    pub fn position(&self, sv: Sv, t: GnssTime) -> Option<SatPosition> {
        self.ephemeris(sv, t)?.position(t)
    }
}

/// A receiver's place, for looking at satellites from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Station {
    position: [f64; 3],
    /// East, north and up unit vectors in ECEF.
    enu: [[f64; 3]; 3],
}

impl Station {
    /// At an ECEF position, metres. `None` near the Earth's centre, where
    /// there's no up.
    pub fn new(position: [f64; 3]) -> Option<Self> {
        let [x, y, z] = position;
        let p = libm::sqrt(x * x + y * y);
        if libm::sqrt(p * p + z * z) < 1.0e6 {
            return None;
        }
        // Geodetic latitude on WGS 84, by iteration.
        let (a, f) = (6_378_137.0, 1.0 / 298.257_223_563);
        let e2 = f * (2.0 - f);
        let mut lat = libm::atan2(z, p * (1.0 - e2));
        for _ in 0..10 {
            let s = libm::sin(lat);
            let n = a / libm::sqrt(1.0 - e2 * s * s);
            lat = libm::atan2(z + e2 * n * s, p);
        }
        let lon = libm::atan2(y, x);
        let (sl, cl) = (libm::sin(lat), libm::cos(lat));
        let (so, co) = (libm::sin(lon), libm::cos(lon));
        Some(Self {
            position,
            enu: [
                [-so, co, 0.0],
                [-sl * co, -sl * so, cl],
                [cl * co, cl * so, sl],
            ],
        })
    }

    /// A satellite's azimuth (clockwise from north, 0 to 360) and elevation,
    /// in degrees.
    pub fn look(&self, satellite: [f64; 3]) -> (f64, f64) {
        let d = [
            satellite[0] - self.position[0],
            satellite[1] - self.position[1],
            satellite[2] - self.position[2],
        ];
        let dot = |u: &[f64; 3]| u[0] * d[0] + u[1] * d[1] + u[2] * d[2];
        let [east, north, up] = self.enu.each_ref().map(dot);
        let azimuth = libm::atan2(east, north).to_degrees().rem_euclid(360.0);
        let elevation = libm::atan2(up, libm::sqrt(east * east + north * north)).to_degrees();
        (azimuth, elevation)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::Station;

    #[test]
    fn looking_up_and_along_the_horizon() {
        // On the equator at 0° E; a point straight up, and one due north
        // on the horizon plane.
        let s = Station::new([6_378_137.0, 0.0, 0.0]).unwrap_or_else(|| unreachable!());
        let (_, el) = s.look([26_000_000.0, 0.0, 0.0]);
        assert!((el - 90.0).abs() < 1e-9);
        let (az, el) = s.look([6_378_137.0, 0.0, 1.0e6]);
        assert!(az.abs() < 1e-9 && el.abs() < 1e-9);
        let (az, _) = s.look([6_378_137.0, 1.0e6, 0.0]);
        assert!((az - 90.0).abs() < 1e-9);
        assert!(Station::new([0.0, 0.0, 10.0]).is_none());
    }
}
