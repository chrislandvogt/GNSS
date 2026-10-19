//! Geoid models: the separation N between an ellipsoid and a height datum, so
//! that a datum height is H = h − N for an ellipsoidal height h.
//!
//! AUSGeoid2020 relates GDA2020 ellipsoidal heights to the Australian Height
//! Datum (EPSG:8451 and 9466). It's a hybrid model, fitted to AHD, so it gives
//! AHD heights directly. Geoscience Australia publishes it as an NTv2 file
//! whose four fields are N, the two deflections of the vertical and an unused
//! field, with N = -999 offshore; PROJ's GeoTIFF version holds N only. Its
//! uncertainty is a separate NTv2 file, whose first field is the one-sigma
//! uncertainty of N (GA's GeodePy reads it as a standard deviation).
//!
//! N is interpolated bilinearly. Where some of the four nodes around a point
//! have no value, at the edge of the model's coverage offshore, the others are
//! used with their weights rescaled, as PROJ does (`read_vgrid_value` in
//! `grids.cpp`); only where none has a value is there no N. The uncertainty is
//! interpolated the same way.

use crate::error::DatumError;
use crate::frame::FrameId;
use crate::grid::{Grid, GridFormat, invalid};
use crate::height::{Height, HeightKind};
use crate::position::Position;

/// The value AUSGeoid's NTv2 files hold where the model has no value.
const NTV2_NO_VALUE: f64 = -999.0;

/// How far apart two grids' limits can be and still count as the same, in
/// radians (about 0.6 mm). The NTv2 and GeoTIFF versions of a model state
/// the same limits in different units.
const SAME_GRID_TOLERANCE_RAD: f64 = 1e-10;

/// The separation between the ellipsoid and the height datum at a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Separation {
    /// N: the height of the datum's surface above the ellipsoid, in metres.
    pub n_m: f64,
    /// One standard deviation of N, in metres, if the model has an
    /// uncertainty grid and it gives a value here.
    pub sigma_m: Option<f64>,
}

/// A geoid model: grids of N, and optionally of its uncertainty, relating a
/// frame's ellipsoidal heights to a height datum.
#[derive(Debug)]
pub struct GeoidModel {
    name: String,
    frame: FrameId,
    datum: HeightKind,
    undulation: Grid,
    band: usize,
    uncertainty: Option<Grid>,
}

impl GeoidModel {
    /// AUSGeoid2020: GDA2020 ellipsoidal heights to AHD.
    ///
    /// `grid` is Geoscience Australia's `AUSGeoid2020_20180201.gsb` or PROJ's
    /// `au_ga_AUSGeoid2020_20180201.tif`. `uncertainty`, if given, is
    /// `AUSGeoid2020_20180201_error.gsb`, and must cover the same area.
    pub fn ausgeoid2020(grid: Grid, uncertainty: Option<Grid>) -> Result<Self, DatumError> {
        let meta = grid.metadata();
        let band = match grid.format() {
            GridFormat::Ntv2 => {
                let from = meta.get("SYSTEM_F").unwrap_or("");
                let to = meta.get("SYSTEM_T").unwrap_or("");
                if !from.contains("GDA2020") || !to.starts_with("AHD") {
                    return Err(invalid(
                        grid.name(),
                        format!(
                            "isn't AUSGeoid2020: its header relates {from:?} to {to:?}, not \
                             GDA2020 to AHD"
                        ),
                    ));
                }
                0
            }
            GridFormat::GeoTiff => {
                let band = meta
                    .bands
                    .iter()
                    .position(|b| b.description.as_deref() == Some("geoid_undulation"));
                let vertical = meta.get("TYPE") == Some("VERTICAL_OFFSET_GEOGRAPHIC_TO_VERTICAL");
                let ahd = meta.get("target_crs_epsg_code") == Some("5711");
                match band {
                    Some(b) if vertical && ahd => b,
                    _ => {
                        return Err(invalid(
                            grid.name(),
                            "isn't AUSGeoid2020: it needs TYPE=VERTICAL_OFFSET_GEOGRAPHIC_TO_VERTICAL, \
                             a geoid_undulation band and target EPSG:5711 (AHD)",
                        ));
                    }
                }
            }
        };
        if let Some(u) = &uncertainty {
            let to = u.metadata().get("SYSTEM_T").unwrap_or("");
            if u.format() != GridFormat::Ntv2 || !to.starts_with("AHD") {
                return Err(invalid(
                    u.name(),
                    "isn't AUSGeoid2020's uncertainty grid (an NTv2 file to AHD)",
                ));
            }
            if !same_layout(&grid, u) {
                return Err(invalid(
                    u.name(),
                    format!("doesn't cover the same area as {}", grid.name()),
                ));
            }
        }
        Ok(GeoidModel {
            name: format!("AUSGeoid2020 ({})", grid.name()),
            frame: FrameId::Gda2020,
            datum: HeightKind::Ahd,
            undulation: grid,
            band,
            uncertainty,
        })
    }

