//! The grid readers and grid transformations against PROJ and GeodePy: ICSM's
//! GDA94 to GDA2020 conformal and distortion grid, and AUSGeoid2020 with its
//! uncertainty.
//!
//! The published grids are too big to commit, so `tests/data/grids` holds
//! small windows of them around the test points, written by
//! `tests/oracle/generate_grids.py`: NTv2 files, and GeoTIFF files written by
//! GDAL in PROJ's conventions. The reference values come from the full
//! published grids (versions in each CSV's first line), and the generator
//! checks PROJ gets the same values from the windows. Everything is compiled
//! in, so these tests also run in the browser.
//!
//! `published_grids_match_proj` reads the published files themselves, from
//! the directory `GNSS_GRID_DIR` names, and does nothing if it isn't set.

#![allow(clippy::unwrap_used)]

use std::f64::consts::PI;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex};

use gnss_datum::grid::{Grid, GridSource};
use gnss_datum::gridshift::HorizontalShiftGrid;
use gnss_datum::transform::gda94_to_gda2020_conformal_and_distortion;
use gnss_datum::{FrameId, Geodetic, GeoidModel, Position, TransformationGraph};
use nalgebra::Vector3;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const ARC_SECOND: f64 = PI / 648_000.0;

/// PROJ converts each node's shift to radians in single precision
/// (`valueAt` of its NTv2 and GeoTIFF grids, in `grids.cpp`), which rounds it
/// to about 1 part in 17 million: 0.1 µm of a 1.5 m shift. We keep double
/// precision, so the two agree to a few tenths of a micrometre.
const PROJ_SHIFT_M: f64 = 5e-7;

/// GeodePy rounds shifts and accuracies to 1e-6 arc-seconds, so it can be
/// half that from us, plus a hair for rounding in either implementation.
const GEODEPY_ARC_SECONDS: f64 = 5e-7 + 1e-9;

/// The NTv2 and GeoTIFF versions of AUSGeoid2020 state their limits in
/// arc-seconds and degrees, which place nodes a hair apart: worth about
/// 1e-10 m of N.
const PROJ_N_M: f64 = 1e-9;

const ICSM_GRIDS: [(&str, &[u8]); 2] = [
    (
        "gda94_gda2020_crop.gsb",
        include_bytes!("data/grids/gda94_gda2020_crop.gsb"),
    ),
    (
        "gda94_gda2020_crop.tif",
        include_bytes!("data/grids/gda94_gda2020_crop.tif"),
    ),
];

const GEOID_GRIDS: [(&str, &[u8]); 2] = [
    (
        "ausgeoid2020_crop.gsb",
        include_bytes!("data/grids/ausgeoid2020_crop.gsb"),
    ),
    (
        "ausgeoid2020_crop.tif",
        include_bytes!("data/grids/ausgeoid2020_crop.tif"),
    ),
];

const GEOID_UNCERTAINTY: &[u8] = include_bytes!("data/grids/ausgeoid2020_error_crop.gsb");

/// Data rows of a generated CSV file, as numbers.
fn rows(text: &'static str) -> impl Iterator<Item = Vec<f64>> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .skip(1)
        .map(|l| l.split(',').map(|f| f.parse().unwrap()).collect())
}

fn grid(name: &str, bytes: &'static [u8]) -> Grid {
    Grid::open(name, Arc::new(bytes)).unwrap()
}

/// The ground distance of a small difference in latitude and longitude, in degrees.
fn ground_m(lat_deg: f64, d_lat_deg: f64, d_lon_deg: f64) -> f64 {
    let north = d_lat_deg.to_radians() * 6_371_000.0;
    let east = d_lon_deg.to_radians() * 6_371_000.0 * libm::cos(lat_deg.to_radians());
    libm::hypot(north, east)
}

/// The worst forward and reverse differences from PROJ, in metres, over rows
/// of latitude, longitude, the GDA2020 latitude and longitude, and the GDA94
/// latitude and longitude from the first two read as GDA2020.
fn compare_shifts(
    g: &HorizontalShiftGrid,
    rows: impl Iterator<Item = [f64; 6]>,
) -> (f64, f64, usize) {
    let (mut forward, mut reverse, mut count) = (0.0_f64, 0.0_f64, 0);
    for [lat, lon, lat_2020, lon_2020, lat_94, lon_94] in rows {
        let (a, b, _) = g.forward(lat.to_radians(), lon.to_radians()).unwrap();
        forward = forward.max(ground_m(
            lat,
            a.to_degrees() - lat_2020,
            b.to_degrees() - lon_2020,
        ));
        let (c, d, _) = g.inverse(lat.to_radians(), lon.to_radians()).unwrap();
        reverse = reverse.max(ground_m(
            lat,
            c.to_degrees() - lat_94,
            d.to_degrees() - lon_94,
        ));
        count += 1;
    }
    (forward, reverse, count)
}

