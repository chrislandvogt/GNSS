//! Grids of values at regular spacing in latitude and longitude, read from
//! NTv2 files or from GeoTIFF files in PROJ's conventions (D-011).
//!
//! The core does no I/O (CONVENTIONS.md section 9). A host gives a grid its
//! bytes through [`GridSource`], and the grid reads only what it needs: its
//! headers when it's opened, then the nodes around each point it's asked
//! about. A 250 MB geoid model never has to fit in memory.
//!
//! A grid has one or more subgrids, which can be nested, and each node has a
//! value per band. [`Grid::cell`] finds the four nodes around a point in the
//! densest subgrid that covers it, and [`Cell::bilinear`] interpolates between
//! them, as the NTv2 method, EPSG's AUSGeoid method and PROJ all do.
//!
//! What the bands mean depends on the grid: see [`crate::gridshift`] for
//! horizontal shift grids and [`crate::geoid`] for geoid models.

use std::fmt;
use std::io;
use std::sync::Arc;

use crate::error::DatumError;

mod geotiff;
mod ntv2;

/// The most bands a grid can have. NTv2 files have four, and PROJ's grids use up to four.
pub const MAX_BANDS: usize = 8;

/// How far outside a subgrid's edge a point can be, as a fraction of the node
/// spacing, and still use the edge nodes. A point on the edge can land a hair
/// outside through rounding of the grid's stated limits. The value is PROJ's
/// (`REL_TOLERANCE_HGRIDSHIFT` in `grids.cpp`).
const EDGE_TOLERANCE: f64 = 1e-5;

/// The deepest nesting of subgrids accepted. Real grids have two or three levels.
const MAX_DEPTH: usize = 32;

/// The bytes of a grid file, readable at any offset. The host provides this:
/// a file on disk natively, a `File` or OPFS handle in the browser, or bytes
/// in memory.
pub trait GridSource: Send + Sync {
    /// The size of the file, in bytes.
    fn size(&self) -> u64;

    /// Fills `buf` with the bytes starting at `offset`, or fails if there
    /// aren't that many.
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
}

fn read_slice(bytes: &[u8], offset: u64, buf: &mut [u8]) -> io::Result<()> {
    let range = usize::try_from(offset)
        .ok()
        .and_then(|start| Some(start..start.checked_add(buf.len())?));
    match range.and_then(|r| bytes.get(r)) {
        Some(data) => {
            buf.copy_from_slice(data);
            Ok(())
        }
        None => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "asked for {} bytes at offset {offset}, but there are only {} bytes",
                buf.len(),
                bytes.len()
            ),
        )),
    }
}

impl GridSource for Vec<u8> {
    fn size(&self) -> u64 {
        self.len() as u64
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        read_slice(self, offset, buf)
    }
}

impl GridSource for &'static [u8] {
    fn size(&self) -> u64 {
        self.len() as u64
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        read_slice(self, offset, buf)
    }
}

/// A grid file's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridFormat {
    /// NTv2 (`.gsb`), the format ICSM and Geoscience Australia publish grids in.
    Ntv2,
    /// GeoTIFF in PROJ's grid conventions (`.tif`), as on PROJ's CDN.
    GeoTiff,
}

impl fmt::Display for GridFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            GridFormat::Ntv2 => "NTv2",
            GridFormat::GeoTiff => "GeoTIFF",
        })
    }
}

/// One rectangular block of nodes. Rows run south to north and columns west
/// to east, whatever order the file stores them in.
#[derive(Debug, Clone, PartialEq)]
pub struct Subgrid {
    /// Its name in the file, or a made-up one if the file doesn't name it.
    pub name: String,
    /// The subgrid it refines, as an index into [`Grid::subgrids`], or `None`
    /// at the top level.
    pub parent: Option<usize>,
    /// Latitude of the southern row, in radians.
    pub south_rad: f64,
    /// Longitude of the western column, in radians, positive east.
    pub west_rad: f64,
    /// Spacing between rows, in radians.
    pub latitude_step_rad: f64,
    /// Spacing between columns, in radians.
    pub longitude_step_rad: f64,
    /// Number of rows.
    pub rows: usize,
    /// Number of columns.
    pub columns: usize,
}

