//! CORS site logs: what equipment a station had, and when, from Geoscience
//! Australia's site log service (`gws.geodesy.ga.gov.au/api/siteLogs`, the
//! IGS site log as JSON). Used to check a CORS file's RINEX header against
//! what the station was known to have, since a header can be wrong and the
//! antenna it names decides the antenna model applied in processing.

use serde::Deserialize;

use crate::FetchError;
use crate::time::{Window, parse_time};

/// A receiver, as installed.
#[derive(Debug, Clone, PartialEq)]
pub struct LoggedReceiver {
    /// Its type, such as `SEPT POLARX5`.
    pub kind: String,
    /// Its serial number.
    pub serial: String,
    /// Its firmware version.
    pub firmware: String,
    /// When it was installed and removed (still there if open).
    pub installed: Window,
}

/// An antenna, as installed.
#[derive(Debug, Clone, PartialEq)]
pub struct LoggedAntenna {
    /// Its IGS type, such as `LEIAR25.R3`, without the radome.
    pub kind: String,
    /// Its radome, such as `LEIT` or `NONE`.
    pub radome: String,
    /// Its serial number.
    pub serial: String,
    /// The antenna reference point above the marker, and its north and east
    /// offsets, metres.
    pub up: f64,
    /// See `up`.
    pub north: f64,
    /// See `up`.
    pub east: f64,
    /// When it was installed and removed (still there if open).
    pub installed: Window,
}

/// A station's equipment history.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SiteLog {
    /// Receivers, in order of installation.
    pub receivers: Vec<LoggedReceiver>,
    /// Antennas, in order of installation.
    pub antennas: Vec<LoggedAntenna>,
}

/// Far in the future: the end of equipment still installed.
const OPEN: i64 = i64::MAX / 4;

impl SiteLog {
    /// The receivers installed at any time in a window.
    pub fn receivers_in(&self, window: Window) -> Vec<&LoggedReceiver> {
        self.receivers
            .iter()
            .filter(|r| r.installed.overlaps(window))
            .collect()
    }

    /// The antennas installed at any time in a window.
    pub fn antennas_in(&self, window: Window) -> Vec<&LoggedAntenna> {
        self.antennas
            .iter()
            .filter(|a| a.installed.overlaps(window))
            .collect()
    }
}

/// The site log query for a station.
pub fn site_log_url(base: &str, station: &str) -> String {
    format!("{base}?id={station}&format=json")
}

