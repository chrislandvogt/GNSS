//! Fixed-column fields. RINEX is a fixed-format text file, so every field is
//! a range of columns. These never panic: a line that's too short gives
//! empty fields, and anything that isn't text is read as text anyway.

/// The bytes in columns `start..end` (counting from 0), cut to the line.
pub(crate) fn cols(line: &[u8], start: usize, end: usize) -> &[u8] {
    let end = end.min(line.len());
    line.get(start.min(end)..end).unwrap_or_default()
}

/// The text in columns `start..end`, trimmed.
pub(crate) fn text(line: &[u8], start: usize, end: usize) -> String {
    String::from_utf8_lossy(cols(line, start, end))
        .trim()
        .to_owned()
}

/// The header label, columns 61 to 80.
pub(crate) fn label(line: &[u8]) -> &[u8] {
    trim_end(cols(line, 60, 80))
}

/// A number in columns `start..end`, or `None` if they're blank or aren't one.
pub(crate) fn number<T: std::str::FromStr>(line: &[u8], start: usize, end: usize) -> Option<T> {
    std::str::from_utf8(cols(line, start, end))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// The bytes without trailing spaces.
pub(crate) fn trim_end(bytes: &[u8]) -> &[u8] {
    let len = bytes.iter().rposition(|&b| b != b' ').map_or(0, |i| i + 1);
    bytes.get(..len).unwrap_or_default()
}

/// The leading integer, as C's `atoi` reads it: spaces, an optional sign,
/// then digits, stopping at anything else. `None` if there are no digits.
pub(crate) fn leading_int(bytes: &[u8]) -> Option<i64> {
    let mut rest = bytes.iter().skip_while(|&&b| b == b' ').peekable();
    let negative = match rest.peek() {
        Some(b'-') => {
            rest.next();
            true
        }
        Some(b'+') => {
            rest.next();
            false
        }
        _ => false,
    };
    let mut value: i64 = 0;
    let mut any = false;
    for &b in rest.take_while(|b| b.is_ascii_digit()) {
        value = value.checked_mul(10)?.checked_add(i64::from(b - b'0'))?;
        any = true;
    }
    any.then_some(if negative { -value } else { value })
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{cols, label, leading_int, number, text, trim_end};

    #[test]
    fn fields_never_run_off_the_line() {
        assert_eq!(cols(b"abc", 1, 10), b"bc");
        assert_eq!(cols(b"abc", 5, 10), b"");
        assert_eq!(cols(b"abc", 2, 1), b"");
        assert_eq!(text(b"  x  y ", 0, 99), "x  y");
        assert_eq!(number::<f64>(b"   1.5x", 0, 6), Some(1.5));
        assert_eq!(number::<i32>(b"  12", 0, 99), Some(12));
        assert_eq!(number::<i32>(b"    ", 0, 4), None);
        assert_eq!(trim_end(b"ab  "), b"ab");
        assert_eq!(trim_end(b"   "), b"");
        let mut line = vec![b' '; 60];
        line.extend_from_slice(b"END OF HEADER       ");
        assert_eq!(label(&line), b"END OF HEADER");
    }

    #[test]
    fn integers_read_like_atoi() {
        assert_eq!(leading_int(b"  8G01G02"), Some(8));
        assert_eq!(leading_int(b"-12&3"), Some(-12));
        assert_eq!(leading_int(b"x"), None);
        assert_eq!(leading_int(b"99999999999999999999"), None);
    }
}