impl Subgrid {
    /// Latitude of the northern row, in radians.
    pub fn north_rad(&self) -> f64 {
        self.south_rad + self.rows.saturating_sub(1) as f64 * self.latitude_step_rad
    }

    /// Longitude of the eastern column, in radians, positive east.
    pub fn east_rad(&self) -> f64 {
        self.west_rad + self.columns.saturating_sub(1) as f64 * self.longitude_step_rad
    }

    /// The point's longitude, shifted by a turn if that puts it in this
    /// subgrid's longitude range.
    fn unwrap_longitude(&self, longitude_rad: f64) -> f64 {
        let tau = std::f64::consts::TAU;
        if longitude_rad < self.west_rad - self.longitude_step_rad * EDGE_TOLERANCE {
            longitude_rad + tau
        } else if longitude_rad > self.east_rad() + self.longitude_step_rad * EDGE_TOLERANCE {
            longitude_rad - tau
        } else {
            longitude_rad
        }
    }

    /// Whether a point is inside this subgrid, within [`EDGE_TOLERANCE`].
    fn contains(&self, latitude_rad: f64, longitude_rad: f64) -> bool {
        let lon = self.unwrap_longitude(longitude_rad);
        let lat_tolerance = self.latitude_step_rad * EDGE_TOLERANCE;
        let lon_tolerance = self.longitude_step_rad * EDGE_TOLERANCE;
        latitude_rad >= self.south_rad - lat_tolerance
            && latitude_rad <= self.north_rad() + lat_tolerance
            && lon >= self.west_rad - lon_tolerance
            && lon <= self.east_rad() + lon_tolerance
    }

    /// The cell containing a point inside the subgrid: the row and column of
    /// its south-west node, and the point's position across the cell.
    fn locate(&self, latitude_rad: f64, longitude_rad: f64) -> (usize, usize, f64, f64) {
        let lon = self.unwrap_longitude(longitude_rad);
        let (row, y) = split(
            (latitude_rad - self.south_rad) / self.latitude_step_rad,
            self.rows,
        );
        let (column, x) = split(
            (lon - self.west_rad) / self.longitude_step_rad,
            self.columns,
        );
        (row, column, x, y)
    }
}

/// Splits a position in node units into the index of the node before it and
/// the fraction of the way to the next, keeping both nodes inside `0..count`.
/// A point on the last node (or within the edge tolerance of it) uses the last
/// interval, at fraction 1.
fn split(position: f64, count: usize) -> (usize, f64) {
    let last = (count - 1) as f64;
    let p = position.clamp(0.0, last);
    let index = p.floor().min(last - 1.0);
    (index as usize, p - index)
}

/// What a grid file says about itself and its bands.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GridMetadata {
    /// File-level items in file order: NTv2 header records such as
    /// `SYSTEM_F`, or GDAL metadata items such as `TYPE`, with GeoTIFF's
    /// image description as `TIFFTAG_IMAGEDESCRIPTION`. A GeoTIFF's items
    /// come from its first image.
    pub items: Vec<(String, String)>,
    /// Each band's description, unit and other items, in band order.
    pub bands: Vec<BandMetadata>,
}

impl GridMetadata {
    /// The value of a file-level item.
    pub fn get(&self, key: &str) -> Option<&str> {
        lookup(&self.items, key)
    }
}

/// What a grid file says about one band.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BandMetadata {
    /// What the band holds, for example `latitude_offset` or `geoid_undulation`.
    pub description: Option<String>,
    /// Its unit, for example `arc-second` or `metre`.
    pub unit: Option<String>,
    /// Any other items, such as PROJ's `positive_value`.
    pub items: Vec<(String, String)>,
}

impl BandMetadata {
    /// The value of an item.
    pub fn get(&self, key: &str) -> Option<&str> {
        lookup(&self.items, key)
    }
}

