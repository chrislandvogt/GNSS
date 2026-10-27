//! Checking an observation file: its header, its equipment against the
//! antenna catalogue, and its quality metrics (`gnss_rinex_io::qc`).

use std::io::BufRead;

use gnss_antennas::Catalogue;
use gnss_rinex_io::qc::{Qc, QcOptions};
use gnss_rinex_io::{RinexError, read_obs};
use serde::Serialize;

/// What checking an observation file found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileQc {
    /// The mark's name in the header.
    pub marker: String,
    /// The RINEX version, such as `3.04`.
    pub version: String,
    /// The receiver type.
    pub receiver: String,
    /// The antenna's IGS code and radome, as the header gives them.
    pub antenna: String,
    /// See `antenna`.
    pub radome: Option<String>,
    /// What the antenna catalogue says about the antenna, if there's a problem
    /// or it has no IGS calibration.
    pub antenna_note: Option<String>,
    /// The antenna height and offsets in the header (H/E/N), metres.
    pub antenna_delta: Option<[f64; 3]>,
    /// The epochs' time scale, such as `GPS`.
    pub time_system: String,
    /// The first and last epochs, ISO 8601 on that time scale.
    pub first: Option<String>,
    /// See `first`.
    pub last: Option<String>,
    /// The interval in seconds: the most common spacing between epochs.
    pub interval: Option<f64>,
    /// Epochs with observations, and how many there would be without gaps.
    pub epochs: usize,
    /// See `epochs`.
    pub expected_epochs: Option<usize>,
    /// Gaps between epochs: from, to, and seconds.
    pub gaps: Vec<(String, String, f64)>,
    /// Events in the file (epoch flags 2 to 5).
    pub events: usize,
    /// By system.
    pub systems: Vec<SystemQc>,
}

/// One system's metrics.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SystemQc {
    /// Its name, such as `GPS`.
    pub system: String,
    /// Satellites seen.
    pub satellites: usize,
    /// Satellite-epochs.
    pub satellite_epochs: usize,
    /// The bands used for slips and multipath, such as `L1/L2`.
    pub pair: Option<String>,
    /// Slips found from the observations, and losses of lock the receiver
    /// flagged.
    pub slips: usize,
    /// See `slips`.
    pub flagged: usize,
    /// Multipath RMS by band, metres, such as `MP1`.
    pub multipath: Vec<(String, f64)>,
    /// Each signal: code, observations, completeness (0 to 1), and for a
    /// signal strength its mean.
    pub signals: Vec<(String, usize, f64, Option<f64>)>,
}

fn system_name(letter: char) -> String {
    match letter {
        'G' => "GPS",
        'R' => "GLONASS",
        'E' => "Galileo",
        'C' => "BeiDou",
        'J' => "QZSS",
        'I' => "NavIC",
        'S' => "SBAS",
        _ => "other",
    }
    .to_owned()
}

/// Reads an observation file, plain or Hatanaka-compressed, a line at a
/// time, and reports on it.
pub fn check_file<R: BufRead>(reader: R) -> Result<FileQc, RinexError> {
    let mut reader = read_obs(reader)?;
    let mut qc = Qc::new(reader.header(), QcOptions::default());
    while let Some(epoch) = reader.next_epoch()? {
        if !epoch.records.is_empty() {
            qc.replan(reader.header());
        }
        qc.add(&epoch);
    }
    let report = qc.finish();
    let h = reader.header();
    let catalogue = Catalogue::igs();
    let radome = h.antenna.radome.clone();
    let antenna_note = if h.antenna.kind.is_empty() {
        Some("the header doesn't name the antenna".to_owned())
    } else {
        let r = radome.as_deref().unwrap_or("NONE");
        catalogue.check(&h.antenna.kind, r).or_else(|| {
            (!catalogue.is_calibrated(&h.antenna.kind, r)).then(|| {
                format!(
                    "the IGS has no calibration for {} with radome {r}",
                    h.antenna.kind
                )
            })
        })
    };
    Ok(FileQc {
        marker: h.marker_name.clone(),
        version: format!("{}.{:02}", h.version.major, h.version.minor),
        receiver: h.receiver.kind.clone(),
        antenna: h.antenna.kind.clone(),
        radome,
        antenna_note,
        antenna_delta: h.antenna_delta,
        time_system: h.time_system.code().to_owned(),
        first: report.first.map(|t| t.to_string()),
        last: report.last.map(|t| t.to_string()),
        interval: report.interval,
        epochs: report.epochs,
        expected_epochs: report.expected_epochs,
        gaps: report
            .gaps
            .iter()
            .map(|g| (g.after.to_string(), g.before.to_string(), g.seconds()))
            .collect(),
        events: report.events,
        systems: report
            .systems
            .iter()
            .map(|(&letter, s)| SystemQc {
                system: system_name(letter),
                satellites: s.satellites,
                satellite_epochs: s.satellite_epochs,
                pair: s.pair.map(|(a, b)| format!("L{a}/L{b}")),
                slips: s.slips,
                flagged: s.flagged,
                multipath: s
                    .multipath
                    .iter()
                    .map(|(band, m)| (format!("MP{band}"), m.rms))
                    .collect(),
                signals: s
                    .signals
                    .iter()
                    .map(|(code, x)| {
                        (
                            code.clone(),
                            x.observations,
                            x.completeness,
                            x.strength.map(|v| v.mean),
                        )
                    })
                    .collect(),
            })
            .collect(),
    })
}
