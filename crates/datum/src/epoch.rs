//! Coordinate epochs: when a coordinate in a dynamic frame is valid.
//!
//! This module holds the project's one conversion between times and decimal
//! years (CONVENTIONS.md, section 4). Nothing else converts dates to decimal
//! years.

use std::fmt;

use hifitime::{Duration, Epoch, TimeScale};

use crate::error::DatumError;

/// Earliest coordinate epoch accepted.
const MIN_YEAR: f64 = 1900.0;
/// Latest coordinate epoch accepted.
const MAX_YEAR: f64 = 2200.0;

/// The time at which a coordinate in a dynamic frame is valid, as a decimal year.
///
/// # The decimal-year convention
///
/// A decimal year is the calendar year plus the elapsed fraction of that year,
/// counted in GPS time. So 1 January 2018 00:00 GPST is 2018.0, and 2 July
/// 2018 12:00 GPST is 2018.5. Leap years are 366 days long.
///
/// Software differs here. GeodePy counts days of 365.25 from a transformation's
/// reference date, which can differ from this convention by up to three
/// quarters of a day: about 0.15 mm of Australian plate motion. This
/// convention reproduces the GDA2020 Technical Manual's worked example
/// (ITRF2014 at 2018.0) within the manual's 0.1 mm rounding, and GeodePy's
/// convention reproduces it less closely.
///
/// Times in UTC or other scales are converted to GPST first. GPST and UTC
/// differ by 18 seconds (since 2017), which changes a decimal year by 6e-7.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct CoordinateEpoch(f64);

impl CoordinateEpoch {
    /// An epoch from a constant known to be valid. Only for this crate's constants.
    pub(crate) const fn from_const(decimal_year: f64) -> Self {
        Self(decimal_year)
    }

    /// An epoch from a decimal year, such as 2020.0 or 2018.5.
    pub fn from_decimal_year(decimal_year: f64) -> Result<Self, DatumError> {
        if !decimal_year.is_finite() {
            return Err(DatumError::NotFinite {
                what: "decimal year",
                value: decimal_year,
            });
        }
        if !(MIN_YEAR..=MAX_YEAR).contains(&decimal_year) {
            return Err(DatumError::EpochOutOfRange {
                value: decimal_year,
            });
        }
        Ok(Self(decimal_year))
    }

    /// The epoch as a decimal year.
    pub const fn decimal_year(self) -> f64 {
        self.0
    }

    /// The coordinate epoch of a time, in any time scale.
    pub fn from_time(time: Epoch) -> Result<Self, DatumError> {
        let (year, ..) = time.to_gregorian(TimeScale::GPST);
        let start = year_start(year)?;
        let next = year_start(year + 1)?;
        let elapsed_ns = (time - start).total_nanoseconds();
        let year_ns = (next - start).total_nanoseconds();
        Self::from_decimal_year(f64::from(year) + elapsed_ns as f64 / year_ns as f64)
    }

    /// The time this epoch corresponds to, in GPST, to the nearest nanosecond.
    pub fn to_time(self) -> Result<Epoch, DatumError> {
        let year = libm::floor(self.0);
        // The range check on construction keeps this well inside i32.
        let year_int = year as i32;
        let start = year_start(year_int)?;
        let next = year_start(year_int + 1)?;
        let year_ns = (next - start).total_nanoseconds();
        let offset_ns = libm::round((self.0 - year) * year_ns as f64) as i128;
        Ok(start + Duration::from_total_nanoseconds(offset_ns))
    }

    /// Years from `earlier` to this epoch (negative if `earlier` is later).
    pub fn years_since(self, earlier: CoordinateEpoch) -> f64 {
        self.0 - earlier.0
    }
}

impl fmt::Display for CoordinateEpoch {
    /// Formats to four decimal places (under an hour), dropping trailing zeros: `2020.0`, `2018.5`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = format!("{:.4}", self.0);
        let trimmed = text.trim_end_matches('0');
        if trimmed.ends_with('.') {
            write!(f, "{trimmed}0")
        } else {
            f.write_str(trimmed)
        }
    }
}

