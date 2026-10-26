//! Epoch times, kept exactly. RINEX writes seconds to 0.1 µs, so a time is a
//! whole number of 100 ns ticks, compared and subtracted without rounding.

use std::fmt;

/// The time scale a file's epochs are in (`TIME OF FIRST OBS`). GPS time is
/// the default for GPS and mixed files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeSystem {
    /// GPS time.
    Gps,
    /// GLONASS time (UTC(SU) plus three hours, with leap seconds).
    Glonass,
    /// Galileo system time.
    Galileo,
    /// BeiDou time.
    BeiDou,
    /// QZSS time.
    Qzss,
    /// NavIC (IRNSS) time.
    Irnss,
    /// UTC.
    Utc,
    /// International Atomic Time.
    Tai,
}

impl TimeSystem {
    /// Reads the three-letter code, such as `GPS`.
    pub fn parse(code: &str) -> Option<Self> {
        Some(match code {
            "GPS" => Self::Gps,
            "GLO" => Self::Glonass,
            "GAL" => Self::Galileo,
            "BDT" => Self::BeiDou,
            "QZS" => Self::Qzss,
            "IRN" => Self::Irnss,
            "UTC" => Self::Utc,
            "TAI" => Self::Tai,
            _ => return None,
        })
    }

    /// The three-letter code.
    pub fn code(self) -> &'static str {
        match self {
            Self::Gps => "GPS",
            Self::Glonass => "GLO",
            Self::Galileo => "GAL",
            Self::BeiDou => "BDT",
            Self::Qzss => "QZS",
            Self::Irnss => "IRN",
            Self::Utc => "UTC",
            Self::Tai => "TAI",
        }
    }
}

/// A time on the file's own time scale, as 100 ns ticks since
/// 1980-01-06 00:00:00. Within one scale, differences are exact elapsed
/// time except across a leap second in a UTC or GLONASS file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GnssTime(i64);

/// Ticks in one second.
pub const TICKS_PER_SECOND: i64 = 10_000_000;
const TICKS_PER_DAY: i64 = 86_400 * TICKS_PER_SECOND;
/// 1980-01-06, as days since 1970-01-01.
const GPS_EPOCH_DAYS: i64 = 3657;

impl GnssTime {
    /// From calendar fields. `seconds` is the text of the seconds field, such
    /// as `30.0000000`, read exactly. `None` if any field is out of range.
    pub fn from_fields(
        year: i64,
        month: i64,
        day: i64,
        hour: i64,
        minute: i64,
        seconds: &str,
    ) -> Option<Self> {
        if !(1..=12).contains(&month)
            || !(1..=days_in_month(year, month)).contains(&day)
            || !(0..=23).contains(&hour)
            || !(0..=59).contains(&minute)
        {
            return None;
        }
        let ticks = second_ticks(seconds)?;
        // Up to 60.9999999 s, for a leap second in a UTC file.
        if ticks >= 61 * TICKS_PER_SECOND {
            return None;
        }
        let days = days_from_civil(year, month, day) - GPS_EPOCH_DAYS;
        let ticks = days
            .checked_mul(TICKS_PER_DAY)?
            .checked_add((hour * 3600 + minute * 60) * TICKS_PER_SECOND)?
            .checked_add(ticks)?;
        Some(Self(ticks))
    }

    /// The ticks since 1980-01-06 00:00:00.
    pub fn ticks(self) -> i64 {
        self.0
    }

    /// Seconds from `earlier` to this time.
    pub fn seconds_since(self, earlier: Self) -> f64 {
        (self.0 - earlier.0) as f64 / TICKS_PER_SECOND as f64
    }

    /// Year, month, day, hour, minute, and seconds as ticks.
    pub fn fields(self) -> (i64, i64, i64, i64, i64, i64) {
        let days = self.0.div_euclid(TICKS_PER_DAY);
        let in_day = self.0.rem_euclid(TICKS_PER_DAY);
        let (y, m, d) = civil_from_days(days + GPS_EPOCH_DAYS);
        let in_minute = in_day % (60 * TICKS_PER_SECOND);
        let minutes = in_day / (60 * TICKS_PER_SECOND);
        (y, m, d, minutes / 60, minutes % 60, in_minute)
    }
}

impl fmt::Display for GnssTime {
    /// ISO 8601 with 0.1 µs, such as `2025-06-15T00:00:30.0000000`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, mo, d, h, mi, ticks) = self.fields();
        write!(
            f,
            "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{:02}.{:07}",
            ticks / TICKS_PER_SECOND,
            ticks % TICKS_PER_SECOND
        )
    }
}

/// Seconds text, such as `30.0000000` or `5.5`, as ticks. At most seven
/// decimals count; RINEX writes seven.
fn second_ticks(text: &str) -> Option<i64> {
    let text = text.trim();
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || !digits(fraction) || whole.len() > 2 {
        return None;
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let mut ticks = 0;
    let mut scale = TICKS_PER_SECOND / 10;
    for b in fraction.bytes().take(7) {
        ticks += i64::from(b - b'0') * scale;
        scale /= 10;
    }
    Some(whole * TICKS_PER_SECOND + ticks)
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 (proleptic Gregorian), after Howard Hinnant's
/// `days_from_civil`.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The inverse of [`days_from_civil`].
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{GnssTime, TICKS_PER_SECOND, TimeSystem};

    #[test]
    fn times_are_exact_ticks_from_the_gps_epoch() {
        let start = GnssTime::from_fields(1980, 1, 6, 0, 0, " 0.0000000").unwrap();
        assert_eq!(start.ticks(), 0);
        let t = GnssTime::from_fields(2025, 6, 15, 23, 59, "30.1234567").unwrap();
        assert_eq!(t.to_string(), "2025-06-15T23:59:30.1234567");
        let u = GnssTime::from_fields(2025, 6, 16, 0, 0, "0").unwrap();
        assert_eq!(u.ticks() - t.ticks(), 29 * TICKS_PER_SECOND + 8_765_433);
        assert!((u.seconds_since(t) - 29.876_543_3).abs() < 1e-9);
        // Leap years, and the end of a leap second in a UTC file.
        assert!(GnssTime::from_fields(2024, 2, 29, 0, 0, "0").is_some());
        assert!(GnssTime::from_fields(2016, 12, 31, 23, 59, "60.5").is_some());
    }

    #[test]
    fn out_of_range_fields_are_refused() {
        for (y, mo, d, h, mi, s) in [
            (2025, 2, 29, 0, 0, "0"),
            (2025, 13, 1, 0, 0, "0"),
            (2025, 1, 0, 0, 0, "0"),
            (2025, 1, 1, 24, 0, "0"),
            (2025, 1, 1, 0, 60, "0"),
            (2025, 1, 1, 0, 0, "61.0"),
            (2025, 1, 1, 0, 0, "1x"),
            (2025, 1, 1, 0, 0, ""),
            (2025, 1, 1, 0, 0, "-1"),
        ] {
            assert!(
                GnssTime::from_fields(y, mo, d, h, mi, s).is_none(),
                "{y}-{mo}-{d} {h}:{mi}:{s}"
            );
        }
    }

    #[test]
    fn time_systems_round_trip() {
        for code in ["GPS", "GLO", "GAL", "BDT", "QZS", "IRN", "UTC", "TAI"] {
            assert_eq!(TimeSystem::parse(code).unwrap().code(), code);
        }
        assert!(TimeSystem::parse("XYZ").is_none());
    }
}
