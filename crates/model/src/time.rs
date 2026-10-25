use std::fmt;

use hifitime::{Epoch, TimeScale};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ModelError;

/// An instant in UTC, written in files as `2026-10-14T00:12:00Z` (RFC 3339,
/// UTC only, with optional fractional seconds to the nanosecond).
///
/// Times are compared and subtracted as hifitime epochs, so spans across a
/// leap second come out right.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Timestamp(Epoch);

impl Timestamp {
    /// Parses `YYYY-MM-DDTHH:MM:SS[.fff]Z`.
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        let bad = || {
            ModelError::Invalid(format!(
                "time {text:?} isn't UTC in the form 2026-10-14T00:12:00Z"
            ))
        };
        let body = text.strip_suffix('Z').ok_or_else(bad)?;
        let (date, clock) = body.split_once('T').ok_or_else(bad)?;
        let date: Vec<&str> = date.split('-').collect();
        // A '.' must be followed by digits.
        let (whole, fraction) = match clock.split_once('.') {
            Some((_, "")) => return Err(bad()),
            Some(parts) => parts,
            None => (clock, ""),
        };
        let clock: Vec<&str> = whole.split(':').collect();
        let ([year, month, day], [hour, minute, second]) = (&date[..], &clock[..]) else {
            return Err(bad());
        };
        let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
        if !(digits(year, 4)
            && [*month, *day, *hour, *minute, *second]
                .iter()
                .all(|s| digits(s, 2))
            && fraction.len() <= 9
            && fraction.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(bad());
        }
        let number = |s: &str| s.parse::<u32>().map_err(|_| bad());
        let nanos = if fraction.is_empty() {
            0
        } else {
            number(fraction)? * 10u32.pow(9 - fraction.len() as u32)
        };
        let (hour, minute, second) = (number(hour)?, number(minute)?, number(second)?);
        // hifitime rolls 24:00 over to the next day; a file shouldn't. A
        // second of 60 is a leap second.
        if hour > 23 || minute > 59 || second > 60 {
            return Err(bad());
        }
        let epoch = Epoch::maybe_from_gregorian(
            number(year)? as i32,
            number(month)? as u8,
            number(day)? as u8,
            hour as u8,
            minute as u8,
            second as u8,
            nanos,
            TimeScale::UTC,
        )
        .map_err(|_| bad())?;
        Ok(Self(epoch))
    }

    /// The instant as a hifitime epoch.
    pub fn epoch(self) -> Epoch {
        self.0
    }

    /// Seconds from `self` to `later`, negative if `later` is earlier.
    pub fn seconds_until(self, later: Timestamp) -> f64 {
        // In TAI: UTC differences in hifitime leave out leap seconds.
        (later.0.to_time_scale(TimeScale::TAI) - self.0.to_time_scale(TimeScale::TAI)).to_seconds()
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (year, month, day, hour, minute, second, nanos) = self.0.to_gregorian(TimeScale::UTC);
        write!(
            f,
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}"
        )?;
        if nanos != 0 {
            let fraction = format!("{nanos:09}");
            write!(f, ".{}", fraction.trim_end_matches('0'))?;
        }
        f.write_str("Z")
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Timestamp::parse(&String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::Timestamp;

    #[test]
    fn utc_times_round_trip() {
        for text in [
            "2026-10-14T00:12:00Z",
            "2026-10-14T02:31:30.5Z",
            "2000-02-29T23:59:59.123456789Z",
        ] {
            assert_eq!(Timestamp::parse(text).unwrap().to_string(), text);
        }
        assert_eq!(
            Timestamp::parse("2026-10-14T00:12:00.500Z")
                .unwrap()
                .to_string(),
            "2026-10-14T00:12:00.5Z"
        );
    }

    #[test]
    fn spans_count_the_leap_second() {
        let before = Timestamp::parse("2016-12-31T23:59:00Z").unwrap();
        let after = Timestamp::parse("2017-01-01T00:01:00Z").unwrap();
        assert_eq!(before.seconds_until(after), 121.0);
        assert!(before < after);
    }

    #[test]
    fn only_utc_in_one_form_is_accepted() {
        for bad in [
            "2026-10-14 00:12:00Z",
            "2026-10-14T00:12:00",
            "2026-10-14T00:12:00+10:00",
            "2026-10-14T00:12Z",
            "26-10-14T00:12:00Z",
            "2026-13-14T00:12:00Z",
            "2026-02-30T00:12:00Z",
            "2026-10-14T24:00:00Z",
            "2026-10-14T00:12:00.Z",
            "2026-10-14T00:12:00.1234567891Z",
        ] {
            assert!(Timestamp::parse(bad).is_err(), "{bad}");
        }
    }
}
