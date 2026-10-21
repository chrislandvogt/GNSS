//! Reading and writing numbers, angles and dates as exchange formats write them.

use std::borrow::Cow;

use gnss_datum::CoordinateEpoch;
use gnss_datum::hifitime::{Duration, Epoch, TimeScale};

/// A file's text: UTF-8 without its byte order mark if it's valid UTF-8, and
/// otherwise Latin-1 (ISO 8859-1), which older Windows programs write and
/// which can't fail to decode.
pub fn decode(bytes: &[u8]) -> Cow<'_, str> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Cow::Borrowed(text.strip_prefix('\u{feff}').unwrap_or(text)),
        Err(_) => Cow::Owned(bytes.iter().map(|&b| char::from(b)).collect()),
    }
}

/// A finite number, or a message saying which field isn't one.
pub(crate) fn number(field: &str, text: &str) -> Result<f64, String> {
    let trimmed = text.trim();
    match trimmed.parse::<f64>() {
        Ok(v) if v.is_finite() => Ok(v),
        _ if trimmed.is_empty() => Err(format!("{field} is empty")),
        _ => Err(format!("{field} isn't a number: {trimmed:?}")),
    }
}

/// An angle written in HP notation, `±ddd.mmssss…` (degrees, then two digits
/// of minutes, then seconds), as decimal degrees. The digits are read as
/// written, so no rounding comes from splitting a binary fraction.
pub(crate) fn hp_degrees(field: &str, text: &str) -> Result<f64, String> {
    let trimmed = text.trim();
    let bad = || format!("{field} isn't an angle in ddd.mmss notation: {trimmed:?}");
    let (negative, unsigned) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    if !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    let mut digits = fraction.to_owned();
    while digits.len() < 4 {
        digits.push('0');
    }
    let degrees: f64 = whole.parse().map_err(|_| bad())?;
    let minutes: f64 = digits[..2].parse().map_err(|_| bad())?;
    let seconds: f64 = format!("{}.{}", &digits[2..4], &digits[4..])
        .trim_end_matches('.')
        .parse()
        .map_err(|_| bad())?;
    if minutes >= 60.0 || seconds >= 60.0 {
        return Err(format!(
            "{field} has {minutes} minutes and {seconds} seconds, but both must be under 60: {trimmed:?}"
        ));
    }
    let value = degrees + minutes / 60.0 + seconds / 3600.0;
    Ok(if negative { -value } else { value })
}

/// Decimal degrees in HP notation, `±ddd.mmssss…`, with seconds to `decimals`
/// places, carrying 60 seconds into a minute and 60 minutes into a degree.
pub(crate) fn hp_text(degrees: f64, decimals: usize) -> String {
    let sign = if degrees < 0.0 { "-" } else { "" };
    let scale = ten_to(decimals);
    // Work in units of the last printed decimal of a second, so the carries are exact.
    let total = (degrees.abs() * 3600.0 * scale).round();
    let per_minute = 60.0 * scale;
    let per_degree = 3600.0 * scale;
    let whole_degrees = (total / per_degree).floor();
    let rest = total - whole_degrees * per_degree;
    let minutes = (rest / per_minute).floor();
    let units = rest - minutes * per_minute;
    let seconds = units / scale;
    let seconds_text = format!("{seconds:0width$.decimals$}", width = decimals + 3);
    let digits = seconds_text.replace('.', "");
    format!("{sign}{whole_degrees:.0}.{minutes:02.0}{digits}")
}

/// 10 to a small power, exactly, without the platform's maths library (D-024).
fn ten_to(exponent: usize) -> f64 {
    (0..exponent).fold(1.0, |acc, _| acc * 10.0)
}

