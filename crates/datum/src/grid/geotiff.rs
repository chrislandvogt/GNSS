//! GeoTIFF grids in PROJ's conventions, as published on PROJ's CDN (PROJ's
//! "Geodetic TIFF grids" specification).
//!
//! Each subgrid is an image in the file, and reduced-resolution images are
//! skipped. An image is georeferenced by a tie point and a pixel scale in
//! degrees on a geographic CRS, with nodes at pixel centres for
//! `PixelIsPoint` and half a pixel in from the tie point for `PixelIsArea`.
//! GDAL's metadata tag gives the grid's type, each band's description, unit,
//! offset and scale, and each subgrid's name and parent. Images can be tiled
//! or stripped, interleaved or band by band, and compressed; the `tiff` crate
//! decodes them, one tile or strip at a time.

use std::f64::consts::PI;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex, PoisonError};

use tiff::TiffError;
use tiff::decoder::{ChunkType, Decoder, DecodingResult, Limits};
use tiff::tags::Tag;

use super::{BandMetadata, GridMetadata, GridSource, MAX_BANDS, Subgrid, invalid};
use crate::error::DatumError;

/// GDAL's metadata tag, an XML string.
const GDAL_METADATA: u16 = 42112;
/// The TIFF image description tag.
const IMAGE_DESCRIPTION: u16 = 270;
/// GeoTIFF's model type key, 2 for a geographic CRS.
const MODEL_TYPE_KEY: u16 = 1024;
/// GeoTIFF's raster type key: 1 for `PixelIsArea`, 2 for `PixelIsPoint`.
const RASTER_TYPE_KEY: u16 = 1025;
/// GeoTIFF's angular units key, 9102 for degrees.
const ANGULAR_UNITS_KEY: u16 = 2054;
/// The most images (subgrids) accepted. PROJ's largest grids have about a
/// hundred; each image's decoder walks the file's chain of images to reach
/// it, so the limit also bounds the work of opening a file.
const MAX_IMAGES: usize = 1_000;
/// How many decoded tiles or strips are kept in memory.
const CACHE_CHUNKS: usize = 16;

/// Limits on what a file can make the decoder allocate. PROJ's grids use
/// 256 × 256 tiles of up to four 32-bit bands, well within these.
fn limits() -> Limits {
    let mut l = Limits::default();
    l.decoding_buffer_size = 64 << 20;
    l.ifd_value_size = 16 << 20;
    l.intermediate_buffer_size = 64 << 20;
    l
}

/// A grid source read as a stream, for the `tiff` crate.
struct SourceReader {
    source: Arc<dyn GridSource>,
    position: u64,
}

impl Read for SourceReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.source.size().saturating_sub(self.position);
        let n = usize::try_from(left).map_or(buf.len(), |left| left.min(buf.len()));
        self.source.read_exact_at(self.position, &mut buf[..n])?;
        self.position += n as u64;
        Ok(n)
    }
}

impl Seek for SourceReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.source.size().checked_add_signed(d),
            SeekFrom::Current(d) => self.position.checked_add_signed(d),
        };
        self.position = target.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before the start of the grid",
            )
        })?;
        Ok(self.position)
    }
}

type TiffDecoder = Decoder<BufReader<SourceReader>>;

fn decoder(name: &str, source: &Arc<dyn GridSource>) -> Result<TiffDecoder, DatumError> {
    let reader = BufReader::new(SourceReader {
        source: Arc::clone(source),
        position: 0,
    });
    Decoder::new(reader)
        .map(|d| d.with_limits(limits()))
        .map_err(|e| tiff_error(name, e))
}

fn tiff_error(grid: &str, error: TiffError) -> DatumError {
    match error {
        TiffError::IoError(e) => DatumError::GridRead {
            grid: grid.to_owned(),
            reason: e.to_string(),
        },
        other => invalid(grid, format!("isn't a readable TIFF: {other}")),
    }
}

/// One image (subgrid) and how to find its values.
struct Image {
    decoder: Mutex<TiffDecoder>,
    height: usize,
    chunk_width: usize,
    chunk_height: usize,
    chunks_across: usize,
    chunks_per_plane: usize,
    planar: bool,
    samples: usize,
    scale: [f64; MAX_BANDS],
    offset: [f64; MAX_BANDS],
    nodata: Option<f64>,
}

