//! Reading a file a line at a time, without ever holding all of it, so a
//! 24-hour 1 Hz file can be read in slices (ARCHITECTURE.md section 9.1).

use std::io::BufRead;

use crate::RinexError;

/// The longest line accepted. RINEX lines are at most a few hundred
/// characters; this stops a file with no line breaks from filling memory.
pub(crate) const MAX_LINE: usize = 64 * 1024;

/// Lines from a reader, without their line endings (LF or CR LF).
pub(crate) struct Lines<R> {
    reader: R,
    /// The number of the last line read, from 1.
    line: u64,
}

impl<R: BufRead> Lines<R> {
    pub(crate) fn new(reader: R) -> Self {
        Self { reader, line: 0 }
    }

    /// The number of the last line read, from 1.
    pub(crate) fn line(&self) -> u64 {
        self.line
    }

    /// Reads the next line into `buf`. False at the end of the file.
    pub(crate) fn read(&mut self, buf: &mut Vec<u8>) -> Result<bool, RinexError> {
        buf.clear();
        loop {
            let available = match self.reader.fill_buf() {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(RinexError::Io(e.to_string())),
            };
            if available.is_empty() {
                if buf.is_empty() {
                    return Ok(false);
                }
                break;
            }
            let (chunk, used, done) = match available.iter().position(|&b| b == b'\n') {
                Some(i) => (available.get(..i).unwrap_or_default(), i + 1, true),
                None => (available, available.len(), false),
            };
            if buf.len() + chunk.len() > MAX_LINE {
                return Err(RinexError::at(
                    self.line + 1,
                    format!(
                        "the line is longer than {MAX_LINE} characters; this isn't a RINEX file"
                    ),
                ));
            }
            buf.extend_from_slice(chunk);
            self.reader.consume(used);
            if done {
                break;
            }
        }
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
        self.line += 1;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{Lines, MAX_LINE};

    #[test]
    fn lines_lose_their_endings_and_are_counted() {
        let mut lines = Lines::new(&b"a\r\nbc\n\nlast"[..]);
        let mut buf = Vec::new();
        let mut seen = Vec::new();
        while lines.read(&mut buf).unwrap() {
            seen.push((lines.line(), String::from_utf8(buf.clone()).unwrap()));
        }
        assert_eq!(
            seen,
            [
                (1, "a".into()),
                (2, "bc".into()),
                (3, String::new()),
                (4, "last".into())
            ]
        );
    }

    #[test]
    fn a_file_without_line_breaks_is_refused() {
        let data = vec![b'x'; MAX_LINE + 1];
        let mut lines = Lines::new(std::io::BufReader::with_capacity(1000, &data[..]));
        let err = lines.read(&mut Vec::new()).unwrap_err();
        assert!(
            err.to_string().starts_with("line 1: the line is longer"),
            "{err}"
        );
    }
}
