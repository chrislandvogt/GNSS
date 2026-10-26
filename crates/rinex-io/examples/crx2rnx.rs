//! Restores a Hatanaka-compressed file to RINEX on standard output, for
//! comparing with the reference CRX2RNX: `cargo run --release --example
//! crx2rnx -- file.crx > file.rnx`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufReader, BufWriter};

fn main() {
    let path = std::env::args().nth(1).expect("usage: crx2rnx <file>");
    let input = BufReader::new(std::fs::File::open(&path).unwrap());
    let output = BufWriter::new(std::io::stdout().lock());
    if let Err(e) = gnss_rinex_io::decompress(input, output) {
        eprintln!("{path}: {e}");
        std::process::exit(1);
    }
}