/// A date written `dd.mm.yyyy`, as DynaML writes epochs, as a coordinate epoch
/// at the start of that day.
pub(crate) fn dmy_epoch(field: &str, text: &str) -> Result<CoordinateEpoch, String> {
    let trimmed = text.trim();
    let bad = || format!("{field} isn't a date in dd.mm.yyyy form: {trimmed:?}");
    let parts: Vec<&str> = trimmed.split('.').collect();
    let [day, month, year] = parts[..] else {
        return Err(bad());
    };
    let day: u8 = day.parse().map_err(|_| bad())?;
    let month: u8 = month.parse().map_err(|_| bad())?;
    let year: i32 = year.parse().map_err(|_| bad())?;
    date_epoch(year, month, day).ok_or_else(bad)
}

/// An epoch written as a decimal year (`2026.79`), an ISO date
/// (`2026-10-14`, the start of the day) or date and time in UTC
/// (`2026-10-14T03:00:00Z`, the `Z` optional, or a space for the `T`), or a
/// DynaML date (`14.10.2026`).
pub(crate) fn epoch_text(field: &str, text: &str) -> Result<CoordinateEpoch, String> {
    let trimmed = text.trim();
    let bad = || {
        format!(
            "{field} should be a decimal year, a date (2026-10-14) or a date and time \
             (2026-10-14T03:00:00Z), not {trimmed:?}"
        )
    };
    if trimmed.matches('.').count() == 2 {
        return dmy_epoch(field, trimmed);
    }
    if !trimmed.contains('-') {
        let year: f64 = trimmed.parse().map_err(|_| bad())?;
        return CoordinateEpoch::from_decimal_year(year).map_err(|e| format!("{field}: {e}"));
    }
    let (date, clock) = match trimmed.split_once(['T', ' ']) {
        Some((d, c)) => (d, Some(c.trim_end_matches('Z'))),
        None => (trimmed, None),
    };
    let parts: Vec<&str> = date.split('-').collect();
    let [year, month, day] = parts[..] else {
        return Err(bad());
    };
    let (year, month, day): (i32, u8, u8) = (
        year.parse().map_err(|_| bad())?,
        month.parse().map_err(|_| bad())?,
        day.parse().map_err(|_| bad())?,
    );
    let (hour, minute, second) = match clock {
        None => (0, 0, 0.0),
        Some(c) => {
            let hms: Vec<&str> = c.split(':').collect();
            match hms[..] {
                [h, m] => (
                    h.parse().map_err(|_| bad())?,
                    m.parse().map_err(|_| bad())?,
                    0.0,
                ),
                [h, m, s] => (
                    h.parse().map_err(|_| bad())?,
                    m.parse().map_err(|_| bad())?,
                    s.parse::<f64>().map_err(|_| bad())?,
                ),
                _ => return Err(bad()),
            }
        }
    };
    if !(0.0..61.0).contains(&second) {
        return Err(bad());
    }
    let whole = second.floor();
    let nanos = ((second - whole) * 1e9).round().min(999_999_999.0) as u32;
    // A date alone is the start of the day, as DynaML's dates are; a time of
    // day is in UTC.
    let scale = if clock.is_some() {
        TimeScale::UTC
    } else {
        TimeScale::GPST
    };
    let time =
        Epoch::maybe_from_gregorian(year, month, day, hour, minute, whole as u8, nanos, scale)
            .map_err(|_| bad())?;
    CoordinateEpoch::from_time(time).map_err(|e| format!("{field}: {e}"))
}

/// The coordinate epoch at the start of a day, if the date exists.
pub(crate) fn date_epoch(year: i32, month: u8, day: u8) -> Option<CoordinateEpoch> {
    let time = Epoch::maybe_from_gregorian(year, month, day, 0, 0, 0, 0, TimeScale::GPST).ok()?;
    CoordinateEpoch::from_time(time).ok()
}

