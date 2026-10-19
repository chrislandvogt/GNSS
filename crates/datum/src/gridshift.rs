//! Horizontal shift grids: latitude and longitude offsets from one frame to
//! another, interpolated from an NTv2 file or a PROJ `HORIZONTAL_OFFSET`
//! GeoTIFF and applied as EPSG's NTv2 method (9615) does.
//!
//! The reverse direction has no closed form. It's found by iteration, as PROJ
//! does (`pj_hgrid_apply_internal` in `grids.cpp`), so a shift and its reverse
//! return to the start to well under a micrometre.

use std::f64::consts::PI;

use crate::error::DatumError;
use crate::grid::{Grid, GridFormat, invalid};

/// The reverse iteration stops when a step moves the point less than this, in
/// radians (about 6 µm). The value is PROJ's; the shift's gradient is so small
/// that the point is then within a picometre of the answer.
const INVERSE_TOLERANCE_RAD: f64 = 1e-12;

/// The most iterations for the reverse direction, as in PROJ. Two or three are
/// needed in practice.
const INVERSE_ITERATIONS: usize = 10;

/// The shift at a point, and its uncertainty where the grid gives one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shift {
    /// Shift in latitude, in radians, positive north.
    pub latitude_rad: f64,
    /// Shift in longitude, in radians, positive east.
    pub longitude_rad: f64,
    /// One-sigma uncertainty of the latitude shift, in radians, if the grid gives one here.
    pub latitude_sigma_rad: Option<f64>,
    /// One-sigma uncertainty of the longitude shift, in radians, if the grid gives one here.
    pub longitude_sigma_rad: Option<f64>,
    /// The subgrid the shift came from, as an index into [`Grid::subgrids`].
    pub subgrid: usize,
}

/// Which bands of a grid hold what, and their units.
#[derive(Debug, Clone, Copy)]
struct Bands {
    latitude: usize,
    longitude: usize,
    accuracy: Option<(usize, usize)>,
    /// Radians per unit of the shift bands.
    unit_rad: f64,
    /// Radians per unit of the accuracy bands.
    accuracy_unit_rad: f64,
    /// 1 if the longitude shift is positive east, -1 if positive west.
    longitude_sign: f64,
}

/// A grid of latitude and longitude shifts from one frame to another.
#[derive(Debug)]
pub struct HorizontalShiftGrid {
    grid: Grid,
    bands: Bands,
}

/// Radians per unit of an angle unit as PROJ's GeoTIFF grids name it.
fn angle_unit(name: Option<&str>) -> Option<f64> {
    match name {
        Some("arc-second") => Some(PI / 648_000.0),
        Some("degree") => Some(PI / 180.0),
        Some("radian") => Some(1.0),
        _ => None,
    }
}

impl HorizontalShiftGrid {
    /// Interprets a grid as horizontal shifts.
    ///
    /// - An NTv2 file has a latitude shift, a longitude shift (positive west)
    ///   and their accuracies, in the units of `GS_TYPE`. A negative accuracy
    ///   means none is given there.
    /// - A GeoTIFF must have `TYPE=HORIZONTAL_OFFSET` and bands described as
    ///   `latitude_offset` and `longitude_offset`, optionally with
    ///   `latitude_offset_accuracy` and `longitude_offset_accuracy`, in
    ///   arc-seconds, degrees or radians.
    pub fn new(grid: Grid) -> Result<Self, DatumError> {
        let bands = match grid.format() {
            GridFormat::Ntv2 => {
                let unit_rad = match grid.metadata().get("GS_TYPE").map(str::to_ascii_uppercase) {
                    Some(t) if t == "MINUTES" => PI / 10_800.0,
                    Some(t) if t == "DEGREES" => PI / 180.0,
                    _ => PI / 648_000.0,
                };
                Bands {
                    latitude: 0,
                    longitude: 1,
                    accuracy: Some((2, 3)),
                    unit_rad,
                    accuracy_unit_rad: unit_rad,
                    longitude_sign: -1.0,
                }
            }
            GridFormat::GeoTiff => Self::geotiff_bands(&grid)?,
        };
        Ok(HorizontalShiftGrid { grid, bands })
    }

