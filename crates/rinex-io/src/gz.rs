//! Gzip-compressed files (`.gz`), as archives publish RINEX, read as they
//! come: decompressed a block at a time, never whole.

use std::io::{self, BufRead, BufReader, Read};

use flate2::bufread::MultiGzDecoder;

use crate::RinexError;

/// A file's bytes, decompressed if it's gzipped.
pub enum Decoded<R> {
    /// Not compressed.
    Plain(R),
    /// Gzipped: every member, one after another, as `gunzip` reads them.
    Gzip(BufReader<MultiGzDecoder<R>>),
}

/// The file's bytes as RINEX: gunzipped if it starts like gzip. A Unix
/// `compress` (`.Z`) file is refused, since nothing reads those in Rust.
pub fn decoded<R: BufRead>(mut reader: R) -> Result<Decoded<R>, RinexError> {
    let head = reader
        .fill_buf()
        .map_err(|e| RinexError::Io(e.to_string()))?;
    if head.starts_with(&[0x1f, 0x8b]) {
        return Ok(Decoded::Gzip(BufReader::with_capacity(
            1 << 16,
            MultiGzDecoder::new(reader),
        )));
    }
    if head.starts_with(&[0x1f, 0x9d]) {
        return Err(RinexError::at(
            1,
            "this file is Unix-compressed (.Z); decompress it first",
        ));
    }
    Ok(Decoded::Plain(reader))
}

impl<R: BufRead> Read for Decoded<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Decoded::Plain(r) => r.read(buf),
            Decoded::Gzip(r) => r.read(buf).map_err(damaged),
        }
    }
}

fn damaged(e: io::Error) -> io::Error {
    io::Error::new(
        e.kind(),
        format!("the gzip data is damaged or cut short ({e})"),
    )
}

impl<R: BufRead> BufRead for Decoded<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        match self {
            Decoded::Plain(r) => r.fill_buf(),
            Decoded::Gzip(r) => r.fill_buf().map_err(damaged),
        }
    }

    fn consume(&mut self, amount: usize) {
        match self {
            Decoded::Plain(r) => r.consume(amount),
            Decoded::Gzip(r) => r.consume(amount),
        }
    }
}