/// A decoded tile or strip.
struct Chunk {
    values: DecodingResult,
    /// Pixels in each row of `values`.
    width: usize,
}

/// Reads nodes from a GeoTIFF grid.
pub(super) struct Reader {
    images: Vec<Image>,
    /// Recently decoded chunks, most recent first, by image and chunk index.
    cache: Mutex<Vec<(usize, usize, Arc<Chunk>)>>,
}

/// An item of GDAL's metadata: its name, the band it belongs to (if any),
/// its role (if any) and its value.
#[derive(Debug, PartialEq)]
struct Item {
    name: String,
    sample: Option<usize>,
    role: Option<String>,
    value: String,
}

/// Parses GDAL's metadata XML: a list of `<Item name=".." sample=".." role="..">value</Item>`.
fn parse_gdal_metadata(xml: &str) -> Vec<Item> {
    let mut items = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<Item") {
        rest = &rest[start + 5..];
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..end];
        rest = &rest[end + 1..];
        let value = if tag.ends_with('/') {
            String::new()
        } else {
            let close = rest.find("</Item>").unwrap_or(rest.len());
            let v = unescape(&rest[..close]);
            rest = &rest[close..];
            v
        };
        let attribute = |key: &str| {
            let pattern = format!("{key}=\"");
            let at = tag.find(&pattern)? + pattern.len();
            let len = tag[at..].find('"')?;
            Some(unescape(&tag[at..at + len]))
        };
        if let Some(name) = attribute("name") {
            items.push(Item {
                name,
                sample: attribute("sample").and_then(|s| s.trim().parse().ok()),
                role: attribute("role"),
                value,
            });
        }
    }
    items
}

/// Replaces XML's predefined entities.
fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The value of a GeoTIFF key held directly in the key directory.
fn geokey(directory: &[u16], key: u16) -> Option<u16> {
    let count = usize::from(*directory.get(3)?);
    directory
        .get(4..)?
        .chunks_exact(4)
        .take(count)
        .find(|entry| entry[0] == key && entry[1] == 0)
        .map(|entry| entry[3])
}

/// Everything read from one image's tags.
struct ImageInfo {
    subgrid: Subgrid,
    parent_name: Option<String>,
    metadata: GridMetadata,
    image: Image,
}

