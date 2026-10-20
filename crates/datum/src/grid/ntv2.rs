//! NTv2 grid files (`.gsb`), the format of ICSM's GDA94 to GDA2020 grids and
//! Geoscience Australia's AUSGeoid models.
//!
//! The layout follows Natural Resources Canada's NTv2 developer's guide. An
//! overview header of 11 records, each an 8-byte name and an 8-byte value, is
//! followed for each subgrid by a header of 11 records and then the subgrid's
//! nodes, 16 bytes each: four 32-bit floats. Nodes start at the subgrid's
//! south-east corner and run west along each row, then north, because NTv2
//! longitudes are positive west. A file can be in either byte order;
//! `NUM_OREC` = 11 says which.
//!
//! The four fields are a latitude shift, a longitude shift (positive west) and
//! their accuracies, in the units `GS_TYPE` names. Geoscience Australia's
//! AUSGeoid files reuse the layout for other quantities: see [`crate::geoid`].

use std::f64::consts::PI;
use std::sync::Arc;

use super::{BandMetadata, GridMetadata, GridSource, MAX_BANDS, Subgrid, invalid, read_error};
use crate::error::DatumError;

/// Bytes in a header record.
const RECORD: usize = 16;
/// Records in the overview header and in each subgrid header.
const HEADER_RECORDS: usize = 11;
/// Bytes in a header.
const HEADER: usize = RECORD * HEADER_RECORDS;
/// Bytes in a node: four 32-bit floats.
const NODE: u64 = 16;
/// The most subgrids accepted, so a corrupt count can't ask for absurd work.
const MAX_SUBGRIDS: usize = 100_000;
/// How far a subgrid's extent can be from a whole number of node spacings,
/// as a fraction of the spacing, before the header is rejected. Published
/// grids are off by up to 1e-10 through rounding of their limits.
const SPACING_TOLERANCE: f64 = 1e-3;

/// Where a subgrid's nodes are in the file.
struct Layout {
    /// Offset of the first node.
    offset: u64,
    /// Nodes in each row.
    columns: usize,
}

/// Reads nodes from an NTv2 file.
pub(super) struct Reader {
    source: Arc<dyn GridSource>,
    little_endian: bool,
    layouts: Vec<Layout>,
}

/// A header record's name, without padding, and its value bytes.
fn record(header: &[u8], index: usize) -> (String, [u8; 8]) {
    let r = &header[index * RECORD..(index + 1) * RECORD];
    let mut value = [0; 8];
    value.copy_from_slice(&r[8..]);
    (text(&r[..8]), value)
}

/// Text in a header, up to any NUL, without padding. Some files leave bytes
/// after a NUL (AUSGeoid2020's `SYSTEM_T` is `AHD71\071`).
fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_owned()
}

/// Number decoding in the file's byte order.
#[derive(Clone, Copy)]
struct Endian {
    little: bool,
}

impl Endian {
    fn int(self, v: [u8; 8]) -> i32 {
        let b = [v[0], v[1], v[2], v[3]];
        if self.little {
            i32::from_le_bytes(b)
        } else {
            i32::from_be_bytes(b)
        }
    }

    fn double(self, v: [u8; 8]) -> f64 {
        if self.little {
            f64::from_le_bytes(v)
        } else {
            f64::from_be_bytes(v)
        }
    }

    fn float(self, b: [u8; 4]) -> f32 {
        if self.little {
            f32::from_le_bytes(b)
        } else {
            f32::from_be_bytes(b)
        }
    }
}

/// The number of nodes from `from` to `to` at spacing `step`, both ends included.
fn node_count(from: f64, to: f64, step: f64) -> Option<usize> {
    let intervals = (to - from) / step;
    let whole = intervals.round();
    let ok = intervals.is_finite()
        && (1.0..1e7).contains(&whole)
        && (intervals - whole).abs() <= SPACING_TOLERANCE;
    ok.then(|| whole as usize + 1)
}

