//! Corrected copies of observation files. A raw file is never changed:
//! corrections to its header are kept apart (as overlays in a project) and
//! applied to the header as it's read, or to a copy written for export.

use std::io::{BufRead, Write};

use crate::header::ObsHeader;
use crate::obs::{LineSource, ObsReader};
use crate::text::{cols, label};
use crate::time::{GnssTime, TICKS_PER_SECOND};
use crate::{RinexError, open_source};

/// Values that replace those in an observation file's header. Only those
/// given are replaced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeaderPatch {
    /// `MARKER NAME`.
    pub marker_name: Option<String>,
    /// The receiver type in `REC # / TYPE / VERS`.
    pub receiver: Option<String>,
    /// The antenna type in `ANT # / TYPE`.
    pub antenna: Option<String>,
    /// The radome in `ANT # / TYPE`.
    pub radome: Option<String>,
    /// `ANTENNA: DELTA H/E/N`, metres.
    pub antenna_delta: Option<[f64; 3]>,
}

impl HeaderPatch {
    /// Whether it replaces nothing.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Adds `later`'s values, which win.
    pub fn then(&mut self, later: &HeaderPatch) {
        let pick = |a: &mut Option<String>, b: &Option<String>| {
            if b.is_some() {
                a.clone_from(b);
            }
        };
        pick(&mut self.marker_name, &later.marker_name);
        pick(&mut self.receiver, &later.receiver);
        pick(&mut self.antenna, &later.antenna);
        pick(&mut self.radome, &later.radome);
        if later.antenna_delta.is_some() {
            self.antenna_delta = later.antenna_delta;
        }
    }
}

impl ObsHeader {
    /// The header with a patch applied.
    pub fn patch(&mut self, p: &HeaderPatch) {
        if let Some(v) = &p.marker_name {
            self.marker_name.clone_from(v);
        }
        if let Some(v) = &p.receiver {
            self.receiver.kind.clone_from(v);
        }
        if let Some(v) = &p.antenna {
            self.antenna.kind.clone_from(v);
        }
        if let Some(v) = &p.radome {
            self.antenna.radome = Some(v.clone());
        }
        if p.antenna_delta.is_some() {
            self.antenna_delta = p.antenna_delta;
        }
    }
}

/// A header line: content in columns 1 to 60, then the label.
fn line(content: &str, label: &str) -> Vec<u8> {
    let content: String = content.chars().take(60).collect();
    format!("{content:<60}{label}").into_bytes()
}

/// Columns `start..end` of a line, as text padded to their width.
fn field(old: &[u8], start: usize, end: usize) -> String {
    let text = String::from_utf8_lossy(cols(old, start, end)).into_owned();
    format!("{text:<width$}", width = end - start)
}

/// Writes an observation file, plain or Hatanaka-compressed, as plain
/// RINEX with its header patched, and `comments` added as `COMMENT`
/// records before `END OF HEADER`. Records the patch replaces that the
/// header lacks are added there too. The epochs are copied as they are
/// (header records within events aren't patched). Returns the lines
/// written.
pub fn rewrite_obs<R: BufRead, W: Write>(
    reader: R,
    mut out: W,
    patch: &HeaderPatch,
    comments: &[String],
) -> Result<u64, RinexError> {
    let mut source = open_source(reader)?;
    let mut buf = Vec::new();
    let mut header = HeaderWriter::new(patch, comments);
    while source.read(&mut buf)? {
        if header.done {
            header.emit(&mut out, &buf)?;
        } else {
            header.line(&mut out, &buf)?;
        }
    }
    if !header.done {
        return Err(RinexError::at(
            source.line(),
            "the file ended before END OF HEADER",
        ));
    }
    Ok(header.written)
}

/// Which epochs [`trim_obs`] keeps.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Trim {
    /// The first and last epochs to keep, inclusive, on the file's time
    /// scale; all if none.
    pub window: Option<(GnssTime, GnssTime)>,
    /// Keep only epochs on whole multiples of this interval, in seconds
    /// (to within 5 ms, as RTKLIB does); all if none.
    pub interval: Option<f64>,
}

/// What [`trim_obs`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trimmed {
    /// Observation epochs read.
    pub read: u64,
    /// Observation epochs kept.
    pub kept: u64,
    /// Lines written.
    pub lines: u64,
}

/// Within this of a whole interval, an epoch is on it (RTKLIB's `DTTOL`).
const INTERVAL_TOLERANCE: i64 = TICKS_PER_SECOND / 200;

