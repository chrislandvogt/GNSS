//! Checking an observation file: its header, its equipment against the
//! antenna catalogue, and its quality metrics (`gnss_rinex_io::qc`),
//! with elevation coverage if there are navigation files.

use std::io::BufRead;

use gnss_antennas::Catalogue;
pub use gnss_rinex_io::orbit::{Ephemeris, Orbits};
use gnss_rinex_io::qc::{Qc, QcOptions};
use gnss_rinex_io::{RinexError, read_obs};
use serde::Serialize;

/// Reads a navigation file's broadcast ephemerides (RINEX 2, 3 or 4).
/// Those of several files go together into [`Orbits::new`].
pub fn read_nav<R: BufRead>(reader: R) -> Result<Vec<Ephemeris>, RinexError> {
    Ok(gnss_rinex_io::read_nav(reader)?.ephemerides)
}

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
    /// Why there's no elevation coverage although there are orbits.
    pub coverage_note: Option<String>,
    /// By system.
    pub systems: Vec<SystemQc>,
}

/// One signal's metrics.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SignalQc {
    /// Its observation code, such as `L1C`.
    pub code: String,
    /// Its observations.
    pub observations: usize,
    /// Over the satellite-epochs its system was tracked, 0 to 1.
    pub completeness: f64,
    /// For a signal strength, its mean.
    pub mean_strength: Option<f64>,
    /// Given orbits: its observations above the elevation cutoff over the
    /// satellite-epochs expected there, 0 to 1.
    pub coverage: Option<f64>,
}

/// Elevation coverage: how much of what was in the sky was tracked.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Coverage {
    /// The elevation cutoff, degrees.
    pub cutoff: f64,
    /// Healthy satellite-epochs above the cutoff, by the broadcast orbits,
    /// at every expected epoch.
    pub expected: usize,
    /// Of those, the ones with observations.
    pub tracked: usize,
    /// `tracked` over `expected`, 0 to 1.
    pub ratio: Option<f64>,
    /// Satellite-epochs tracked below the cutoff.
    pub below_cutoff: usize,
    /// Slips found above the cutoff.
    pub slips_above_cutoff: usize,
    /// Satellite-epochs tracked that the ephemeris says are unhealthy.
    pub unhealthy: usize,
    /// Satellite-epochs tracked with no ephemeris for the time.
    pub no_ephemeris: usize,
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
    /// Epochs that jumped on the pair without the next confirming a slip.
    pub outliers: usize,
    /// Multipath RMS by band, metres, such as `MP1`.
    pub multipath: Vec<(String, f64)>,
    /// Elevation coverage, given orbits for the system.
    pub coverage: Option<Coverage>,
    /// Each signal.
    pub signals: Vec<SignalQc>,
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
/// time, and reports on it. With orbits, from the file's navigation files,
/// elevation coverage too, seen from the header's approximate position.
pub fn check_file<R: BufRead>(reader: R, orbits: Option<Orbits>) -> Result<FileQc, RinexError> {
    let mut reader = read_obs(reader)?;
    let mut qc = Qc::new(reader.header(), QcOptions::default());
    let position = reader
        .header()
        .approx_position
        .filter(|p| p.iter().map(|x| x * x).sum::<f64>() > 1e12);
    let coverage_note = match (orbits, position) {
        (Some(orbits), Some(position)) => {
            qc = qc.with_orbits(orbits, position);
            None
        }
        (Some(_), None) => Some(
            "the header has no approximate position, so there's no elevation coverage".to_owned(),
        ),
        (None, _) => None,
    };
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
        coverage_note,
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
                outliers: s.outliers,
                multipath: s
                    .multipath
                    .iter()
                    .map(|(band, m)| (format!("MP{band}"), m.rms))
                    .collect(),
                coverage: s.coverage.map(|c| Coverage {
                    cutoff: c.cutoff,
                    expected: c.expected,
                    tracked: c.tracked,
                    ratio: c.ratio(),
                    below_cutoff: c.below_cutoff,
                    slips_above_cutoff: c.slips_above_cutoff,
                    unhealthy: c.unhealthy,
                    no_ephemeris: c.no_ephemeris,
                }),
                signals: s
                    .signals
                    .iter()
                    .map(|(code, x)| SignalQc {
                        code: code.clone(),
                        observations: x.observations,
                        completeness: x.completeness,
                        mean_strength: x.strength.map(|v| v.mean),
                        coverage: x.coverage,
                    })
                    .collect(),
            })
            .collect(),
    })
}
