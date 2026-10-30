//! Geoscience Australia's GNSS Data Repository: finding CORS RINEX files.
//!
//! Its Web API (`/api/v1/rinexFiles`) lists a station's files of a type and
//! period that overlap a time span, each with a link to download it. Daily
//! files start at 00:00 and hourly ones on the hour, named in RINEX 3's long
//! form, for example `SYDN00AUS_R_20251660000_01D_30S_MO.crx.gz`. A query
//! that matches nothing is answered with 404.

use serde::Deserialize;

use crate::FetchError;
use crate::time::{DAY, Window, format_time, parse_time};

/// Observation or navigation files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    /// Observation files.
    Obs,
    /// Navigation files.
    Nav,
}

impl FileType {
    fn api(self) -> &'static str {
        match self {
            FileType::Obs => "obs",
            FileType::Nav => "nav",
        }
    }
}

/// The span a file covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Period {
    /// An hour, from the hour.
    Hourly,
    /// A day, from 00:00.
    Daily,
}

impl Period {
    fn api(self) -> &'static str {
        match self {
            Period::Hourly => "01H",
            Period::Daily => "01D",
        }
    }

    /// Its length in seconds.
    pub fn seconds(self) -> i64 {
        match self {
            Period::Hourly => 3600,
            Period::Daily => DAY,
        }
    }
}

/// The listing query for a station's files.
pub fn listing_url(
    base: &str,
    station: &str,
    window: Window,
    file: FileType,
    period: Period,
) -> String {
    format!(
        "{base}?stationId={station}&startDate={}&endDate={}&fileType={}&rinexVersion=3&filePeriod={}",
        format_time(window.start),
        // The end is inclusive, so a window ending on the hour asks for the
        // file starting then too; that's harmless, and choose() drops it.
        format_time(window.end),
        file.api(),
        period.api(),
    )
}

/// A file in the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    /// The station, as the archive writes it (four characters, upper case).
    pub station: String,
    /// The file's name, for example `SYDN00AUS_R_20251660000_01D_30S_MO.crx.gz`.
    pub name: String,
    /// Where to download it. GA's links are signed and expire, so they're
    /// used straight away, never stored.
    pub url: String,
    /// Its size, if the listing gives it.
    pub bytes: Option<u64>,
    /// Daily or hourly.
    pub period: Period,
    /// The span it covers.
    pub window: Window,
}

impl RemoteFile {
    /// Whether it's a mixed-constellation navigation file (`_MN`).
    pub fn is_mixed_nav(&self) -> bool {
        self.name.contains("_MN.")
    }
}

/// Reads a listing. An empty listing (the API's 404) is the caller's to
/// turn into `[]`.
pub fn parse_listing(json: &str) -> Result<Vec<RemoteFile>, FetchError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Entry {
        site_id: String,
        file_period: String,
        start_date: String,
        file_location: String,
        file_size: Option<u64>,
    }
    let entries: Vec<Entry> =
        serde_json::from_str(json).map_err(|e| FetchError::Listing(e.to_string()))?;
    let mut files = Vec::new();
    for e in entries {
        let period = match e.file_period.as_str() {
            "01D" => Period::Daily,
            "01H" => Period::Hourly,
            // High-rate (15-minute) files aren't used yet.
            _ => continue,
        };
        let start = parse_time(&e.start_date)
            .ok_or_else(|| FetchError::Listing(format!("bad start date {:?}", e.start_date)))?;
        let path = e.file_location.split('?').next().unwrap_or_default();
        let name = path.rsplit('/').next().unwrap_or_default().to_owned();
        if name.is_empty() || !e.file_location.starts_with("https://") {
            return Err(FetchError::Listing(format!(
                "bad file location {:?}",
                e.file_location
            )));
        }
        files.push(RemoteFile {
            station: e.site_id.to_uppercase(),
            name,
            url: e.file_location,
            bytes: e.file_size,
            period,
            window: Window::new(start, start + period.seconds()),
        });
    }
    files.sort_by(|a, b| (a.window, &a.name).cmp(&(b.window, &b.name)));
    files.dedup_by(|a, b| a.name == b.name);
    Ok(files)
}