    fn geotiff_bands(grid: &Grid) -> Result<Bands, DatumError> {
        let meta = grid.metadata();
        if meta.get("TYPE") != Some("HORIZONTAL_OFFSET") {
            return Err(invalid(
                grid.name(),
                format!(
                    "isn't a horizontal shift grid: its TYPE is {:?}, not HORIZONTAL_OFFSET",
                    meta.get("TYPE").unwrap_or("missing")
                ),
            ));
        }
        let find = |description: &str| {
            meta.bands
                .iter()
                .position(|b| b.description.as_deref() == Some(description))
        };
        let (Some(latitude), Some(longitude)) = (find("latitude_offset"), find("longitude_offset"))
        else {
            return Err(invalid(
                grid.name(),
                "has no bands described as latitude_offset and longitude_offset",
            ));
        };
        let unit_of = |band: usize| meta.bands[band].unit.as_deref();
        let unit_rad = angle_unit(unit_of(latitude))
            .filter(|u| angle_unit(unit_of(longitude)) == Some(*u))
            .ok_or_else(|| {
                invalid(
                    grid.name(),
                    "its offsets aren't in arc-seconds, degrees or radians",
                )
            })?;
        let accuracy = find("latitude_offset_accuracy").zip(find("longitude_offset_accuracy"));
        let accuracy_unit_rad = match accuracy {
            Some((a, b)) => angle_unit(unit_of(a))
                .filter(|u| angle_unit(unit_of(b)) == Some(*u))
                .ok_or_else(|| {
                    invalid(
                        grid.name(),
                        "its accuracies aren't in arc-seconds, degrees or radians",
                    )
                })?,
            None => unit_rad,
        };
        let longitude_sign = match meta.bands[longitude].get("positive_value") {
            Some("west") => -1.0,
            _ => 1.0,
        };
        Ok(Bands {
            latitude,
            longitude,
            accuracy,
            unit_rad,
            accuracy_unit_rad,
            longitude_sign,
        })
    }

    /// The underlying grid.
    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    /// Whether the grid covers a point.
    pub fn covers(&self, latitude_rad: f64, longitude_rad: f64) -> bool {
        self.grid.covers(latitude_rad, longitude_rad)
    }

    /// The shift at a point in the source frame.
    pub fn shift_at(&self, latitude_rad: f64, longitude_rad: f64) -> Result<Shift, DatumError> {
        let cell = self.grid.cell(latitude_rad, longitude_rad)?;
        let b = &self.bands;
        let lat = cell.bilinear(b.latitude);
        let lon = cell.bilinear(b.longitude);
        if !lat.is_finite() || !lon.is_finite() {
            return Err(self.grid.no_value(latitude_rad, longitude_rad));
        }
        // An accuracy is only meaningful if every node around the point has one.
        let sigma = |band: usize| {
            let known = cell.nodes(band).iter().all(|v| v.is_finite() && *v >= 0.0);
            known.then(|| cell.bilinear(band) * b.accuracy_unit_rad)
        };
        let (latitude_sigma_rad, longitude_sigma_rad) = match b.accuracy {
            Some((a_lat, a_lon)) => match (sigma(a_lat), sigma(a_lon)) {
                (Some(s_lat), Some(s_lon)) => (Some(s_lat), Some(s_lon)),
                _ => (None, None),
            },
            None => (None, None),
        };
        Ok(Shift {
            latitude_rad: lat * b.unit_rad,
            longitude_rad: lon * b.unit_rad * b.longitude_sign,
            latitude_sigma_rad,
            longitude_sigma_rad,
            subgrid: cell.subgrid,
        })
    }

    /// Shifts a point from the source frame to the target frame. Returns the
    /// new latitude and longitude, in radians, and the shift applied.
    pub fn forward(
        &self,
        latitude_rad: f64,
        longitude_rad: f64,
    ) -> Result<(f64, f64, Shift), DatumError> {
        let s = self.shift_at(latitude_rad, longitude_rad)?;
        Ok((
            latitude_rad + s.latitude_rad,
            longitude_rad + s.longitude_rad,
            s,
        ))
    }