fn lookup<'a>(items: &'a [(String, String)], key: &str) -> Option<&'a str> {
    items
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// The four nodes around a point, in one subgrid, and where the point lies
/// between them.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// The subgrid, as an index into [`Grid::subgrids`].
    pub subgrid: usize,
    /// Row of the southern nodes.
    pub row: usize,
    /// Column of the western nodes.
    pub column: usize,
    /// How far the point is from the western nodes to the eastern ones, 0 to 1.
    pub x: f64,
    /// How far the point is from the southern nodes to the northern ones, 0 to 1.
    pub y: f64,
    /// Values at the south-west, south-east, north-west and north-east nodes.
    nodes: [[f64; MAX_BANDS]; 4],
}

impl Cell {
    /// A band's values at the south-west, south-east, north-west and
    /// north-east nodes. NaN where the grid has no value, or for a band the
    /// grid doesn't have.
    pub fn nodes(&self, band: usize) -> [f64; 4] {
        self.nodes
            .map(|node| node.get(band).copied().unwrap_or(f64::NAN))
    }

    /// The bilinear weights of the south-west, south-east, north-west and
    /// north-east nodes, which sum to 1.
    pub fn weights(&self) -> [f64; 4] {
        let (x, y) = (self.x, self.y);
        [(1.0 - x) * (1.0 - y), x * (1.0 - y), (1.0 - x) * y, x * y]
    }

    /// A band's value at the point, interpolated bilinearly. NaN if any of the
    /// four nodes has no value.
    pub fn bilinear(&self, band: usize) -> f64 {
        let [sw, se, nw, ne] = self.nodes(band);
        let [w_sw, w_se, w_nw, w_ne] = self.weights();
        // In PROJ's order (pj_hgrid_interpolate in grids.cpp).
        w_sw * sw + w_se * se + w_nw * nw + w_ne * ne
    }
}

/// How a grid's nodes are read from its file.
enum Reader {
    Ntv2(ntv2::Reader),
    GeoTiff(geotiff::Reader),
}

/// A grid file, read through a [`GridSource`].
pub struct Grid {
    name: String,
    format: GridFormat,
    subgrids: Vec<Subgrid>,
    children: Vec<Vec<usize>>,
    bands: usize,
    metadata: GridMetadata,
    reader: Reader,
}

impl fmt::Debug for Grid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Grid")
            .field("name", &self.name)
            .field("format", &self.format)
            .field("subgrids", &self.subgrids.len())
            .field("bands", &self.bands)
            .finish_non_exhaustive()
    }
}

impl Grid {
    /// Opens a grid, recognising NTv2 or GeoTIFF from its first bytes. Only
    /// the headers are read. `name` identifies the grid in errors and
    /// provenance, usually its file name.
    pub fn open(name: impl Into<String>, source: Arc<dyn GridSource>) -> Result<Grid, DatumError> {
        let name = name.into();
        let mut magic = [0_u8; 8];
        if source.size() < magic.len() as u64 {
            return Err(invalid(&name, "the file is too short to be a grid"));
        }
        source
            .read_exact_at(0, &mut magic)
            .map_err(|e| read_error(&name, &e))?;
        if &magic == b"NUM_OREC" {
            let (subgrids, metadata, reader) = ntv2::open(&name, source)?;
            Grid::new(
                name,
                GridFormat::Ntv2,
                subgrids,
                4,
                metadata,
                Reader::Ntv2(reader),
            )
        } else if matches!(&magic[..4], b"II*\0" | b"MM\0*" | b"II+\0" | b"MM\0+") {
            let (subgrids, bands, metadata, reader) = geotiff::open(&name, source)?;
            Grid::new(
                name,
                GridFormat::GeoTiff,
                subgrids,
                bands,
                metadata,
                Reader::GeoTiff(reader),
            )
        } else {
            Err(invalid(
                &name,
                "isn't an NTv2 or GeoTIFF grid: it doesn't start with NUM_OREC or a TIFF header",
            ))
        }
    }