/// The observation files chosen to cover a window, and what they don't cover.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Choice {
    /// The files, in order.
    pub files: Vec<RemoteFile>,
    /// Parts of the window no file covers.
    pub missing: Vec<Window>,
}

/// Hours of a day a window must cover before the daily file is taken over
/// hourly ones: below this, hourly files download less.
pub const DAILY_FROM_HOURS: i64 = 12;

/// Chooses observation files to cover a window: for each day it touches,
/// the daily file if the window covers at least [`DAILY_FROM_HOURS`] of it
/// or hourly files are missing, otherwise the hourly files.
pub fn choose(daily: &[RemoteFile], hourly: &[RemoteFile], window: Window) -> Choice {
    let mut choice = Choice::default();
    for day in window.days() {
        let part = Window::new(
            window.start.max(day.start()),
            window.end.min(day.start() + DAY),
        );
        let daily_file = daily.iter().find(|f| f.window.start == day.start());
        let hours: Vec<i64> = (part.start.div_euclid(3600)..(part.end + 3599).div_euclid(3600))
            .map(|h| h * 3600)
            .collect();
        let hourly_files: Vec<&RemoteFile> = hours
            .iter()
            .filter_map(|&h| hourly.iter().find(|f| f.window.start == h))
            .collect();
        let hourly_complete = hourly_files.len() == hours.len();
        let want_daily = (part.end - part.start) >= DAILY_FROM_HOURS * 3600 || !hourly_complete;
        match daily_file {
            Some(f) if want_daily || hourly_files.is_empty() => choice.files.push(f.clone()),
            _ => {
                choice.files.extend(hourly_files.iter().map(|&f| f.clone()));
                for &h in &hours {
                    if !hourly_files.iter().any(|f| f.window.start == h) {
                        let gap = Window::new(h.max(part.start), (h + 3600).min(part.end));
                        match choice.missing.last_mut() {
                            Some(last) if last.end == gap.start => last.end = gap.end,
                            _ => choice.missing.push(gap),
                        }
                    }
                }
            }
        }
    }
    choice
}