    /// Shifts a point from the target frame back to the source frame, by
    /// iteration. Returns the latitude and longitude in the source frame, in
    /// radians, and the shift at that point (the forward shift, not its negative).
    pub fn inverse(
        &self,
        latitude_rad: f64,
        longitude_rad: f64,
    ) -> Result<(f64, f64, Shift), DatumError> {
        let first = self.shift_at(latitude_rad, longitude_rad)?;
        let mut lat = latitude_rad - first.latitude_rad;
        let mut lon = longitude_rad - first.longitude_rad;
        for _ in 0..INVERSE_ITERATIONS {
            let s = self.shift_at(lat, lon)?;
            let d_lat = lat + s.latitude_rad - latitude_rad;
            let d_lon = lon + s.longitude_rad - longitude_rad;
            lat -= d_lat;
            lon -= d_lon;
            if d_lat * d_lat + d_lon * d_lon <= INVERSE_TOLERANCE_RAD * INVERSE_TOLERANCE_RAD {
                return Ok((lat, lon, s));
            }
        }
        Err(DatumError::GridInverseDiverged {
            grid: self.grid.name().to_owned(),
            latitude_deg: latitude_rad.to_degrees(),
            longitude_deg: longitude_rad.to_degrees(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::tests::{ARC_SECOND, TestSubgrid, ntv2_bytes};
    use std::sync::Arc;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// Shifts of about a metre that vary across the grid, like GDA94 to GDA2020's.
    fn shifts(lat: f64, lon: f64) -> [f32; 4] {
        [
            (0.045 + 0.001 * (lat + 35.0)) as f32,
            // NTv2 longitude shifts are positive west.
            -(0.03 + 0.002 * (lon - 145.0)) as f32,
            0.0003,
            if lat > -34.6 { -1.0 } else { 0.0004 },
        ]
    }

    fn grid() -> HorizontalShiftGrid {
        let bytes = ntv2_bytes(
            &[TestSubgrid {
                name: "TEST",
                parent: "NONE",
                s_lat: -36.0 * 3600.0,
                n_lat: -34.0 * 3600.0,
                e_long: -146.0 * 3600.0,
                w_long: -144.0 * 3600.0,
                lat_inc: 60.0,
                long_inc: 60.0,
                values: &shifts,
            }],
            true,
        );
        HorizontalShiftGrid::new(Grid::open("test.gsb", Arc::new(bytes)).unwrap()).unwrap()
    }

    #[test]
    fn shifts_east_positive_in_radians() {
        let g = grid();
        let (lat, lon) = ((-35.0_f64).to_radians(), 145.0_f64.to_radians());
        // On a node, so exactly the node's values, which the file holds as f32.
        let s = g.shift_at(lat, lon).unwrap();
        let close = |a: f64, b: f32| (a - f64::from(b) * ARC_SECOND).abs() < 1e-12 * ARC_SECOND;
        assert!(close(s.latitude_rad, 0.045));
        assert!(close(s.longitude_rad, 0.03));
        assert!(close(s.latitude_sigma_rad.unwrap(), 0.0003));
        let (lat2, lon2, _) = g.forward(lat, lon).unwrap();
        assert_eq!(lat2, lat + s.latitude_rad);
        assert_eq!(lon2, lon + s.longitude_rad);
    }

    #[test]
    fn unknown_accuracy_is_none() {
        let g = grid();
        let s = g
            .shift_at((-34.5_f64).to_radians(), 145.0_f64.to_radians())
            .unwrap();
        assert_eq!((s.latitude_sigma_rad, s.longitude_sigma_rad), (None, None));
    }

    #[test]
    fn inverse_undoes_forward() {
        let g = grid();
        let mut rng = crate::testing::SplitMix64::new(7);
        for _ in 0..500 {
            let lat = rng.uniform(-35.99, -34.01).to_radians();
            let lon = rng.uniform(144.01, 145.99).to_radians();
            let (lat2, lon2, _) = g.forward(lat, lon).unwrap();
            let (lat3, lon3, _) = g.inverse(lat2, lon2).unwrap();
            // Within a few units in the last place (1e-16 rad is 0.6 nm).
            assert!((lat3 - lat).abs() < 5e-16 && (lon3 - lon).abs() < 5e-16);
        }
    }

    #[test]
    fn points_off_the_grid_are_refused() {
        let g = grid();
        assert!(matches!(
            g.forward((-33.0_f64).to_radians(), 145.0_f64.to_radians()),
            Err(DatumError::OutsideGrid { .. })
        ));
        assert!(!g.covers((-33.0_f64).to_radians(), 145.0_f64.to_radians()));
    }
}
