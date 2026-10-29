//! RINEX ingest, header reconciliation and quality metrics.
//!
//! See `docs/ARCHITECTURE.md` (section 3) for this crate's place in the
//! design. It reads RINEX 2.11, 3.0x and 4.0x observation files, plain or
//! Hatanaka-compressed, a line at a time, so a whole day at 1 Hz never has
//! to be in memory at once. It reads navigation files (2.11 to 4.0x) too,
//! for broadcast orbits ([`orbit`]). It never panics on what it reads:
//! anything malformed is an error naming its line (D-027).

#![cfg_attr(not(test), warn(clippy::indexing_slicing))]

mod crinex;
mod error;
mod gz;
mod header;
mod lines;
pub mod nav;
mod obs;
pub mod occupation;
pub mod orbit;
pub mod qc;
mod rewrite;
pub mod signal;
mod text;
mod time;

use std::io::{BufRead, Write};

pub use error::RinexError;
pub use gz::{Decoded, decoded};
pub use header::{Antenna, ObsHeader, ObsTypes, Receiver, Version};
pub use nav::{NavFile, read_nav};
pub use obs::{Epoch, EpochFlag, ObsReader, Observation, SatObservations, Sv};
pub use rewrite::{HeaderPatch, rewrite_obs};
pub use time::{GnssTime, TICKS_PER_SECOND, TimeSystem};

use crinex::Crinex;
use lines::Lines;
use obs::LineSource;

/// Where an observation file's lines come from.
pub(crate) enum Source<R> {
    /// A plain RINEX file, and its first line, already read.
    Plain(Lines<R>, Option<Vec<u8>>),
    /// A Hatanaka-compressed file, restored as it's read.
    Compact(Box<Crinex<R>>),
}

impl<R: BufRead> LineSource for Source<R> {
    fn read(&mut self, buf: &mut Vec<u8>) -> Result<bool, RinexError> {
        match self {
            Self::Plain(lines, first) => match first.take() {
                Some(line) => {
                    *buf = line;
                    Ok(true)
                }
                None => lines.read(buf),
            },
            Self::Compact(crinex) => crinex.read(buf),
        }
    }

    fn line(&self) -> u64 {
        match self {
            Self::Plain(lines, _) => lines.line(),
            Self::Compact(crinex) => crinex.line(),
        }
    }
}

/// Opens a RINEX observation file, plain or Hatanaka-compressed (`.crx`,
/// `.??d`), gzipped or not, and reads its header. Its epochs are then read
/// one at a time.
///
/// In a compressed file, error line numbers count lines of the restored
/// RINEX file.
pub fn read_obs<R: BufRead>(reader: R) -> Result<ObsReader<Decoded<R>>, RinexError> {
    ObsReader::new(open_source(reader)?)
}

/// An observation file's lines, gunzipped and restored if it's compressed.
pub(crate) fn open_source<R: BufRead>(reader: R) -> Result<Source<Decoded<R>>, RinexError> {
    let mut lines = Lines::new(decoded(reader)?);
    let mut first = Vec::new();
    if !lines.read(&mut first)? {
        return Err(RinexError::at(1, "the file is empty"));
    }
    Ok(if text::label(&first) == b"CRINEX VERS   / TYPE" {
        Source::Compact(Box::new(Crinex::new(lines, &first)?))
    } else {
        Source::Plain(lines, Some(first))
    })
}

/// Restores a Hatanaka-compressed file to RINEX, a line at a time. The
/// output is byte for byte what CRX2RNX 4.1.0 writes. Returns the number of
/// lines written.
pub fn decompress<R: BufRead, W: Write>(reader: R, mut out: W) -> Result<u64, RinexError> {
    let mut lines = Lines::new(decoded(reader)?);
    let mut first = Vec::new();
    if !lines.read(&mut first)? || text::label(&first) != b"CRINEX VERS   / TYPE" {
        return Err(RinexError::at(
            1,
            "this isn't a Hatanaka-compressed file: it doesn't start with CRINEX VERS / TYPE",
        ));
    }
    let mut crinex = Crinex::new(lines, &first)?;
    let mut buf = Vec::new();
    while crinex.read(&mut buf)? {
        buf.push(b'\n');
        out.write_all(&buf)
            .map_err(|e| RinexError::Io(e.to_string()))?;
    }
    Ok(crinex.line())
}

#[cfg(all(test, target_arch = "wasm32"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);