/// Reads GA's JSON site log. Deleted entries are left out.
pub fn parse_site_log(json: &str) -> Result<SiteLog, FetchError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Log {
        // Required: a log without them isn't one.
        gnss_receivers: Vec<Receiver>,
        gnss_antennas: Vec<Antenna>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Receiver {
        #[serde(rename = "type")]
        kind: Option<String>,
        serial_number: Option<String>,
        firmware_version: Option<String>,
        date_installed: Option<String>,
        date_removed: Option<String>,
        #[serde(default)]
        deleted: bool,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Antenna {
        #[serde(rename = "type")]
        kind: Option<String>,
        serial_number: Option<String>,
        marker_arp_up_ecc: Option<f64>,
        marker_arp_north_ecc: Option<f64>,
        marker_arp_east_ecc: Option<f64>,
        antenna_radome_type: Option<String>,
        date_installed: Option<String>,
        date_removed: Option<String>,
        #[serde(default)]
        deleted: bool,
    }
    let log: Log = serde_json::from_str(json).map_err(|e| FetchError::Listing(e.to_string()))?;
    let span = |from: &Option<String>, to: &Option<String>| -> Option<Window> {
        let start = parse_time(from.as_deref()?)?;
        let end = to.as_deref().and_then(parse_time).unwrap_or(OPEN);
        Some(Window::new(start, end))
    };
    let text = |s: Option<String>| s.unwrap_or_default().trim().to_owned();
    let mut receivers: Vec<LoggedReceiver> = log
        .gnss_receivers
        .into_iter()
        .filter(|r| !r.deleted)
        .filter_map(|r| {
            Some(LoggedReceiver {
                installed: span(&r.date_installed, &r.date_removed)?,
                kind: text(r.kind),
                serial: text(r.serial_number),
                firmware: text(r.firmware_version),
            })
        })
        .collect();
    let mut antennas: Vec<LoggedAntenna> = log
        .gnss_antennas
        .into_iter()
        .filter(|a| !a.deleted)
        .filter_map(|a| {
            // The type is the IGS's 20 characters: 16 of antenna, 4 of radome.
            let full = a.kind.clone().unwrap_or_default();
            let kind = full.get(..16).unwrap_or(&full).trim().to_owned();
            let radome = a
                .antenna_radome_type
                .clone()
                .filter(|r| !r.trim().is_empty())
                .or_else(|| full.get(16..).map(str::to_owned))
                .unwrap_or_default()
                .trim()
                .to_owned();
            Some(LoggedAntenna {
                installed: span(&a.date_installed, &a.date_removed)?,
                kind,
                radome: if radome.is_empty() {
                    "NONE".into()
                } else {
                    radome
                },
                serial: text(a.serial_number),
                up: a.marker_arp_up_ecc.unwrap_or(0.0),
                north: a.marker_arp_north_ecc.unwrap_or(0.0),
                east: a.marker_arp_east_ecc.unwrap_or(0.0),
            })
        })
        .collect();
    receivers.sort_by_key(|r| r.installed);
    antennas.sort_by_key(|a| a.installed);
    Ok(SiteLog {
        receivers,
        antennas,
    })
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;

    /// Trimmed from GA's site log of SYDN (2026-10-30), out of order as GA
    /// gives it, with a deleted entry.
    const SYDN: &str = r#"{
      "gnssReceivers": [
        {"type": "SEPT POLARX4TR", "serialNumber": "3007592", "firmwareVersion": "2.9.6",
         "dateInstalled": "2017-01-05T00:00:00Z", "dateRemoved": "2021-05-08T00:00:00Z", "deleted": false},
        {"type": "SEPT POLARX5TR", "serialNumber": "3057890", "firmwareVersion": "5.5.0",
         "dateInstalled": "2021-05-08T00:00:00Z", "dateRemoved": null, "deleted": false},
        {"type": "WRONG", "serialNumber": "1", "dateInstalled": "2020-01-01T00:00:00Z", "deleted": true}
      ],
      "gnssAntennas": [
        {"type": "ASH701945C_M    NONE", "serialNumber": "CR519994908", "markerArpUpEcc": 0.03,
         "markerArpNorthEcc": 0.0, "markerArpEastEcc": 0.0, "antennaRadomeType": "NONE",
         "dateInstalled": "2004-05-05T00:00:00Z", "dateRemoved": "2026-03-31T04:00:00Z", "deleted": false},
        {"type": "LEIAR25.R3      LEIT", "serialNumber": "726123", "markerArpUpEcc": 0.0,
         "markerArpNorthEcc": 0.0, "markerArpEastEcc": 0.0, "antennaRadomeType": null,
         "dateInstalled": "2026-03-31T05:00:00Z", "dateRemoved": null, "deleted": false}
      ]
    }"#;

    fn t(s: &str) -> i64 {
        parse_time(s).unwrap()
    }

    #[test]
    fn equipment_is_found_by_when_it_was_installed() {
        let log = parse_site_log(SYDN).unwrap();
        assert_eq!(log.receivers.len(), 2);
        let day = Window::new(t("2025-06-15T00:00:00Z"), t("2025-06-16T00:00:00Z"));
        let r = log.receivers_in(day);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].kind, "SEPT POLARX5TR");
        let a = log.antennas_in(day);
        assert_eq!(
            (a[0].kind.as_str(), a[0].radome.as_str(), a[0].up),
            ("ASH701945C_M", "NONE", 0.03)
        );
        // The radome from the type when it isn't given on its own.
        let later = Window::new(t("2026-06-01T00:00:00Z"), t("2026-06-02T00:00:00Z"));
        assert_eq!(log.antennas_in(later)[0].radome, "LEIT");
        // A window over a change sees both.
        let change = Window::new(t("2026-03-31T00:00:00Z"), t("2026-04-01T00:00:00Z"));
        assert_eq!(log.antennas_in(change).len(), 2);
        assert_eq!(
            site_log_url(
                "https://gws.example/api/siteLogs/search/findByFourCharacterId",
                "SYDN"
            ),
            "https://gws.example/api/siteLogs/search/findByFourCharacterId?id=SYDN&format=json"
        );
        assert!(parse_site_log("[]").is_err());
        assert!(parse_site_log("{}").is_err());
    }
}