/// Reads an NTv2 file's headers.
pub(super) fn open(
    name: &str,
    source: Arc<dyn GridSource>,
) -> Result<(Vec<Subgrid>, GridMetadata, Reader), DatumError> {
    let size = source.size();
    let read = |offset: u64, len: usize| -> Result<Vec<u8>, DatumError> {
        if offset.checked_add(len as u64).is_none_or(|end| end > size) {
            return Err(invalid(
                name,
                format!("the file is {size} bytes, so it ends inside a header at byte {offset}"),
            ));
        }
        let mut buf = vec![0; len];
        source
            .read_exact_at(offset, &mut buf)
            .map_err(|e| read_error(name, &e))?;
        Ok(buf)
    };
    let expect = |header: &[u8], index: usize, key: &str| -> Result<[u8; 8], DatumError> {
        let (k, v) = record(header, index);
        if k == key {
            Ok(v)
        } else {
            Err(invalid(
                name,
                format!("expected header record {key}, found {k:?}"),
            ))
        }
    };

    let overview = read(0, HEADER)?;
    let (_, orec) = record(&overview, 0);
    let first4 = [orec[0], orec[1], orec[2], orec[3]];
    let e = match (i32::from_le_bytes(first4), i32::from_be_bytes(first4)) {
        (11, _) => Endian { little: true },
        (_, 11) => Endian { little: false },
        _ => {
            return Err(invalid(
                name,
                "NUM_OREC isn't 11 in either byte order, so this isn't an NTv2 file",
            ));
        }
    };
    let count = e.int(expect(&overview, 2, "NUM_FILE")?);
    let count = usize::try_from(count)
        .ok()
        .filter(|n| (1..=MAX_SUBGRIDS).contains(n))
        .ok_or_else(|| {
            invalid(
                name,
                format!("NUM_FILE is {count}, but must be from 1 to {MAX_SUBGRIDS}"),
            )
        })?;
    let gs_type = text(&expect(&overview, 3, "GS_TYPE")?);
    let unit_rad = match gs_type.to_ascii_uppercase().as_str() {
        "SECONDS" => PI / 648_000.0,
        "MINUTES" => PI / 10_800.0,
        "DEGREES" => PI / 180.0,
        other => {
            return Err(invalid(
                name,
                format!("GS_TYPE is {other:?}; NTv2 grids are in SECONDS, MINUTES or DEGREES"),
            ));
        }
    };

    let mut items = Vec::new();
    for i in 3..HEADER_RECORDS {
        let (k, v) = record(&overview, i);
        let value = if k.starts_with("MAJOR") || k.starts_with("MINOR") {
            e.double(v).to_string()
        } else {
            text(&v)
        };
        items.push((k, value));
    }

    let mut subgrids = Vec::with_capacity(count);
    let mut parents = Vec::with_capacity(count);
    let mut layouts = Vec::with_capacity(count);
    let mut offset = HEADER as u64;
    for _ in 0..count {
        let h = read(offset, HEADER)?;
        let sub_name = text(&expect(&h, 0, "SUB_NAME")?);
        let parent = text(&expect(&h, 1, "PARENT")?);
        let value = |i: usize, key: &str| expect(&h, i, key).map(|v| e.double(v));
        let s_lat = value(4, "S_LAT")?;
        let n_lat = value(5, "N_LAT")?;
        let e_long = value(6, "E_LONG")?;
        let w_long = value(7, "W_LONG")?;
        let lat_inc = value(8, "LAT_INC")?;
        let long_inc = value(9, "LONG_INC")?;
        let gs_count = e.int(expect(&h, 10, "GS_COUNT")?);

        let bad_limits = || {
            invalid(
                name,
                format!(
                    "subgrid {sub_name}'s limits and spacing don't make a grid: latitude \
                     {s_lat} to {n_lat} by {lat_inc}, longitude (positive west) {e_long} to \
                     {w_long} by {long_inc}"
                ),
            )
        };
        let rows = node_count(s_lat, n_lat, lat_inc).ok_or_else(bad_limits)?;
        let columns = node_count(e_long, w_long, long_inc).ok_or_else(bad_limits)?;
        let nodes = rows.checked_mul(columns).ok_or_else(bad_limits)?;
        if usize::try_from(gs_count) != Ok(nodes) {
            return Err(invalid(
                name,
                format!(
                    "subgrid {sub_name} has GS_COUNT {gs_count}, but its limits and spacing give \
                     {rows} rows of {columns} nodes"
                ),
            ));
        }
        let first_node = offset + HEADER as u64;
        let end = (nodes as u64)
            .checked_mul(NODE)
            .and_then(|n| n.checked_add(first_node))
            .filter(|end| *end <= size)
            .ok_or_else(|| {
                invalid(
                    name,
                    format!("the file is {size} bytes, too short for subgrid {sub_name}'s nodes"),
                )
            })?;

        subgrids.push(Subgrid {
            name: sub_name,
            parent: None,
            south_rad: s_lat * unit_rad,
            west_rad: -w_long * unit_rad,
            latitude_step_rad: lat_inc * unit_rad,
            longitude_step_rad: long_inc * unit_rad,
            rows,
            columns,
        });
        parents.push(parent);
        layouts.push(Layout {
            offset: first_node,
            columns,
        });
        offset = end;
    }

    for (i, parent) in parents.iter().enumerate() {
        if parent.is_empty() || parent.eq_ignore_ascii_case("NONE") {
            continue;
        }
        let index = subgrids
            .iter()
            .position(|s| &s.name == parent)
            .ok_or_else(|| {
                invalid(
                    name,
                    format!(
                        "subgrid {} names its parent as {parent:?}, which isn't in the file",
                        subgrids[i].name
                    ),
                )
            })?;
        subgrids[i].parent = Some(index);
    }

    let metadata = GridMetadata {
        items,
        bands: vec![BandMetadata::default(); 4],
    };
    let reader = Reader {
        source,
        little_endian: e.little,
        layouts,
    };
    Ok((subgrids, metadata, reader))
}

impl Reader {
    /// Reads the four nodes of a cell: south-west, south-east, north-west and
    /// north-east, each with its four fields.
    pub(super) fn read_cell(
        &self,
        grid: &str,
        subgrid: usize,
        row: usize,
        column: usize,
        nodes: &mut [[f64; MAX_BANDS]; 4],
    ) -> Result<(), DatumError> {
        let layout = &self.layouts[subgrid];
        // Rows run east to west in the file, so a cell's eastern node comes first.
        let file_column = layout.columns - 2 - column;
        for (r, west) in [(row, 0), (row + 1, 2)] {
            let index = (r * layout.columns + file_column) as u64;
            let mut buf = [0_u8; 2 * NODE as usize];
            self.source
                .read_exact_at(layout.offset + index * NODE, &mut buf)
                .map_err(|e| read_error(grid, &e))?;
            nodes[west] = self.node(&buf[16..]);
            nodes[west + 1] = self.node(&buf[..16]);
        }
        Ok(())
    }

    fn node(&self, bytes: &[u8]) -> [f64; MAX_BANDS] {
        let e = Endian {
            little: self.little_endian,
        };
        let mut out = [f64::NAN; MAX_BANDS];
        for (value, b) in out.iter_mut().zip(bytes.chunks_exact(4)) {
            *value = f64::from(e.float([b[0], b[1], b[2], b[3]]));
        }
        out
    }
}