    fn new(
        name: String,
        format: GridFormat,
        subgrids: Vec<Subgrid>,
        bands: usize,
        metadata: GridMetadata,
        reader: Reader,
    ) -> Result<Grid, DatumError> {
        if subgrids.is_empty() {
            return Err(invalid(&name, "has no subgrids"));
        }
        if bands == 0 || bands > MAX_BANDS {
            return Err(invalid(
                &name,
                format!("has {bands} bands; grids with 1 to {MAX_BANDS} are supported"),
            ));
        }
        for s in &subgrids {
            let finite = [
                s.south_rad,
                s.west_rad,
                s.latitude_step_rad,
                s.longitude_step_rad,
            ]
            .iter()
            .all(|v| v.is_finite());
            if !finite || s.latitude_step_rad <= 0.0 || s.longitude_step_rad <= 0.0 {
                return Err(invalid(
                    &name,
                    format!("subgrid {} has invalid limits or spacing", s.name),
                ));
            }
            if s.rows < 2 || s.columns < 2 {
                return Err(invalid(
                    &name,
                    format!(
                        "subgrid {} has {} rows and {} columns; at least 2 of each are needed \
                         to interpolate",
                        s.name, s.rows, s.columns
                    ),
                ));
            }
        }
        let mut children = vec![Vec::new(); subgrids.len()];
        for (i, s) in subgrids.iter().enumerate() {
            if let Some(p) = s.parent {
                // Following parents must reach the top level without a cycle.
                let mut depth = 0;
                let mut at = Some(p);
                while let Some(a) = at {
                    depth += 1;
                    if a == i || depth > MAX_DEPTH || a >= subgrids.len() {
                        return Err(invalid(
                            &name,
                            format!("subgrid {} has an invalid chain of parents", s.name),
                        ));
                    }
                    at = subgrids[a].parent;
                }
                children[p].push(i);
            }
        }
        Ok(Grid {
            name,
            format,
            subgrids,
            children,
            bands,
            metadata,
            reader,
        })
    }

    /// The grid's name, as given when it was opened.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The file format.
    pub fn format(&self) -> GridFormat {
        self.format
    }

    /// The subgrids, in file order.
    pub fn subgrids(&self) -> &[Subgrid] {
        &self.subgrids
    }

    /// The number of values at each node.
    pub fn bands(&self) -> usize {
        self.bands
    }

    /// What the file says about itself and its bands.
    pub fn metadata(&self) -> &GridMetadata {
        &self.metadata
    }

    /// The densest subgrid containing a point: the first top-level subgrid that
    /// contains it, then the first of its children that does, and so on, as
    /// NTv2 and PROJ choose. `None` if the grid doesn't cover the point.
    pub fn subgrid_at(&self, latitude_rad: f64, longitude_rad: f64) -> Option<usize> {
        let inside = |i: &usize| self.subgrids[*i].contains(latitude_rad, longitude_rad);
        let mut found = (0..self.subgrids.len())
            .filter(|i| self.subgrids[*i].parent.is_none())
            .find(inside)?;
        while let Some(child) = self.children[found].iter().copied().find(inside) {
            found = child;
        }
        Some(found)
    }

    /// Whether the grid covers a point.
    pub fn covers(&self, latitude_rad: f64, longitude_rad: f64) -> bool {
        self.subgrid_at(latitude_rad, longitude_rad).is_some()
    }

    /// The four nodes around a point, read from the densest subgrid covering it.
    pub fn cell(&self, latitude_rad: f64, longitude_rad: f64) -> Result<Cell, DatumError> {
        let subgrid = self
            .subgrid_at(latitude_rad, longitude_rad)
            .ok_or_else(|| self.outside(latitude_rad, longitude_rad))?;
        let s = &self.subgrids[subgrid];
        let (row, column, x, y) = s.locate(latitude_rad, longitude_rad);
        let mut nodes = [[f64::NAN; MAX_BANDS]; 4];
        match &self.reader {
            Reader::Ntv2(r) => r.read_cell(&self.name, subgrid, row, column, &mut nodes)?,
            Reader::GeoTiff(r) => r.read_cell(&self.name, subgrid, row, column, &mut nodes)?,
        }
        Ok(Cell {
            subgrid,
            row,
            column,
            x,
            y,
            nodes,
        })
    }

