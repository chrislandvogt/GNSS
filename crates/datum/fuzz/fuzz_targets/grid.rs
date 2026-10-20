//! Any bytes given to the grid readers, as an NTv2 or GeoTIFF file, must give
//! a grid or an error, never a panic, a hang or a huge allocation. A grid that
//! opens is then read at points in and around each subgrid, as a shift grid
//! both ways and as a geoid model.

#![no_main]

use std::sync::Arc;

use gnss_datum::GeoidModel;
use gnss_datum::grid::Grid;
use gnss_datum::gridshift::HorizontalShiftGrid;
use libfuzzer_sys::fuzz_target;

fn open(data: &[u8]) -> Option<Grid> {
    Grid::open("fuzz", Arc::new(data.to_vec())).ok()
}

fuzz_target!(|data: &[u8]| {
    let Some(grid) = open(data) else { return };
    let points: Vec<(f64, f64)> = grid
        .subgrids()
        .iter()
        .take(16)
        .flat_map(|s| {
            let (north, east) = (s.north_rad(), s.east_rad());
            let (mid_lat, mid_lon) = ((s.south_rad + north) / 2.0, (s.west_rad + east) / 2.0);
            [
                (s.south_rad, s.west_rad),
                (north, east),
                (mid_lat, mid_lon),
                (north + s.latitude_step_rad, mid_lon),
            ]
        })
        .collect();
    for &(lat, lon) in &points {
        if let Ok(cell) = grid.cell(lat, lon) {
            for band in 0..grid.bands() {
                let _ = cell.bilinear(band);
            }
        }
    }
    if let Some(shift) = open(data).and_then(|g| HorizontalShiftGrid::new(g).ok()) {
        for &(lat, lon) in &points {
            let _ = shift.forward(lat, lon);
            let _ = shift.inverse(lat, lon);
        }
    }
    if let Ok(geoid) = GeoidModel::ausgeoid2020(grid, open(data)) {
        for &(lat, lon) in &points {
            let _ = geoid.separation(lat, lon);
        }
    }
});
