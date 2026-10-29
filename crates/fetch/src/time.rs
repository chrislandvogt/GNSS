//! Days and windows, as whole seconds since 1970-01-01 on a calendar with
//! no leap seconds. Planning works in GPS time, as archives name and split
//! their files by GPS days and hours; `crate` callers convert from UTC.

use std::fmt;

/// Seconds in a day.
pub const DAY: i64 = 86_400;

/// A span of time, `start` to `end`, in seconds since 1970-01-01 UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Window {
    /// The first second.
    pub start: i64,
    /// The second after the last.
    pub end: i64,
}

impl Window {
    /// A window; `end` before `start` is taken as empty at `start`.
    pub fn new(start: i64, end: i64) -> Self {
        Self {
            start,
            end: end.max(start),
        }
    }

    /// The window widened by `seconds` at each end.
    pub fn padded(self, seconds: i64) -> Self {
        Self::new(self.start - seconds, self.end + seconds)
    }

    /// Whether the two windows share any time.
    pub fn overlaps(self, other: Window) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// The days the window touches, in order.
    pub fn days(self) -> Vec<Day> {
        let first = Day::containing(self.start);
        // A window ending exactly at midnight doesn't need the next day.
        let last = Day::containing((self.end - 1).max(self.start));
        let mut days = vec![first];
        let mut day = first;
        while day < last {
            day = day.next();
            days.push(day);
        }
        days
    }
}

/// A UTC day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Day {
    /// Days since 1970-01-01.
    days: i64,
}

impl Day {
    /// The day containing a time.
    pub fn containing(seconds: i64) -> Self {
        Self {
            days: seconds.div_euclid(DAY),
        }
    }

    /// The day from a civil date, if it's a real one.
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Option<Self> {
        if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
            return None;
        }
        Some(Self {
            days: days_from_civil(i64::from(year), month, day),
        })
    }

    /// Parses `YYYY-MM-DD`.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('-');
        let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
            return None;
        }
        Self::from_ymd(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?)
    }

    /// Midnight at the start of the day.
    pub fn start(self) -> i64 {
        self.days * DAY
    }

    /// The day as a window.
    pub fn window(self) -> Window {
        Window::new(self.start(), self.start() + DAY)
    }

    /// The day after.
    pub fn next(self) -> Self {
        Self {
            days: self.days + 1,
        }
    }

    /// Days from `self` to `later`.
    pub fn days_until(self, later: Day) -> i64 {
        later.days - self.days
    }

    /// Year, month and day.
    pub fn ymd(self) -> (i32, u32, u32) {
        civil_from_days(self.days)
    }

    /// Its year.
    pub fn year(self) -> i32 {
        self.ymd().0
    }

    /// The day of the year, 1 to 366.
    pub fn doy(self) -> u32 {
        let year = self.year();
        // Days since 1970 are never so far out that this overflows.
        (self.days - days_from_civil(i64::from(year), 1, 1) + 1) as u32
    }

    /// `YYYYDDD`, as in long RINEX and product names.
    pub fn yyyyddd(self) -> String {
        format!("{:04}{:03}", self.year(), self.doy())
    }
}

impl fmt::Display for Day {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = self.ymd();
        write!(f, "{y:04}-{m:02}-{d:02}")
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ`.
pub fn format_time(seconds: i64) -> String {
    let day = Day::containing(seconds);
    let s = seconds - day.start();
    format!("{day}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

/// Parses `YYYY-MM-DDTHH:MM:SS[.fff]Z`, to whole seconds.
pub fn parse_time(text: &str) -> Option<i64> {
    let (date, clock) = text.strip_suffix('Z')?.split_once('T')?;
    let day = Day::parse(date)?;
    let clock = clock.split('.').next()?;
    let mut parts = clock.split(':');
    let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
    let (h, m, s): (i64, i64, i64) = (h.parse().ok()?, m.parse().ok()?, s.parse().ok()?);
    if parts.next().is_some() || h > 23 || m > 59 || s > 60 {
        return None;
    }
    Some(day.start() + h * 3600 + m * 60 + s)
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

// Howard Hinnant's algorithms for the proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y as i32, m, d)
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;

    #[test]
    fn days_round_trip_and_count_the_year() {
        let day = Day::parse("2025-06-15").unwrap();
        assert_eq!(day.doy(), 166);
        assert_eq!(day.yyyyddd(), "2025166");
        assert_eq!(day.to_string(), "2025-06-15");
        assert_eq!(Day::parse("2024-12-31").unwrap().doy(), 366);
        assert_eq!(
            Day::parse("2025-12-31").unwrap().next().to_string(),
            "2026-01-01"
        );
        assert_eq!(Day::parse("2025-02-29"), None);
        assert_eq!(Day::parse("1970-01-01").unwrap().start(), 0);
        for days in -800_000..800_000i64 {
            if days % 997 == 0 {
                let day = Day { days };
                let (y, m, d) = day.ymd();
                assert_eq!(Day::from_ymd(y, m, d), Some(day));
            }
        }
    }

    #[test]
    fn windows_list_the_days_they_touch() {
        let t = parse_time("2025-06-15T22:00:00Z").unwrap();
        let w = Window::new(t, t + 4 * 3600);
        let days: Vec<String> = w.days().iter().map(Day::to_string).collect();
        assert_eq!(days, ["2025-06-15", "2025-06-16"]);
        // Ending at midnight stays on one day.
        let w = Window::new(t, t + 2 * 3600);
        assert_eq!(w.days().len(), 1);
        assert_eq!(format_time(t), "2025-06-15T22:00:00Z");
        assert_eq!(parse_time("2025-06-15T22:00:00.5Z"), Some(t));
    }
}