    /// The model's name, with the grid file it was read from.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The frame whose ellipsoidal heights the model relates to the datum.
    pub fn frame(&self) -> FrameId {
        self.frame
    }

    /// The kind of height the model gives.
    pub fn datum(&self) -> HeightKind {
        self.datum
    }

    /// The grid of N.
    pub fn grid(&self) -> &Grid {
        &self.undulation
    }

    /// The grid of N's uncertainty, if the model has one.
    pub fn uncertainty_grid(&self) -> Option<&Grid> {
        self.uncertainty.as_ref()
    }

    /// N, and its uncertainty if known, at a point on the frame's ellipsoid.
    pub fn separation(
        &self,
        latitude_rad: f64,
        longitude_rad: f64,
    ) -> Result<Separation, DatumError> {
        let cell = self.undulation.cell(latitude_rad, longitude_rad)?;
        let n_m = interpolate(&cell, self.band, |v| v != NTV2_NO_VALUE)
            .ok_or_else(|| self.undulation.no_value(latitude_rad, longitude_rad))?;
        let sigma_m = match &self.uncertainty {
            Some(u) => {
                // Offshore the file holds 0, which isn't an uncertainty.
                interpolate(&u.cell(latitude_rad, longitude_rad)?, 0, |v| v > 0.0)
            }
            None => None,
        };
        Ok(Separation { n_m, sigma_m })
    }

    /// The datum height (for AUSGeoid2020, the AHD height) of a position, and
    /// the separation used. The position must be in the model's frame.
    ///
    /// The height's sigma combines the position's vertical uncertainty with
    /// N's, and is `None` unless both are known.
    pub fn datum_height(&self, position: &Position) -> Result<(Height, Separation), DatumError> {
        if position.frame() != self.frame {
            return Err(DatumError::WrongFrameForHeightModel {
                model: self.name.clone(),
                expected: self.frame,
                frame: position.frame(),
            });
        }
        let g = position.geodetic();
        let s = self.separation(g.latitude_rad(), g.longitude_rad())?;
        let h_sigma = position
            .enu_covariance()
            .map(|c| libm::sqrt(c[(2, 2)].max(0.0)));
        Ok((
            Height {
                value_m: g.ellipsoidal_height_m() - s.n_m,
                sigma_m: combine(h_sigma, s.sigma_m),
                kind: self.datum,
                model: Some(self.name.clone()),
            },
            s,
        ))
    }

    /// The ellipsoidal height, on the model's frame, of a datum height at a
    /// point, and the separation used.
    pub fn ellipsoidal_height(
        &self,
        latitude_rad: f64,
        longitude_rad: f64,
        height: &Height,
    ) -> Result<(Height, Separation), DatumError> {
        if height.kind != self.datum {
            return Err(DatumError::WrongHeightKind {
                operation: "converting a height to an ellipsoidal height",
                expected: self.datum.to_string(),
                given: height.kind.to_string(),
            });
        }
        let s = self.separation(latitude_rad, longitude_rad)?;
        Ok((
            Height {
                value_m: height.value_m + s.n_m,
                sigma_m: combine(height.sigma_m, s.sigma_m),
                kind: HeightKind::Ellipsoidal(self.frame),
                model: Some(self.name.clone()),
            },
            s,
        ))
    }
}

