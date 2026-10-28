//! Corrected copies of observation files. A raw file is never changed:
//! corrections to its header are kept apart (as overlays in a project) and
//! applied to the header as it's read, or to a copy written for export.

use std::io::{BufRead, Write};

use crate::header::ObsHeader;
use crate::obs::LineSource;
use crate::text::{cols, label};
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
    let mut written = 0u64;
    let mut emit = |out: &mut W, bytes: &[u8]| -> Result<(), RinexError> {
        out.write_all(bytes)
            .and_then(|()| out.write_all(b"\n"))
            .map_err(|e| RinexError::Io(e.to_string()))?;
        written += 1;
        Ok(())
    };
    let (mut marker, mut receiver, mut antenna, mut delta) = (false, false, false, false);
    let mut in_header = true;
    while source.read(&mut buf)? {
        if !in_header {
            emit(&mut out, &buf)?;
            continue;
        }
        let new = match label(&buf) {
            b"MARKER NAME" if patch.marker_name.is_some() => {
                marker = true;
                Some(line(
                    patch.marker_name.as_deref().unwrap_or_default(),
                    "MARKER NAME",
                ))
            }
            b"REC # / TYPE / VERS" if patch.receiver.is_some() => {
                receiver = true;
                let kind = patch.receiver.as_deref().unwrap_or_default();
                Some(line(
                    &format!("{}{kind:<20.20}{}", field(&buf, 0, 20), field(&buf, 40, 60)),
                    "REC # / TYPE / VERS",
                ))
            }
            b"ANT # / TYPE" if patch.antenna.is_some() || patch.radome.is_some() => {
                antenna = true;
                let kind = patch.antenna.clone().unwrap_or_else(|| field(&buf, 20, 36));
                let radome = patch.radome.clone().unwrap_or_else(|| field(&buf, 36, 40));
                Some(line(
                    &format!("{}{kind:<16.16}{radome:<4.4}", field(&buf, 0, 20)),
                    "ANT # / TYPE",
                ))
            }
            b"ANTENNA: DELTA H/E/N" if patch.antenna_delta.is_some() => {
                delta = true;
                patch.antenna_delta.map(delta_line)
            }
            b"END OF HEADER" => {
                in_header = false;
                // What the header lacked, then the comments.
                if !marker && let Some(m) = &patch.marker_name {
                    emit(&mut out, &line(m, "MARKER NAME"))?;
                }
                if !receiver && let Some(r) = &patch.receiver {
                    emit(
                        &mut out,
                        &line(&format!("{:20}{r:<20.20}", ""), "REC # / TYPE / VERS"),
                    )?;
                }
                if !antenna && (patch.antenna.is_some() || patch.radome.is_some()) {
                    let kind = patch.antenna.as_deref().unwrap_or_default();
                    let radome = patch.radome.as_deref().unwrap_or("NONE");
                    emit(
                        &mut out,
                        &line(
                            &format!("{:20}{kind:<16.16}{radome:<4.4}", ""),
                            "ANT # / TYPE",
                        ),
                    )?;
                }
                if !delta && let Some(d) = patch.antenna_delta {
                    emit(&mut out, &delta_line(d))?;
                }
                for c in comments {
                    // Comments longer than a record go on over more.
                    let chars: Vec<char> = c.chars().collect();
                    for chunk in chars.chunks(60) {
                        emit(
                            &mut out,
                            &line(&chunk.iter().collect::<String>(), "COMMENT"),
                        )?;
                    }
                }
                None
            }
            _ => None,
        };
        emit(&mut out, new.as_deref().unwrap_or(&buf))?;
    }
    if in_header {
        return Err(RinexError::at(
            source.line(),
            "the file ended before END OF HEADER",
        ));
    }
    Ok(written)
}

fn delta_line([h, e, n]: [f64; 3]) -> Vec<u8> {
    line(&format!("{h:14.4}{e:14.4}{n:14.4}"), "ANTENNA: DELTA H/E/N")
}