    /// The error for a point the grid doesn't cover.
    pub(crate) fn outside(&self, latitude_rad: f64, longitude_rad: f64) -> DatumError {
        DatumError::OutsideGrid {
            grid: self.name.clone(),
            latitude_deg: latitude_rad.to_degrees(),
            longitude_deg: longitude_rad.to_degrees(),
        }
    }

    /// The error for a point where the grid has no value.
    pub(crate) fn no_value(&self, latitude_rad: f64, longitude_rad: f64) -> DatumError {
        DatumError::NoGridValue {
            grid: self.name.clone(),
            latitude_deg: latitude_rad.to_degrees(),
            longitude_deg: longitude_rad.to_degrees(),
        }
    }
}

/// An error for a grid file that isn't what it should be.
pub(crate) fn invalid(grid: &str, reason: impl Into<String>) -> DatumError {
    DatumError::InvalidGrid {
        grid: grid.to_owned(),
        reason: reason.into(),
    }
}

/// An error for bytes the host couldn't supply.
fn read_error(grid: &str, error: &io::Error) -> DatumError {
    DatumError::GridRead {
        grid: grid.to_owned(),
        reason: error.to_string(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// Arc-seconds to radians.
    pub(crate) const ARC_SECOND: f64 = std::f64::consts::PI / 648_000.0;

    /// A subgrid for building NTv2 test files: limits in arc-seconds, NTv2
    /// style (longitudes positive west), and a function giving each node's
    /// four values from its latitude and east-positive longitude in degrees.
    pub(crate) struct TestSubgrid<'a> {
        pub name: &'a str,
        pub parent: &'a str,
        pub s_lat: f64,
        pub n_lat: f64,
        pub e_long: f64,
        pub w_long: f64,
        pub lat_inc: f64,
        pub long_inc: f64,
        pub values: &'a dyn Fn(f64, f64) -> [f32; 4],
    }

    fn record(out: &mut Vec<u8>, key: &str, value: [u8; 8]) {
        let mut k = [b' '; 8];
        k[..key.len()].copy_from_slice(key.as_bytes());
        out.extend_from_slice(&k);
        out.extend_from_slice(&value);
    }

    fn text(s: &str) -> [u8; 8] {
        let mut v = [b' '; 8];
        v[..s.len()].copy_from_slice(s.as_bytes());
        v
    }

    /// An NTv2 file with the given subgrids, in either byte order.
    pub(crate) fn ntv2_bytes(subgrids: &[TestSubgrid<'_>], little_endian: bool) -> Vec<u8> {
        let int = |v: i32| {
            let mut b = [0_u8; 8];
            b[..4].copy_from_slice(&if little_endian {
                v.to_le_bytes()
            } else {
                v.to_be_bytes()
            });
            b
        };
        let double = |v: f64| {
            if little_endian {
                v.to_le_bytes()
            } else {
                v.to_be_bytes()
            }
        };
        let mut out = Vec::new();
        record(&mut out, "NUM_OREC", int(11));
        record(&mut out, "NUM_SREC", int(11));
        record(&mut out, "NUM_FILE", int(subgrids.len() as i32));
        record(&mut out, "GS_TYPE", text("SECONDS"));
        record(&mut out, "VERSION", text("NTv2.0"));
        record(&mut out, "SYSTEM_F", text("GDA94"));
        record(&mut out, "SYSTEM_T", text("GDA2020"));
        record(&mut out, "MAJOR_F", double(6_378_137.0));
        record(&mut out, "MINOR_F", double(6_356_752.314));
        record(&mut out, "MAJOR_T", double(6_378_137.0));
        record(&mut out, "MINOR_T", double(6_356_752.314));
        for s in subgrids {
            let rows = ((s.n_lat - s.s_lat) / s.lat_inc).round() as i32 + 1;
            let cols = ((s.w_long - s.e_long) / s.long_inc).round() as i32 + 1;
            record(&mut out, "SUB_NAME", text(s.name));
            record(&mut out, "PARENT", text(s.parent));
            record(&mut out, "CREATED", text("20261019"));
            record(&mut out, "UPDATED", text("20261019"));
            record(&mut out, "S_LAT", double(s.s_lat));
            record(&mut out, "N_LAT", double(s.n_lat));
            record(&mut out, "E_LONG", double(s.e_long));
            record(&mut out, "W_LONG", double(s.w_long));
            record(&mut out, "LAT_INC", double(s.lat_inc));
            record(&mut out, "LONG_INC", double(s.long_inc));
            record(&mut out, "GS_COUNT", int(rows * cols));
            for r in 0..rows {
                for c in 0..cols {
                    let lat = (s.s_lat + f64::from(r) * s.lat_inc) / 3600.0;
                    let lon_west = (s.e_long + f64::from(c) * s.long_inc) / 3600.0;
                    for v in (s.values)(lat, -lon_west) {
                        out.extend_from_slice(&if little_endian {
                            v.to_le_bytes()
                        } else {
                            v.to_be_bytes()
                        });
                    }
                }
            }
        }
        record(&mut out, "END", [0; 8]);
        out
    }

    /// A smooth test field, exact in f32 at the nodes of the test grids.
    pub(crate) fn plane(lat: f64, lon: f64) -> [f32; 4] {
        [
            (lat * 4.0 + 200.0) as f32,
            (lon * 2.0 - 250.0) as f32,
            0.25,
            0.5,
        ]
    }

    fn open(bytes: Vec<u8>) -> Grid {
        Grid::open("test.gsb", Arc::new(bytes)).unwrap()
    }

    /// Two top-level subgrids, the first with a child.
    fn nested() -> Vec<u8> {
        ntv2_bytes(
            &[
                TestSubgrid {
                    name: "WEST",
                    parent: "NONE",
                    s_lat: -36.0 * 3600.0,
                    n_lat: -34.0 * 3600.0,
                    e_long: -146.0 * 3600.0,
                    w_long: -144.0 * 3600.0,
                    lat_inc: 1800.0,
                    long_inc: 1800.0,
                    values: &plane,
                },
                TestSubgrid {
                    name: "EAST",
                    parent: "NONE",
                    s_lat: -36.0 * 3600.0,
                    n_lat: -34.0 * 3600.0,
                    e_long: -148.0 * 3600.0,
                    w_long: -146.0 * 3600.0,
                    lat_inc: 3600.0,
                    long_inc: 3600.0,
                    values: &plane,
                },
                TestSubgrid {
                    name: "CHILD",
                    parent: "WEST",
                    s_lat: -35.5 * 3600.0,
                    n_lat: -35.0 * 3600.0,
                    e_long: -145.0 * 3600.0,
                    w_long: -144.5 * 3600.0,
                    lat_inc: 900.0,
                    long_inc: 900.0,
                    values: &|lat, lon| {
                        let [a, b, ..] = plane(lat, lon);
                        [a, b, 0.125, 0.0625]
                    },
                },
            ],
            true,
        )
    }

    #[test]
    fn reads_ntv2_headers() {
        let g = open(nested());
        assert_eq!(g.format(), GridFormat::Ntv2);
        assert_eq!(g.bands(), 4);
        assert_eq!(g.metadata().get("SYSTEM_F"), Some("GDA94"));
        assert_eq!(g.metadata().get("GS_TYPE"), Some("SECONDS"));
        let names: Vec<_> = g.subgrids().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["WEST", "EAST", "CHILD"]);
        let west = &g.subgrids()[0];
        assert_eq!((west.rows, west.columns), (5, 5));
        assert!((west.west_rad - 144.0_f64.to_radians()).abs() < 1e-15);
        assert!((west.east_rad() - 146.0_f64.to_radians()).abs() < 1e-15);
        assert!((west.north_rad() - (-34.0_f64).to_radians()).abs() < 1e-15);
        assert_eq!(g.subgrids()[2].parent, Some(0));
    }

    #[test]
    fn chooses_the_densest_subgrid() {
        let g = open(nested());
        let at = |lat: f64, lon: f64| g.subgrid_at(lat.to_radians(), lon.to_radians());
        assert_eq!(at(-35.2, 144.7), Some(2));
        assert_eq!(at(-35.7, 144.7), Some(0));
        assert_eq!(at(-35.7, 147.5), Some(1));
        // On the shared edge, the first subgrid in the file wins.
        assert_eq!(at(-35.7, 146.0), Some(0));
        assert_eq!(at(-33.9, 145.0), None);
        assert_eq!(at(-35.0, 143.9), None);
        assert!(matches!(
            g.cell((-33.9_f64).to_radians(), 145.0_f64.to_radians()),
            Err(DatumError::OutsideGrid { .. })
        ));
    }

    fn rad(deg: f64) -> f64 {
        deg.to_radians()
    }

    #[test]
    fn interpolates_a_plane_exactly() {
        let g = open(nested());
        for (lat, lon) in [
            (-35.2, 144.7),
            (-35.71, 145.33),
            (-34.0, 146.0),
            (-36.0, 144.0),
            (-35.5, 147.9),
        ] {
            // The nodes hold the plane exactly, so interpolation reproduces it.
            let c = g.cell(rad(lat), rad(lon)).unwrap();
            assert!(
                (c.bilinear(0) - (lat * 4.0 + 200.0)).abs() < 1e-9,
                "{lat} {lon}"
            );
            assert!((c.bilinear(1) - (lon * 2.0 - 250.0)).abs() < 1e-9);
        }
        // The child's own values come through.
        let c = g.cell(rad(-35.2), rad(144.7)).unwrap();
        assert_eq!(c.bilinear(2), 0.125);
        assert!(c.bilinear(MAX_BANDS + 1).is_nan());
    }

    #[test]
    fn corners_and_edges_use_the_last_interval() {
        let g = open(nested());
        let c = g.cell(rad(-34.0), rad(146.0)).unwrap();
        assert_eq!((c.subgrid, c.row, c.column), (0, 3, 3));
        // Rounding in radians leaves the corner a hair inside.
        assert!((c.x - 1.0).abs() < 1e-12 && (c.y - 1.0).abs() < 1e-12);
        let c = g.cell(rad(-36.0), rad(144.0)).unwrap();
        assert_eq!((c.row, c.column, c.x, c.y), (0, 0, 0.0, 0.0));
    }

    #[test]
    fn both_byte_orders_read_the_same() {
        let little = open(nested());
        let big = open(ntv2_bytes(
            &[TestSubgrid {
                name: "WEST",
                parent: "NONE",
                s_lat: -36.0 * 3600.0,
                n_lat: -34.0 * 3600.0,
                e_long: -146.0 * 3600.0,
                w_long: -144.0 * 3600.0,
                lat_inc: 1800.0,
                long_inc: 1800.0,
                values: &plane,
            }],
            false,
        ));
        let (lat, lon) = (rad(-35.71), rad(145.33));
        for band in 0..4 {
            assert_eq!(
                big.cell(lat, lon).unwrap().nodes(band),
                little.cell(lat, lon).unwrap().nodes(band)
            );
        }
    }

    #[test]
    fn rejects_files_that_arent_grids() {
        for bytes in [vec![], b"hello world, not a grid".to_vec()] {
            assert!(matches!(
                Grid::open("x", Arc::new(bytes)),
                Err(DatumError::InvalidGrid { .. })
            ));
        }
        // Truncated nodes.
        let mut bytes = nested();
        bytes.truncate(bytes.len() - 100);
        let err = Grid::open("cut.gsb", Arc::new(bytes)).unwrap_err();
        assert!(err.to_string().contains("cut.gsb"), "{err}");
    }

    #[test]
    fn split_keeps_nodes_inside() {
        assert_eq!(split(0.0, 5), (0, 0.0));
        assert_eq!(split(2.5, 5), (2, 0.5));
        assert_eq!(split(4.0, 5), (3, 1.0));
        assert_eq!(split(4.000_001, 5), (3, 1.0));
        assert_eq!(split(-0.000_001, 5), (0, 0.0));
    }
}