/// Midnight on 1 January of `year`, GPST.
fn year_start(year: i32) -> Result<Epoch, DatumError> {
    Epoch::maybe_from_gregorian(year, 1, 1, 0, 0, 0, 0, TimeScale::GPST).map_err(|e| {
        DatumError::InvalidTime {
            reason: e.to_string(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn gpst(y: i32, mo: u8, d: u8, h: u8, mi: u8, s: u8) -> Epoch {
        Epoch::from_gregorian(y, mo, d, h, mi, s, 0, TimeScale::GPST)
    }

    #[test]
    fn new_year_is_a_whole_year() {
        let e = CoordinateEpoch::from_time(gpst(2018, 1, 1, 0, 0, 0)).unwrap();
        assert_eq!(e.decimal_year(), 2018.0);
    }

    #[test]
    fn mid_year_in_common_and_leap_years() {
        // 182.5 of 365 days.
        let e = CoordinateEpoch::from_time(gpst(2018, 7, 2, 12, 0, 0)).unwrap();
        assert_eq!(e.decimal_year(), 2018.5);
        // 183 of 366 days.
        let e = CoordinateEpoch::from_time(gpst(2020, 7, 2, 0, 0, 0)).unwrap();
        assert_eq!(e.decimal_year(), 2020.5);
    }

    #[test]
    fn utc_is_converted_to_gpst() {
        // Midnight UTC on 1 January 2017 is 18 s after midnight GPST.
        let utc = Epoch::from_gregorian_utc_at_midnight(2017, 1, 1);
        let e = CoordinateEpoch::from_time(utc).unwrap();
        assert_eq!(e.decimal_year(), 2017.0 + 18.0 / (365.0 * 86_400.0));
    }

    #[test]
    fn gpst_utc_and_tai_offsets_follow_leap_seconds() {
        let utc_2016 = Epoch::from_gregorian_utc_at_midnight(2016, 6, 1);
        let utc_2020 = Epoch::from_gregorian_utc_at_midnight(2020, 6, 1);
        let gpst_minus_utc = |t: Epoch| {
            let (.., h, mi, s, _) = t.to_gregorian(TimeScale::GPST);
            u32::from(h) * 3600 + u32::from(mi) * 60 + u32::from(s)
        };
        assert_eq!(gpst_minus_utc(utc_2016), 17);
        assert_eq!(gpst_minus_utc(utc_2020), 18);
        let tai = utc_2020.to_time_scale(TimeScale::TAI);
        let (.., s, _) = tai.to_gregorian(TimeScale::TAI);
        assert_eq!(s, 37);
    }

    #[test]
    fn round_trips_to_the_nanosecond() {
        for y in [1994.0, 2015.25, 2018.0, 2020.5, 2023.123456789, 2031.999] {
            let e = CoordinateEpoch::from_decimal_year(y).unwrap();
            let back = CoordinateEpoch::from_time(e.to_time().unwrap()).unwrap();
            // A nanosecond is 3e-17 years, below f64 resolution at 2000.
            assert!((back.decimal_year() - y).abs() < 1e-12, "{y} -> {back}");
        }
    }

    #[test]
    fn rejects_nonsense() {
        assert!(CoordinateEpoch::from_decimal_year(f64::NAN).is_err());
        assert!(CoordinateEpoch::from_decimal_year(20180.0).is_err());
        assert!(CoordinateEpoch::from_decimal_year(1066.0).is_err());
    }

    #[test]
    fn displays_compactly() {
        let e = |y| CoordinateEpoch::from_decimal_year(y).unwrap().to_string();
        assert_eq!(e(2020.0), "2020.0");
        assert_eq!(e(2018.5), "2018.5");
        assert_eq!(e(2018.25), "2018.25");
        assert_eq!(e(2023.123456), "2023.1235");
    }
}