fn read_image(
    name: &str,
    source: &Arc<dyn GridSource>,
    scan: &mut TiffDecoder,
    index: usize,
) -> Result<Option<ImageInfo>, DatumError> {
    let e = |error: TiffError| tiff_error(name, error);
    let reduced = scan
        .find_tag_unsigned::<u32>(Tag::NewSubfileType)
        .map_err(e)?
        .is_some_and(|t| t & 1 != 0);
    if reduced {
        return Ok(None);
    }
    let (width, height) = scan.dimensions().map_err(e)?;
    let (width, height) = (width as usize, height as usize);
    let samples = scan
        .find_tag_unsigned::<u16>(Tag::SamplesPerPixel)
        .map_err(e)?
        .unwrap_or(1) as usize;
    if samples == 0 || samples > MAX_BANDS {
        return Err(invalid(
            name,
            format!("image {index} has {samples} bands; 1 to {MAX_BANDS} are supported"),
        ));
    }
    let planar = scan
        .find_tag_unsigned::<u16>(Tag::PlanarConfiguration)
        .map_err(e)?
        == Some(2);

    let (chunk_width, chunk_height, chunks) = match scan.get_chunk_type() {
        ChunkType::Tile => {
            let (w, h) = scan.chunk_dimensions();
            (
                w as usize,
                h as usize,
                scan.tile_count().map_err(e)? as usize,
            )
        }
        ChunkType::Strip => {
            let (_, h) = scan.chunk_dimensions();
            (width, h as usize, scan.strip_count().map_err(e)? as usize)
        }
    };
    let chunks_across = width.div_ceil(chunk_width.max(1));
    let chunks_per_plane = chunks_across * height.div_ceil(chunk_height.max(1));
    let planes = if planar { samples } else { 1 };
    if chunk_width == 0 || chunk_height == 0 || chunks_per_plane * planes != chunks {
        return Err(invalid(
            name,
            format!("image {index} has {chunks} tiles or strips, which doesn't match its size"),
        ));
    }

    // Georeferencing.
    let doubles = |scan: &mut TiffDecoder, tag: Tag| -> Result<Option<Vec<f64>>, DatumError> {
        scan.find_tag(tag)
            .map_err(e)?
            .map(|v| v.into_f64_vec().map_err(e))
            .transpose()
    };
    let scale = doubles(scan, Tag::ModelPixelScaleTag)?;
    let tie = doubles(scan, Tag::ModelTiepointTag)?;
    let (Some(scale), Some(tie)) = (scale, tie) else {
        return Err(invalid(
            name,
            format!(
                "image {index} has no tie point and pixel scale; grids rotated with a \
                 transformation matrix aren't supported"
            ),
        ));
    };
    if scale.len() < 2 || tie.len() < 6 {
        return Err(invalid(
            name,
            format!("image {index} has a short tie point or pixel scale"),
        ));
    }
    let keys = match scan.find_tag(Tag::GeoKeyDirectoryTag).map_err(e)? {
        Some(v) => v.into_u16_vec().map_err(e)?,
        None => Vec::new(),
    };
    if geokey(&keys, MODEL_TYPE_KEY).is_some_and(|t| t != 2) {
        return Err(invalid(
            name,
            "isn't on a geographic CRS, so it isn't a PROJ grid",
        ));
    }
    if geokey(&keys, ANGULAR_UNITS_KEY).is_some_and(|u| u != 9102) {
        return Err(invalid(name, "has angular units other than degrees"));
    }
    let pixel_is_point = geokey(&keys, RASTER_TYPE_KEY) == Some(2);
    let (step_x, step_y) = (scale[0], scale[1]);
    let half = if pixel_is_point { 0.0 } else { 0.5 };
    let west_deg = tie[3] + (half - tie[0]) * step_x;
    let north_deg = tie[4] - (half - tie[1]) * step_y;
    let south_deg = north_deg - height.saturating_sub(1) as f64 * step_y;
    let degree = PI / 180.0;

    // GDAL's metadata: grid-level items, then per-band items.
    let mut items = Vec::new();
    if let Some(v) = scan.find_tag(Tag::Unknown(IMAGE_DESCRIPTION)).map_err(e)? {
        items.push((
            "TIFFTAG_IMAGEDESCRIPTION".to_owned(),
            v.into_string().map_err(e)?,
        ));
    }
    let mut bands = vec![BandMetadata::default(); samples];
    let mut band_scale = [1.0; MAX_BANDS];
    let mut band_offset = [0.0; MAX_BANDS];
    if let Some(v) = scan.find_tag(Tag::Unknown(GDAL_METADATA)).map_err(e)? {
        for item in parse_gdal_metadata(&v.into_string().map_err(e)?) {
            let Some(band) = item.sample else {
                items.push((item.name, item.value));
                continue;
            };
            let Some(b) = bands.get_mut(band) else {
                continue;
            };
            let number = || {
                item.value
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| {
                        invalid(
                            name,
                            format!(
                                "band {band}'s {} {:?} isn't a number",
                                item.name, item.value
                            ),
                        )
                    })
            };
            match item.role.as_deref() {
                Some("description") => b.description = Some(item.value),
                Some("unittype") => b.unit = Some(item.value),
                Some("scale") => band_scale[band] = number()?,
                Some("offset") => band_offset[band] = number()?,
                _ => b.items.push((item.name, item.value)),
            }
        }
    }
    let nodata =
        match scan.find_tag(Tag::GdalNodata).map_err(e)? {
            Some(v) => {
                let text = v.into_string().map_err(e)?;
                let text = text.trim_end_matches('\0').trim();
                Some(text.parse::<f64>().map_err(|_| {
                    invalid(name, format!("its no-data value {text:?} isn't a number"))
                })?)
            }
            None => None,
        };

    let subgrid_name = items
        .iter()
        .find(|(k, _)| k == "grid_name")
        .map_or_else(|| format!("image {index}"), |(_, v)| v.clone());
    let parent_name = items
        .iter()
        .find(|(k, _)| k == "parent_grid_name")
        .map(|(_, v)| v.clone());

    // A decoder of its own, so reading this image never disturbs another.
    let mut own = decoder(name, source)?;
    own.seek_to_image(index).map_err(e)?;

    Ok(Some(ImageInfo {
        subgrid: Subgrid {
            name: subgrid_name,
            parent: None,
            south_rad: south_deg * degree,
            west_rad: west_deg * degree,
            latitude_step_rad: step_y * degree,
            longitude_step_rad: step_x * degree,
            rows: height,
            columns: width,
        },
        parent_name,
        metadata: GridMetadata { items, bands },
        image: Image {
            decoder: Mutex::new(own),
            height,
            chunk_width,
            chunk_height,
            chunks_across,
            chunks_per_plane,
            planar,
            samples,
            scale: band_scale,
            offset: band_offset,
            nodata,
        },
    }))
}