/// The day's mixed-constellation navigation file from a listing, if any.
pub fn mixed_nav(files: &[RemoteFile]) -> Option<&RemoteFile> {
    files
        .iter()
        .find(|f| f.is_mixed_nav() && f.period == Period::Daily)
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;
    use crate::time::Day;

    fn listing(station: &str, period: &str, starts: &[&str], kind: &str) -> String {
        let entries: Vec<String> = starts
            .iter()
            .map(|s| {
                let t = parse_time(s).unwrap();
                let day = Day::containing(t);
                let hhmm = format!("{:02}00", (t - day.start()) / 3600);
                format!(
                    r#"{{"siteId":"{}","fileType":"obs","filePeriod":"{period}","startDate":"{s}","rinexVersion":"3","fileLocation":"https://x.example/default/{station}00AUS_R_{}{hhmm}_{period}_30S_{kind}?sig=1","fileSize":100}}"#,
                    station.to_lowercase(),
                    day.yyyyddd()
                )
            })
            .collect();
        format!("[{}]", entries.join(","))
    }

    #[test]
    fn listings_read_with_names_and_spans() {
        let files = parse_listing(&listing(
            "SYDN",
            "01D",
            &["2025-06-15T00:00:00Z"],
            "MO.crx.gz",
        ))
        .unwrap();
        let [f] = &files[..] else { panic!() };
        assert_eq!(f.station, "SYDN");
        assert_eq!(f.name, "SYDN00AUS_R_20251660000_01D_30S_MO.crx.gz");
        assert_eq!(f.window.end - f.window.start, DAY);
        assert!(f.url.ends_with("?sig=1"));
        assert!(parse_listing("{}").is_err());
        assert_eq!(parse_listing("[]").unwrap(), []);
    }

    #[test]
    fn short_windows_take_hourly_files_and_long_ones_daily() {
        let daily = parse_listing(&listing(
            "SYDN",
            "01D",
            &["2025-06-15T00:00:00Z"],
            "MO.crx.gz",
        ))
        .unwrap();
        let hours: Vec<String> = (0..24)
            .map(|h| format!("2025-06-15T{h:02}:00:00Z"))
            .collect();
        let hours: Vec<&str> = hours.iter().map(String::as_str).collect();
        let hourly = parse_listing(&listing("SYDN", "01H", &hours, "MO.crx.gz")).unwrap();
        let t = parse_time("2025-06-15T03:20:00Z").unwrap();
        // Two hours and a bit: hours 3, 4 and 5.
        let c = choose(&daily, &hourly, Window::new(t, t + 2 * 3600 + 60));
        assert_eq!(c.files.len(), 3);
        assert!(c.files.iter().all(|f| f.period == Period::Hourly));
        assert!(c.missing.is_empty());
        // Twelve hours: the daily file.
        let c = choose(&daily, &hourly, Window::new(t, t + 12 * 3600));
        assert_eq!(c.files.len(), 1);
        assert_eq!(c.files[0].period, Period::Daily);
        // An hour missing: the daily file covers it.
        let gappy: Vec<RemoteFile> = hourly
            .iter()
            .filter(|f| f.window.start != t - 1200 + 3600)
            .cloned()
            .collect();
        let c = choose(&daily, &gappy, Window::new(t, t + 2 * 3600));
        assert_eq!(c.files[0].period, Period::Daily);
        // No daily file and an hour missing: say so.
        let c = choose(&[], &gappy, Window::new(t, t + 2 * 3600));
        assert_eq!(c.files.len(), 2);
        assert_eq!(c.missing, [Window::new(t - 1200 + 3600, t - 1200 + 7200)]);
    }

    #[test]
    fn windows_over_midnight_take_a_file_per_day() {
        let daily = parse_listing(&listing(
            "SYDN",
            "01D",
            &["2025-06-15T00:00:00Z", "2025-06-16T00:00:00Z"],
            "MO.crx.gz",
        ))
        .unwrap();
        let t = parse_time("2025-06-15T08:00:00Z").unwrap();
        let c = choose(&daily, &[], Window::new(t, t + 20 * 3600));
        assert_eq!(c.files.len(), 2);
        let c = choose(&daily[..1], &[], Window::new(t, t + 20 * 3600));
        assert_eq!(c.files.len(), 1);
        assert_eq!(c.missing.len(), 1);
    }

    #[test]
    fn mixed_navigation_files_are_preferred() {
        let files = parse_listing(&listing(
            "TOW2",
            "01D",
            &["2025-06-15T00:00:00Z"],
            "MN.rnx.gz",
        ))
        .unwrap();
        assert!(mixed_nav(&files).is_some());
        let files = parse_listing(&listing(
            "TOW2",
            "01D",
            &["2025-06-15T00:00:00Z"],
            "EN.rnx.gz",
        ))
        .unwrap();
        assert!(mixed_nav(&files).is_none());
    }

    #[test]
    fn the_query_names_the_window() {
        let t = parse_time("2025-06-15T03:00:00Z").unwrap();
        let url = listing_url(
            "https://a.example/api",
            "SYDN",
            Window::new(t, t + 3600),
            FileType::Obs,
            Period::Hourly,
        );
        assert_eq!(
            url,
            "https://a.example/api?stationId=SYDN&startDate=2025-06-15T03:00:00Z&endDate=2025-06-15T04:00:00Z&fileType=obs&rinexVersion=3&filePeriod=01H"
        );
    }
}