/// As [`rewrite_obs`], keeping only the epochs a [`Trim`] selects, each
/// copied as it is: RTKLIB reads every epoch of a file, so trimming to the
/// processing window and interval saves memory and time without changing
/// the solution (M0 spike). Events (flags 2 to 5) are always kept, since a
/// change of equipment before the window still applies within it.
pub fn trim_obs<R: BufRead, W: Write>(
    reader: R,
    mut out: W,
    patch: &HeaderPatch,
    comments: &[String],
    trim: Trim,
) -> Result<Trimmed, RinexError> {
    let mut obs = ObsReader::capturing(open_source(reader)?)?;
    let mut header = HeaderWriter::new(patch, comments);
    for line in obs.take_lines() {
        header.line(&mut out, &line)?;
    }
    let step = trim
        .interval
        .filter(|i| *i > 0.0)
        // Ticks are whole: an interval rounds to the nearest tick.
        .map(|i| (i * TICKS_PER_SECOND as f64).round() as i64);
    let (mut read, mut kept) = (0, 0);
    while let Some(epoch) = obs.next_epoch()? {
        let lines = obs.take_lines();
        let keep = match epoch.time {
            _ if epoch.flag.is_event() => true,
            None => true,
            Some(t) => {
                let observing = epoch.flag.has_observations();
                read += u64::from(observing);
                let inside = trim.window.is_none_or(|(a, b)| a <= t && t <= b);
                let on_step = step.is_none_or(|s| {
                    let off = (t.ticks() + INTERVAL_TOLERANCE).rem_euclid(s);
                    off <= 2 * INTERVAL_TOLERANCE
                });
                let keep = inside && on_step;
                kept += u64::from(keep && observing);
                keep
            }
        };
        if keep {
            for line in &lines {
                header.emit(&mut out, line)?;
            }
        }
    }
    Ok(Trimmed {
        read,
        kept,
        lines: header.written,
    })
}

/// Writes a header's lines with a patch applied, then anything else as it
/// is, counting the lines.
struct HeaderWriter<'a> {
    patch: &'a HeaderPatch,
    comments: &'a [String],
    marker: bool,
    receiver: bool,
    antenna: bool,
    delta: bool,
    /// `END OF HEADER` has been written.
    done: bool,
    written: u64,
}

impl<'a> HeaderWriter<'a> {
    fn new(patch: &'a HeaderPatch, comments: &'a [String]) -> Self {
        Self {
            patch,
            comments,
            marker: false,
            receiver: false,
            antenna: false,
            delta: false,
            done: false,
            written: 0,
        }
    }

    fn emit<W: Write>(&mut self, out: &mut W, bytes: &[u8]) -> Result<(), RinexError> {
        out.write_all(bytes)
            .and_then(|()| out.write_all(b"\n"))
            .map_err(|e| RinexError::Io(e.to_string()))?;
        self.written += 1;
        Ok(())
    }

    /// One header line, patched.
    fn line<W: Write>(&mut self, out: &mut W, buf: &[u8]) -> Result<(), RinexError> {
        let patch = self.patch;
        let new = match label(buf) {
            b"MARKER NAME" if patch.marker_name.is_some() => {
                self.marker = true;
                Some(line(
                    patch.marker_name.as_deref().unwrap_or_default(),
                    "MARKER NAME",
                ))
            }
            b"REC # / TYPE / VERS" if patch.receiver.is_some() => {
                self.receiver = true;
                let kind = patch.receiver.as_deref().unwrap_or_default();
                Some(line(
                    &format!("{}{kind:<20.20}{}", field(buf, 0, 20), field(buf, 40, 60)),
                    "REC # / TYPE / VERS",
                ))
            }
            b"ANT # / TYPE" if patch.antenna.is_some() || patch.radome.is_some() => {
                self.antenna = true;
                let kind = patch.antenna.clone().unwrap_or_else(|| field(buf, 20, 36));
                let radome = patch.radome.clone().unwrap_or_else(|| field(buf, 36, 40));
                Some(line(
                    &format!("{}{kind:<16.16}{radome:<4.4}", field(buf, 0, 20)),
                    "ANT # / TYPE",
                ))
            }
            b"ANTENNA: DELTA H/E/N" if patch.antenna_delta.is_some() => {
                self.delta = true;
                patch.antenna_delta.map(delta_line)
            }
            b"END OF HEADER" => {
                self.done = true;
                // What the header lacked, then the comments.
                if !self.marker
                    && let Some(m) = &patch.marker_name
                {
                    self.emit(out, &line(m, "MARKER NAME"))?;
                }
                if !self.receiver
                    && let Some(r) = &patch.receiver
                {
                    self.emit(
                        out,
                        &line(&format!("{:20}{r:<20.20}", ""), "REC # / TYPE / VERS"),
                    )?;
                }
                if !self.antenna && (patch.antenna.is_some() || patch.radome.is_some()) {
                    let kind = patch.antenna.as_deref().unwrap_or_default();
                    let radome = patch.radome.as_deref().unwrap_or("NONE");
                    self.emit(
                        out,
                        &line(
                            &format!("{:20}{kind:<16.16}{radome:<4.4}", ""),
                            "ANT # / TYPE",
                        ),
                    )?;
                }
                if !self.delta
                    && let Some(d) = patch.antenna_delta
                {
                    self.emit(out, &delta_line(d))?;
                }
                for c in self.comments {
                    // Comments longer than a record go on over more.
                    let chars: Vec<char> = c.chars().collect();
                    for chunk in chars.chunks(60) {
                        self.emit(out, &line(&chunk.iter().collect::<String>(), "COMMENT"))?;
                    }
                }
                None
            }
            _ => None,
        };
        self.emit(out, new.as_deref().unwrap_or(buf))
    }
}

fn delta_line([h, e, n]: [f64; 3]) -> Vec<u8> {
    line(&format!("{h:14.4}{e:14.4}{n:14.4}"), "ANTENNA: DELTA H/E/N")
}