#[test]
fn gda94_to_gda2020_shifts_match_proj() {
    for (name, bytes) in ICSM_GRIDS {
        let g = HorizontalShiftGrid::new(grid(name, bytes)).unwrap();
        let (forward, reverse, count) = compare_shifts(
            &g,
            rows(include_str!("data/grids/gda94_gda2020.csv"))
                .map(|r| [r[0], r[1], r[3], r[4], r[9], r[10]]),
        );
        eprintln!(
            "{name} vs PROJ ({count} points): forward {forward:.1e} m, reverse {reverse:.1e} m"
        );
        assert_eq!(count, 49);
        assert!(
            forward < PROJ_SHIFT_M,
            "forward differs by up to {forward} m"
        );
        assert!(
            reverse < PROJ_SHIFT_M,
            "reverse differs by up to {reverse} m"
        );
    }
}

#[test]
fn gda94_to_gda2020_shifts_and_accuracies_match_geodepy() {
    // GeodePy read the NTv2 test grid with its own reader. It rounds to
    // 1e-6 arc-seconds (0.03 mm).
    let g = HorizontalShiftGrid::new(grid(ICSM_GRIDS[0].0, ICSM_GRIDS[0].1)).unwrap();
    let (mut worst_shift, mut worst_accuracy, mut with_accuracy) = (0.0_f64, 0.0_f64, 0);
    for r in rows(include_str!("data/grids/gda94_gda2020.csv")) {
        let s = g.shift_at(r[0].to_radians(), r[1].to_radians()).unwrap();
        worst_shift = worst_shift
            .max((s.latitude_rad / ARC_SECOND - r[11]).abs())
            .max((s.longitude_rad / ARC_SECOND - r[12]).abs());
        // Where any node gives no accuracy, GeodePy interpolates its -1 anyway.
        if let (Some(a), Some(b)) = (s.latitude_sigma_rad, s.longitude_sigma_rad) {
            worst_accuracy = worst_accuracy
                .max((a / ARC_SECOND - r[13]).abs())
                .max((b / ARC_SECOND - r[14]).abs());
            with_accuracy += 1;
        }
    }
    eprintln!(
        "GDA94 to GDA2020 vs GeodePy: shifts {worst_shift:.1e}\", accuracies {worst_accuracy:.1e}\" \
         ({with_accuracy} points with accuracies)"
    );
    assert!(with_accuracy >= 16);
    assert!(
        worst_shift <= GEODEPY_ARC_SECONDS,
        "shifts differ by up to {worst_shift}\""
    );
    assert!(
        worst_accuracy <= GEODEPY_ARC_SECONDS,
        "accuracies differ by up to {worst_accuracy}\""
    );
}

#[test]
fn gda94_to_gda2020_positions_match_proj() {
    // PROJ's grid for latitude and longitude, and its EPSG:8048 for
    // ellipsoidal heights, as the grid transformation defines them.
    for (name, bytes) in ICSM_GRIDS {
        let shift = Arc::new(HorizontalShiftGrid::new(grid(name, bytes)).unwrap());
        let graph = TransformationGraph::builtin().with_preferred(vec![
            gda94_to_gda2020_conformal_and_distortion(shift).unwrap(),
        ]);
        let mut worst: f64 = 0.0;
        for r in rows(include_str!("data/grids/gda94_gda2020.csv")) {
            let g = Geodetic::from_degrees(r[0], r[1], r[2]).unwrap();
            let p = Position::from_geodetic(FrameId::Gda94, None, &g, None).unwrap();
            let t = graph.transform(&p, FrameId::Gda2020, None).unwrap();
            assert_eq!(t.steps[0].transformation, "GDA94 to GDA2020 (2)");
            worst = worst.max((t.position.xyz() - Vector3::new(r[6], r[7], r[8])).norm());
        }
        eprintln!("{name}: GDA94 to GDA2020 positions vs PROJ: worst {worst:.1e} m");
        assert!(worst < PROJ_SHIFT_M, "positions differ by up to {worst} m");
    }
}