/// A band's value, interpolated bilinearly over the nodes that have a value,
/// with their weights rescaled to sum to 1, as PROJ does for vertical grids.
/// `None` if no node with any weight has a value.
fn interpolate(
    cell: &crate::grid::Cell,
    band: usize,
    has_value: impl Fn(f64) -> bool,
) -> Option<f64> {
    let nodes = cell.nodes(band);
    let weights = cell.weights();
    let usable = |v: f64| v.is_finite() && has_value(v);
    if nodes.iter().all(|v| usable(*v)) {
        return Some(cell.bilinear(band));
    }
    let (mut sum, mut total) = (0.0, 0.0);
    for (v, w) in nodes.into_iter().zip(weights) {
        if usable(v) {
            sum += v * w;
            total += w;
        }
    }
    (total > 0.0).then(|| sum / total)
}

/// Two independent one-sigma uncertainties combined, if both are known.
fn combine(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    Some(libm::sqrt(a? * a? + b? * b?))
}

/// Whether two grids have the same subgrids, to within a fraction of a millimetre.
fn same_layout(a: &Grid, b: &Grid) -> bool {
    a.subgrids().len() == b.subgrids().len()
        && a.subgrids().iter().zip(b.subgrids()).all(|(x, y)| {
            x.rows == y.rows
                && x.columns == y.columns
                && [
                    x.south_rad - y.south_rad,
                    x.west_rad - y.west_rad,
                    x.north_rad() - y.north_rad(),
                    x.east_rad() - y.east_rad(),
                ]
                .iter()
                .all(|d| d.abs() < SAME_GRID_TOLERANCE_RAD)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::tests::{TestSubgrid, ntv2_bytes};
    use crate::position::Covariance;
    use crate::{Geodetic, Position};
    use std::sync::Arc;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// A small AUSGeoid2020-like NTv2 file: N rising 1 m per degree east,
    /// and no value south of -35.9°.
    fn model_grid(header_to: &str, values: &dyn Fn(f64, f64) -> [f32; 4]) -> Grid {
        let mut bytes = ntv2_bytes(
            &[TestSubgrid {
                name: "AUSGEOID",
                parent: "NONE",
                s_lat: -36.0 * 3600.0,
                n_lat: -35.0 * 3600.0,
                e_long: -150.0 * 3600.0,
                w_long: -149.0 * 3600.0,
                lat_inc: 60.0,
                long_inc: 60.0,
                values,
            }],
            true,
        );
        // SYSTEM_F and SYSTEM_T.
        bytes[16 * 5 + 8..16 * 5 + 16].copy_from_slice(b"GDA2020 ");
        let mut to = [b' '; 8];
        to[..header_to.len()].copy_from_slice(header_to.as_bytes());
        bytes[16 * 6 + 8..16 * 6 + 16].copy_from_slice(&to);
        Grid::open(format!("{header_to}.gsb"), Arc::new(bytes)).unwrap()
    }

    fn n(lat: f64, lon: f64) -> [f32; 4] {
        let n = if lat < -35.9 {
            -999.0
        } else {
            20.0 + (lon - 149.0) as f32
        };
        [n, -4.0, -5.0, 0.0]
    }

    fn sigma(lat: f64, _lon: f64) -> [f32; 4] {
        [if lat < -35.9 { 0.0 } else { 0.08 }, 0.0, 0.0, 0.0]
    }

    fn model() -> GeoidModel {
        GeoidModel::ausgeoid2020(
            model_grid("AHD71", &n),
            Some(model_grid("AHD_1971", &sigma)),
        )
        .unwrap()
    }

    #[test]
    fn ahd_is_ellipsoidal_height_minus_n() {
        let m = model();
        let g = Geodetic::from_degrees(-35.28, 149.13, 600.0).unwrap();
        let cov = Covariance::from_enu(
            nalgebra::Matrix3::from_diagonal(&nalgebra::Vector3::new(1e-4, 1e-4, 9e-4)),
            &g,
        )
        .unwrap();
        let p = Position::from_geodetic(FrameId::Gda2020, None, &g, Some(cov)).unwrap();
        let (h, s) = m.datum_height(&p).unwrap();
        // The nodes hold N as f32, good to 1e-6 m here.
        assert!((s.n_m - 20.13).abs() < 2e-6, "{}", s.n_m);
        assert!((h.value_m - (600.0 - 20.13)).abs() < 2e-6);
        assert_eq!(h.kind, HeightKind::Ahd);
        // 3 cm from the position and 8 cm from the model.
        let expected = libm::sqrt(0.03 * 0.03 + 0.08 * 0.08);
        assert!((h.sigma_m.unwrap() - expected).abs() < 1e-6);
        assert_eq!(h.model.as_deref(), Some("AUSGeoid2020 (AHD71.gsb)"));

        // And back.
        let (e, _) = m
            .ellipsoidal_height(g.latitude_rad(), g.longitude_rad(), &h)
            .unwrap();
        assert!((e.value_m - 600.0).abs() < 1e-9);
        assert_eq!(e.kind, HeightKind::Ellipsoidal(FrameId::Gda2020));
    }

    #[test]
    fn only_gda2020_positions_are_accepted() {
        let g = Geodetic::from_degrees(-35.28, 149.13, 600.0).unwrap();
        let p = Position::from_geodetic(FrameId::Gda94, None, &g, None).unwrap();
        let err = model().datum_height(&p).unwrap_err();
        assert!(matches!(err, DatumError::WrongFrameForHeightModel { .. }));
        assert!(
            err.to_string().contains("transform it to GDA2020 first"),
            "{err}"
        );
    }

    #[test]
    fn unknown_sigma_stays_unknown() {
        let m = model();
        let g = Geodetic::from_degrees(-35.28, 149.13, 600.0).unwrap();
        let p = Position::from_geodetic(FrameId::Gda2020, None, &g, None).unwrap();
        let (h, s) = m.datum_height(&p).unwrap();
        assert_eq!(h.sigma_m, None);
        assert!((s.sigma_m.unwrap() - 0.08).abs() < 1e-7);
    }

    #[test]
    fn offshore_has_no_value() {
        let m = model();
        let err = m
            .separation((-35.95_f64).to_radians(), 149.5_f64.to_radians())
            .unwrap_err();
        assert!(matches!(err, DatumError::NoGridValue { .. }), "{err}");
    }

    #[test]
    fn at_the_edge_of_coverage_the_nodes_with_values_are_used() {
        // Rows of nodes are a minute apart: -35.9 is between the row at
        // -35.9167, which has no value, and the row at -35.9, which does.
        let m = model();
        let lat = (-35.9 - 0.25 / 60.0_f64).to_radians();
        let s = m.separation(lat, 149.5_f64.to_radians()).unwrap();
        // Only the northern nodes count: N there is 20.5.
        assert!((s.n_m - 20.5).abs() < 2e-6, "{}", s.n_m);
        assert!((s.sigma_m.unwrap() - 0.08).abs() < 1e-7);
    }

    #[test]
    fn wrong_grids_are_refused() {
        // A GDA94 to GDA2020 shift grid isn't a geoid model.
        let shift = model_grid("GDA2020", &n);
        assert!(GeoidModel::ausgeoid2020(shift, None).is_err());
        // Nor is the model itself an uncertainty grid for a different area.
        let other = GeoidModel::ausgeoid2020(
            model_grid("AHD71", &n),
            Some({
                let bytes = ntv2_bytes(
                    &[TestSubgrid {
                        name: "AUSGEOID",
                        parent: "NONE",
                        s_lat: -37.0 * 3600.0,
                        n_lat: -36.0 * 3600.0,
                        e_long: -150.0 * 3600.0,
                        w_long: -149.0 * 3600.0,
                        lat_inc: 60.0,
                        long_inc: 60.0,
                        values: &sigma,
                    }],
                    true,
                );
                let mut b = bytes;
                b[16 * 6 + 8..16 * 6 + 16].copy_from_slice(b"AHD_1971");
                Grid::open("elsewhere.gsb", Arc::new(b)).unwrap()
            }),
        );
        assert!(other.unwrap_err().to_string().contains("same area"));
        // A height of the wrong kind can't be converted.
        let g = Geodetic::from_degrees(-35.28, 149.13, 600.0).unwrap();
        let ellipsoidal = Height {
            value_m: 600.0,
            sigma_m: None,
            kind: HeightKind::Ellipsoidal(FrameId::Gda2020),
            model: None,
        };
        assert!(matches!(
            model().ellipsoidal_height(g.latitude_rad(), g.longitude_rad(), &ellipsoidal),
            Err(DatumError::WrongHeightKind { .. })
        ));
    }
}