/// A coordinate epoch as the date `dd.mm.yyyy` it falls on, in GPST. The time
/// is rounded to the second first, so the start of a day read from a date
/// doesn't come back a nanosecond early, on the day before.
pub(crate) fn epoch_dmy(epoch: CoordinateEpoch) -> Option<String> {
    let time = epoch.to_time().ok()?.round(Duration::from_seconds(1.0));
    let (year, month, day, ..) = time.to_gregorian(TimeScale::GPST);
    Some(format!("{day:02}.{month:02}.{year:04}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn reads_hp_notation_as_written() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
        assert!(close(
            hp_degrees("latitude", "-36.3348253511").unwrap(),
            -(36.0 + 33.0 / 60.0 + 48.253511 / 3600.0)
        ));
        assert!(close(hp_degrees("x", "145.3").unwrap(), 145.5));
        assert!(close(hp_degrees("x", "12").unwrap(), 12.0));
        assert!(close(hp_degrees("x", "+0.0001").unwrap(), 1.0 / 3600.0));
        assert!(hp_degrees("x", "10.6000").is_err());
        assert!(hp_degrees("x", "10.0060").is_err());
        assert!(hp_degrees("x", "1e3").is_err());
        assert!(hp_degrees("x", "").is_err());
        assert!(hp_degrees("x", "-").is_err());
    }

    #[test]
    fn writes_hp_notation_with_carries() {
        assert_eq!(
            hp_text(-(36.0 + 33.0 / 60.0 + 48.253511 / 3600.0), 6),
            "-36.3348253511"
        );
        assert_eq!(hp_text(145.5, 4), "145.30000000");
        // 59.99999996 seconds rounds up into the next minute, then degree.
        assert_eq!(
            hp_text(10.0 + 59.0 / 60.0 + 59.999_999_96 / 3600.0, 6),
            "11.0000000000"
        );
        for text in [
            "-36.3348253511",
            "145.5741006918",
            "0.0000000001",
            "-0.0000000000",
        ] {
            let degrees = hp_degrees("x", text).unwrap();
            assert_eq!(
                hp_degrees("x", &hp_text(degrees, 6)).unwrap(),
                degrees,
                "{text}"
            );
        }
    }

    #[test]
    fn reads_and_writes_dates() {
        let e = dmy_epoch("epoch", "18.02.2015").unwrap();
        assert!((e.decimal_year() - (2015.0 + 48.0 / 365.0)).abs() < 1e-12);
        assert_eq!(epoch_dmy(e).unwrap(), "18.02.2015");
        assert_eq!(
            dmy_epoch("epoch", "01.01.2020").unwrap().decimal_year(),
            2020.0
        );
        assert!(dmy_epoch("epoch", "29.02.2019").is_err());
        assert!(dmy_epoch("epoch", "2019-02-01").is_err());
        assert!(dmy_epoch("epoch", "1.1").is_err());
    }

    #[test]
    fn reads_epochs_in_several_forms() {
        let year = |t: &str| epoch_text("epoch", t).unwrap().decimal_year();
        assert_eq!(year("2026.5"), 2026.5);
        assert_eq!(year("2020-01-01"), 2020.0);
        assert_eq!(year("01.01.2020"), 2020.0);
        // UTC is 18 seconds behind GPST.
        let noon = 2020.0 + (182.0 * 86_400.0 + 43_218.0) / (366.0 * 86_400.0);
        assert!((year("2020-07-01T12:00:00Z") - noon).abs() < 1e-12);
        assert!((year("2020-07-01 12:00") - noon).abs() < 1e-12);
        for bad in ["soon", "2020-13-01", "2020-01-01T25:00:00", "1.2.3.4", ""] {
            assert!(epoch_text("epoch", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn decodes_utf8_and_latin1() {
        assert_eq!(decode("\u{feff}Mt Ōmu".as_bytes()), "Mt Ōmu");
        assert_eq!(decode(b"caf\xe9"), "café");
    }

    #[test]
    fn numbers_must_be_finite() {
        assert_eq!(number("x", " 1.5e-3 ").unwrap(), 0.0015);
        assert!(number("x", "nan").is_err());
        assert!(number("x", "inf").is_err());
        assert_eq!(number("x", "").unwrap_err(), "x is empty");
    }
}