#[test]
fn ausgeoid2020_matches_proj() {
    for (name, bytes) in GEOID_GRIDS {
        let uncertainty = grid("ausgeoid2020_error_crop.gsb", GEOID_UNCERTAINTY);
        let model = GeoidModel::ausgeoid2020(grid(name, bytes), Some(uncertainty)).unwrap();
        let (mut worst_n, mut worst_sigma, mut count) = (0.0_f64, 0.0_f64, 0);
        for r in rows(include_str!("data/grids/ausgeoid2020.csv")) {
            let s = model
                .separation(r[0].to_radians(), r[1].to_radians())
                .unwrap();
            worst_n = worst_n.max((s.n_m - r[2]).abs());
            worst_sigma = worst_sigma.max((s.sigma_m.unwrap() - r[3]).abs());
            count += 1;
        }
        eprintln!("{name} vs PROJ ({count} points): N {worst_n:.1e} m, sigma {worst_sigma:.1e} m");
        assert_eq!(count, 33);
        assert!(worst_n < PROJ_N_M, "N differs by up to {worst_n} m");
        assert!(worst_sigma < 1e-9, "sigma differs by up to {worst_sigma} m");
    }
}

/// A file on disk, read when asked: what a host gives the core.
struct FileSource {
    file: Mutex<File>,
    size: u64,
}

impl GridSource for FileSource {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let mut file = self.file.lock().unwrap();
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(buf)
    }
}

fn published(dir: &Path, name: &str) -> Option<Grid> {
    let file = File::open(dir.join(name)).ok()?;
    let size = file.metadata().ok()?.len();
    let source = FileSource {
        file: Mutex::new(file),
        size,
    };
    Some(Grid::open(name, Arc::new(source)).unwrap())
}

#[test]
fn published_grids_match_proj() {
    let Ok(dir) = std::env::var("GNSS_GRID_DIR") else {
        eprintln!("GNSS_GRID_DIR isn't set, so the published grids aren't tested");
        return;
    };
    let dir = Path::new(&dir);
    let mut tested = Vec::new();

    for name in [
        "au_icsm_GDA94_GDA2020_conformal_and_distortion.tif",
        "GDA94_GDA2020_conformal_and_distortion.gsb",
    ] {
        let Some(g) = published(dir, name) else {
            continue;
        };
        let g = HorizontalShiftGrid::new(g).unwrap();
        let (forward, reverse, count) = compare_shifts(
            &g,
            rows(include_str!("data/grids/gda94_gda2020_full.csv"))
                .map(|r| [r[0], r[1], r[2], r[3], r[4], r[5]]),
        );
        eprintln!(
            "{name} vs PROJ ({count} points): forward {forward:.1e} m, reverse {reverse:.1e} m"
        );
        assert!(forward < PROJ_SHIFT_M && reverse < PROJ_SHIFT_M);
        tested.push(name);
    }

    let uncertainty = || published(dir, "AUSGeoid2020_20180201_error.gsb");
    for name in [
        "AUSGeoid2020_20180201.gsb",
        "au_ga_AUSGeoid2020_20180201.tif",
    ] {
        let Some(g) = published(dir, name) else {
            continue;
        };
        let model = GeoidModel::ausgeoid2020(g, uncertainty()).unwrap();
        let (mut worst_n, mut worst_sigma, mut sigmas) = (0.0_f64, 0.0_f64, 0);
        for r in rows(include_str!("data/grids/ausgeoid2020_full.csv")) {
            let s = model
                .separation(r[0].to_radians(), r[1].to_radians())
                .unwrap();
            worst_n = worst_n.max((s.n_m - r[2]).abs());
            if let Some(sigma) = s.sigma_m {
                worst_sigma = worst_sigma.max((sigma - r[3]).abs());
                sigmas += 1;
            }
        }
        // Without the uncertainty grid there's no sigma to check, and the
        // output says so rather than reporting a perfect zero.
        let sigma_checked = if model.uncertainty_grid().is_some() {
            assert_eq!(sigmas, 400, "every point should have a sigma");
            format!("sigma {worst_sigma:.1e} m at {sigmas} points")
        } else {
            "sigma not checked (no uncertainty grid)".to_owned()
        };
        eprintln!("{name} vs PROJ: N {worst_n:.1e} m, {sigma_checked}");
        assert!(worst_n < PROJ_N_M && worst_sigma < 1e-9);
        tested.push(name);
    }
    assert!(
        !tested.is_empty(),
        "GNSS_GRID_DIR is set but holds none of the published grids"
    );
}