/// Reads a GeoTIFF grid's headers: its subgrids, band count, metadata (from
/// the first image) and a reader for its nodes.
pub(super) fn open(
    name: &str,
    source: Arc<dyn GridSource>,
) -> Result<(Vec<Subgrid>, usize, GridMetadata, Reader), DatumError> {
    let mut scan = decoder(name, &source)?;
    let mut infos: Vec<ImageInfo> = Vec::new();
    for index in 0..MAX_IMAGES {
        if index > 0 {
            if !scan.more_images() {
                break;
            }
            scan.next_image().map_err(|e| tiff_error(name, e))?;
        }
        if let Some(info) = read_image(name, &source, &mut scan, index)? {
            infos.push(info);
        }
    }
    if scan.more_images() {
        return Err(invalid(
            name,
            format!("has more than {MAX_IMAGES} images, the most this reader accepts"),
        ));
    }
    let Some(first) = infos.first() else {
        return Err(invalid(name, "has no full-resolution images"));
    };
    let bands = first.image.samples;
    let metadata = first.metadata.clone();
    if infos.iter().any(|i| i.image.samples != bands) {
        return Err(invalid(
            name,
            "has subgrids with different numbers of bands",
        ));
    }

    let names: Vec<String> = infos.iter().map(|i| i.subgrid.name.clone()).collect();
    let mut subgrids = Vec::with_capacity(infos.len());
    let mut images = Vec::with_capacity(infos.len());
    for info in infos {
        let mut subgrid = info.subgrid;
        if let Some(parent) = info.parent_name {
            let index = names.iter().position(|n| *n == parent).ok_or_else(|| {
                invalid(
                    name,
                    format!(
                        "subgrid {} names its parent as {parent:?}, which isn't in the file",
                        subgrid.name
                    ),
                )
            })?;
            subgrid.parent = Some(index);
        }
        subgrids.push(subgrid);
        images.push(info.image);
    }
    let reader = Reader {
        images,
        cache: Mutex::new(Vec::new()),
    };
    Ok((subgrids, bands, metadata, reader))
}

/// A sample of a decoded chunk, as a number.
fn sample(values: &DecodingResult, index: usize) -> Option<f64> {
    Some(match values {
        DecodingResult::U8(v) => f64::from(*v.get(index)?),
        DecodingResult::U16(v) => f64::from(*v.get(index)?),
        DecodingResult::U32(v) => f64::from(*v.get(index)?),
        DecodingResult::U64(v) => *v.get(index)? as f64,
        DecodingResult::I8(v) => f64::from(*v.get(index)?),
        DecodingResult::I16(v) => f64::from(*v.get(index)?),
        DecodingResult::I32(v) => f64::from(*v.get(index)?),
        DecodingResult::I64(v) => *v.get(index)? as f64,
        DecodingResult::F16(v) => f64::from(v.get(index)?.to_f32()),
        DecodingResult::F32(v) => f64::from(*v.get(index)?),
        DecodingResult::F64(v) => *v.get(index)?,
    })
}

impl Reader {
    /// Reads the four nodes of a cell: south-west, south-east, north-west and
    /// north-east, each with every band.
    pub(super) fn read_cell(
        &self,
        grid: &str,
        subgrid: usize,
        row: usize,
        column: usize,
        nodes: &mut [[f64; MAX_BANDS]; 4],
    ) -> Result<(), DatumError> {
        let image = &self.images[subgrid];
        // Image rows run north to south.
        let south = image.height - 1 - row;
        let north = south - 1;
        let corners = [
            (south, column),
            (south, column + 1),
            (north, column),
            (north, column + 1),
        ];
        for (node, (r, c)) in nodes.iter_mut().zip(corners) {
            for (band, value) in node.iter_mut().enumerate().take(image.samples) {
                *value = self.value(grid, subgrid, band, r, c)?;
            }
        }
        Ok(())
    }

