//! Bit-for-bit determinism across platforms (D-024).
//!
//! This hashes the exact bits of a few thousand conversions, projections and
//! transformations. CI runs it natively on Linux, Windows and macOS and in
//! headless Chrome, and the hash must be the same everywhere: that's the
//! evidence that the CLI and the web app give identical results.
//!
//! A deliberate change to the maths changes the hash. When that happens,
//! check that the oracle tests still pass, then update `GOLDEN`.

#![allow(clippy::unwrap_used)]

use gnss_datum::hifitime::{Duration, Epoch, TimeScale};
use gnss_datum::tm::MgaZone;
use gnss_datum::{CoordinateEpoch, Covariance, FrameId, GRS80, Geodetic, Position, transform};
use nalgebra::Matrix3;
use sha2::{Digest, Sha256};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const GOLDEN: &str = "a4ae1fe3b82403889af4b97cba8170afe3043df86c3b65c998ef510aed40c8f1";

/// SplitMix64, for the same pseudo-random cases on every platform.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn uniform(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * ((self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64)
    }
}

#[test]
fn results_are_bit_identical_on_every_platform() {
    let mut hasher = Sha256::new();
    let mut put = |values: &[f64]| {
        for v in values {
            hasher.update(v.to_bits().to_le_bytes());
        }
    };
    let mut rng = SplitMix64(2026);
    let base = Epoch::from_gregorian_at_midnight(1995, 1, 1, TimeScale::GPST);

    for _ in 0..2_000 {
        let g = Geodetic::from_degrees(
            rng.uniform(-60.0, -9.0),
            rng.uniform(95.0, 170.0),
            rng.uniform(-50.0, 3000.0),
        )
        .unwrap();

        // Geodetic and ECEF.
        let xyz = GRS80.geodetic_to_ecef(&g);
        let back = GRS80.ecef_to_geodetic(&xyz).unwrap();
        put(&[xyz.x, xyz.y, xyz.z]);
        put(&[
            back.latitude_rad(),
            back.longitude_rad(),
            back.ellipsoidal_height_m(),
        ]);

        // MGA, both ways.
        let tm = MgaZone::containing(g.longitude_rad()).projection();
        let p = tm.forward(&g).unwrap();
        put(&[
            p.easting_m,
            p.northing_m,
            p.point_scale_factor,
            p.grid_convergence_rad,
        ]);
        let (inverse, _) = tm.inverse(p.easting_m, p.northing_m).unwrap();
        put(&[inverse.latitude_rad(), inverse.longitude_rad()]);

        // A time to a coordinate epoch.
        let seconds = rng.uniform(0.0, 36.0 * 365.25 * 86_400.0);
        let time = base + Duration::from_total_nanoseconds((seconds * 1e9) as i128);
        let epoch = CoordinateEpoch::from_time(time).unwrap();
        put(&[epoch.decimal_year()]);

        // ITRF2020 at that epoch to GDA2020, with covariance propagation.
        let cov = Covariance::from_enu(
            Matrix3::new(9e-6, 2e-6, 1e-6, 2e-6, 16e-6, -3e-6, 1e-6, -3e-6, 81e-6),
            &g,
        )
        .unwrap();
        let itrf = Position::new(FrameId::Itrf2020, Some(epoch), xyz, Some(cov)).unwrap();
        let t = transform(&itrf, FrameId::Gda2020, None).unwrap();
        put(t.position.xyz().as_slice());
        put(t.position.covariance().unwrap().matrix().as_slice());
        put(t.transformation_covariance.as_slice());

        // GDA94 to GDA2020.
        let gda94 = Position::new(FrameId::Gda94, None, xyz, None).unwrap();
        put(transform(&gda94, FrameId::Gda2020, None)
            .unwrap()
            .position
            .xyz()
            .as_slice());
    }

    let hash: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        hash, GOLDEN,
        "results differ from the golden hash: either the maths changed (check the oracle \
         tests, then update GOLDEN) or this platform computes different bits (D-024)"
    );
}