    /// One band's value at a node, in image rows and columns. NaN where the
    /// grid has no value.
    fn value(
        &self,
        grid: &str,
        index: usize,
        band: usize,
        row: usize,
        column: usize,
    ) -> Result<f64, DatumError> {
        let image = &self.images[index];
        let (cx, cy) = (column / image.chunk_width, row / image.chunk_height);
        let in_plane = cy * image.chunks_across + cx;
        let chunk_index = if image.planar {
            band * image.chunks_per_plane + in_plane
        } else {
            in_plane
        };
        let chunk = self.chunk(grid, index, chunk_index)?;
        let (x, y) = (
            column - cx * image.chunk_width,
            row - cy * image.chunk_height,
        );
        let at = if image.planar {
            y * chunk.width + x
        } else {
            (y * chunk.width + x) * image.samples + band
        };
        let raw = sample(&chunk.values, at)
            .ok_or_else(|| invalid(grid, "a tile or strip holds less data than its image needs"))?;
        if raw.is_nan() || image.nodata == Some(raw) {
            return Ok(f64::NAN);
        }
        Ok(raw * image.scale[band] + image.offset[band])
    }

    /// A decoded chunk, from the cache or the file.
    fn chunk(&self, grid: &str, image: usize, index: usize) -> Result<Arc<Chunk>, DatumError> {
        {
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(at) = cache
                .iter()
                .position(|(i, c, _)| *i == image && *c == index)
            {
                let entry = cache.remove(at);
                let chunk = Arc::clone(&entry.2);
                cache.insert(0, entry);
                return Ok(chunk);
            }
        }
        let chunk = {
            let mut decoder = self.images[image]
                .decoder
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let index = u32::try_from(index)
                .map_err(|_| invalid(grid, "has more tiles or strips than TIFF allows"))?;
            let (width, _) = decoder.chunk_data_dimensions(index);
            let values = decoder.read_chunk(index).map_err(|e| tiff_error(grid, e))?;
            Arc::new(Chunk {
                values,
                width: width as usize,
            })
        };
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.insert(0, (image, index, Arc::clone(&chunk)));
        cache.truncate(CACHE_CHUNKS);
        Ok(chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn parses_gdal_metadata() {
        let xml = r#"<GDALMetadata>
  <Item name="grid_name">WEST_DC5</Item>
  <Item name="TYPE">HORIZONTAL_OFFSET</Item>
  <Item name="DESCRIPTION" sample="0" role="description">latitude_offset</Item>
  <Item name="UNITTYPE" sample="0" role="unittype">arc-second</Item>
  <Item name="positive_value" sample="1">east</Item>
  <Item name="note">a &amp; b &lt;c&gt;</Item>
  <Item name="empty"/>
</GDALMetadata>"#;
        let items = parse_gdal_metadata(xml);
        assert_eq!(items.len(), 7);
        assert_eq!(
            items[2],
            Item {
                name: "DESCRIPTION".into(),
                sample: Some(0),
                role: Some("description".into()),
                value: "latitude_offset".into(),
            }
        );
        assert_eq!(items[4].sample, Some(1));
        assert_eq!(items[5].value, "a & b <c>");
        assert_eq!(items[6].value, "");
        assert!(parse_gdal_metadata("<Item name=\"x\">unterminated").len() == 1);
        assert!(parse_gdal_metadata("<Item").is_empty());
    }

    #[test]
    fn reads_geokeys() {
        // Version 1.1.0, two keys: model type geographic, raster type point.
        let directory = [1, 1, 0, 2, 1024, 0, 1, 2, 1025, 0, 1, 2];
        assert_eq!(geokey(&directory, MODEL_TYPE_KEY), Some(2));
        assert_eq!(geokey(&directory, RASTER_TYPE_KEY), Some(2));
        assert_eq!(geokey(&directory, ANGULAR_UNITS_KEY), None);
        assert_eq!(geokey(&directory[..6], RASTER_TYPE_KEY), None);
        assert_eq!(geokey(&[], RASTER_TYPE_KEY), None);
    }
}
