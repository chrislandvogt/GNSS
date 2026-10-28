//! Importing RINEX observation files into a project: each file goes into
//! `raw/` (with its hash in `raw.lock`), the occupations found in them
//! (`gnss_rinex_io::occupation`) into `occupations/`, and marks not yet in
//! `marks.toml` into it.
//!
//! Everything is worked out and checked before anything is written, so an
//! import that fails leaves the project as it was.

use std::io::BufReader;

use gnss_datum::GRS80;
use gnss_model::{
    Antenna, HeightMeasurement, HeightMethod, Mark, MarkId, Occupation, OccupationId, Receiver,
    SCHEMA, Timestamp,
};
use gnss_rinex_io::occupation::{Setup, merge, segments};
use gnss_rinex_io::{GnssTime, RinexError, TimeSystem};
use gnss_store::{ProjectPath, Store, StoreError};
use nalgebra::Vector3;
use serde::Serialize;

use crate::folder::{LoadError, load};

/// How files are turned into occupations.
#[derive(Debug, Clone, PartialEq)]
pub struct IngestOptions {
    /// The mark every file was on, instead of their headers' marker names.
    pub mark: Option<String>,
    /// Files of one set-up that follow on within this many seconds are one
    /// occupation.
    pub join_s: f64,
    /// A gap this long within an occupation, in seconds, gets a note: the
    /// antenna may have been set up again.
    pub long_gap_s: f64,
}

impl Default for IngestOptions {
    fn default() -> Self {
        Self {
            mark: None,
            join_s: 300.0,
            long_gap_s: 900.0,
        }
    }
}

/// One occupation imported.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Imported {
    /// Its id, which is its file's name in `occupations/`.
    pub id: String,
    /// The mark.
    pub mark: String,
    /// When it started and ended, UTC.
    pub start: String,
    /// See `start`.
    pub end: String,
    /// Its RINEX files, in `raw/`.
    pub files: Vec<String>,
    /// Epochs with observations.
    pub epochs: usize,
    /// The interval, seconds.
    pub interval: Option<f64>,
    /// Things to check, such as a long gap.
    pub notes: Vec<String>,
}

/// What an import did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IngestReport {
    /// The occupations, in time order.
    pub occupations: Vec<Imported>,
    /// Marks added to `marks.toml`, with their positions from the headers.
    pub marks_added: Vec<String>,
    /// Things to check about the files, such as kinematic parts, which
    /// aren't occupations.
    pub notes: Vec<String>,
}

/// Why nothing was imported.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum IngestError {
    /// The project couldn't be loaded.
    #[error(transparent)]
    Load(#[from] LoadError),
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A file didn't read as RINEX.
    #[error("{file}: {error}")]
    Rinex {
        /// The file.
        file: String,
        /// Why.
        error: RinexError,
    },
    /// Something in the files can't become an occupation as it is.
    #[error("{0}")]
    Invalid(String),
}

/// A GNSS time on a scale as a UTC timestamp.
fn timestamp(t: GnssTime, scale: TimeSystem) -> Result<Timestamp, IngestError> {
    let text = format!("{}Z", scale.to_utc(t));
    Timestamp::parse(&text).map_err(|e| IngestError::Invalid(e.to_string()))
}

/// The receiver's make and model from its RINEX type, such as
/// `LEICA GR30`.
fn receiver(setup: &Setup, notes: &mut Vec<String>) -> Receiver {
    let kind = setup.receiver.kind.trim();
    let (make, model) = match kind.split_once(' ') {
        Some((make, model)) => (make.to_owned(), model.trim().to_owned()),
        None if kind.is_empty() => {
            notes.push("the header doesn't name the receiver".into());
            ("UNKNOWN".to_owned(), "UNKNOWN".to_owned())
        }
        None => (kind.to_owned(), kind.to_owned()),
    };
    let optional = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_owned());
    Receiver {
        make,
        model,
        serial: optional(&setup.receiver.number),
        firmware: optional(&setup.receiver.version),
    }
}

/// Imports RINEX observation files, given as (file name, contents), into
/// the project in `store`.
pub fn import_rinex<S: Store + ?Sized>(
    store: &mut S,
    files: &[(String, Vec<u8>)],
    options: &IngestOptions,
) -> Result<IngestReport, IngestError> {
    let loaded = load(store)?;
    let mut marks = loaded.marks;
    let mut taken: Vec<String> = loaded
        .occupations
        .iter()
        .map(|o| o.id.as_str().to_owned())
        .collect();
    let mut notes = Vec::new();

    // Each file into raw/, and its segments.
    let mut raw = Vec::new();
    let mut found = Vec::new();
    for (name, bytes) in files {
        let path = ProjectPath::new(format!("raw/{name}"))?;
        if store.read(&path)?.is_some() || raw.iter().any(|(p, _): &(ProjectPath, _)| p == &path) {
            return Err(StoreError::RawExists(path.to_string()).into());
        }
        let segs = segments(BufReader::new(&bytes[..])).map_err(|error| IngestError::Rinex {
            file: name.clone(),
            error,
        })?;
        for s in segs.iter().filter(|s| s.kinematic) {
            notes.push(format!(
                "{name}: the antenna moved from {} to {} (epoch flag 2); that part isn't an occupation",
                s.first, s.last
            ));
        }
        if segs.is_empty() {
            notes.push(format!("{name}: no epochs with observations"));
        }
        found.push(segs);
        raw.push((path, bytes));
    }

    let mut occupations = Vec::new();
    let mut imported = Vec::new();
    let mut marks_added = Vec::new();
    for o in merge(&found, options.join_s) {
        let mut notes = Vec::new();
        let mut files: Vec<String> = Vec::new();
        for (i, _) in &o.parts {
            let file = raw.get(*i).map(|(p, _)| p.to_string()).unwrap_or_default();
            if !files.contains(&file) {
                files.push(file);
            }
        }
        let marker = options
            .mark
            .clone()
            .unwrap_or_else(|| o.setup.marker.trim().to_owned());
        let mark = MarkId::new(marker.clone()).map_err(|e| {
            IngestError::Invalid(format!(
                "{}: the marker name {marker:?} can't be a mark id ({e}); give the mark with --mark",
                files.join(", ")
            ))
        })?;
        if o.setup.antenna.kind.trim().is_empty() {
            return Err(IngestError::Invalid(format!(
                "{}: the header doesn't name the antenna, so the occupation can't be checked or processed; correct the header first",
                files.join(", ")
            )));
        }
        let height = match o.setup.antenna_delta {
            Some([h, e, n]) => {
                if e != 0.0 || n != 0.0 {
                    notes.push(format!(
                        "the header has the antenna offset {e} m east and {n} m north of the mark; only the height is kept"
                    ));
                }
                h
            }
            None => {
                notes.push(
                    "the header has no ANTENNA: DELTA H/E/N, so the height is 0 until it's corrected"
                        .into(),
                );
                0.0
            }
        };
        for g in &o.gaps {
            if g.seconds() >= options.long_gap_s {
                notes.push(format!(
                    "a gap of {:.0} min, {} to {}: if the antenna was set up again, it's two occupations",
                    g.seconds() / 60.0,
                    g.after,
                    g.before
                ));
            }
        }
        if o.overlap_s > 0.0 {
            notes.push(format!(
                "its files overlap by {:.0} s: the same data twice?",
                o.overlap_s
            ));
        }
        if o.last <= o.first {
            return Err(IngestError::Invalid(format!(
                "{}: {} has a single epoch ({}); an occupation needs a span",
                files.join(", "),
                mark,
                o.first
            )));
        }
        let start = timestamp(o.first, o.time_system)?;
        let end = timestamp(o.last, o.time_system)?;
        // An id from the date (as the file has it: a daily file starting
        // at 00:00 GPS time starts the day before in UTC), the mark and a
        // letter.
        let date = o.first.to_string().get(..10).unwrap_or_default().to_owned();
        let id = (0..)
            .map(|n: u32| {
                let suffix = if n < 26 {
                    char::from(b'A' + n as u8).to_string()
                } else {
                    (n + 1).to_string()
                };
                format!("{date}-{mark}-{suffix}")
            })
            .find(|id| !taken.contains(id))
            .unwrap_or_default();
        taken.push(id.clone());
        let occupation = Occupation {
            schema: SCHEMA,
            id: OccupationId::new(id.clone()).map_err(|e| IngestError::Invalid(e.to_string()))?,
            mark: mark.clone(),
            start,
            end,
            rinex: files.clone(),
            field_log: None,
            receiver: receiver(&o.setup, &mut notes),
            antenna: Antenna {
                igs: o.setup.antenna.kind.trim().to_owned(),
                radome: o
                    .setup
                    .antenna
                    .radome
                    .clone()
                    .unwrap_or_else(|| "NONE".to_owned()),
                serial: (!o.setup.antenna.number.trim().is_empty())
                    .then(|| o.setup.antenna.number.trim().to_owned()),
                calibration: None,
            },
            height: HeightMeasurement {
                method: HeightMethod::Vertical,
                to: "ARP".into(),
                value: height,
                measured_by: None,
                photos: Vec::new(),
            },
        };
        occupation
            .check()
            .map_err(|e| IngestError::Invalid(format!("{}: {e}", files.join(", "))))?;
        if marks.get(&mark).is_none() {
            let mut new = Mark::new(mark.clone());
            new.approx = o
                .setup
                .approx_position
                .filter(|p| p.iter().map(|x| x * x).sum::<f64>() > 1e12)
                .and_then(|[x, y, z]| GRS80.ecef_to_geodetic(&Vector3::new(x, y, z)).ok())
                .map(|g| {
                    [
                        g.latitude_deg(),
                        g.longitude_deg(),
                        g.ellipsoidal_height_m(),
                    ]
                });
            marks.marks.push(new);
            marks_added.push(mark.to_string());
        }
        imported.push(Imported {
            id,
            mark: mark.to_string(),
            start: occupation.start.to_string(),
            end: occupation.end.to_string(),
            files,
            epochs: o.epochs,
            interval: o.interval,
            notes,
        });
        occupations.push(occupation);
    }
    // Written only now that everything's checked.
    for (path, bytes) in &raw {
        store.add_raw(path, bytes)?;
    }
    if !marks_added.is_empty() {
        store.write(&ProjectPath::new("marks.toml")?, marks.to_toml().as_bytes())?;
    }
    for o in &occupations {
        store.write(
            &ProjectPath::new(format!("occupations/{}.toml", o.id))?,
            o.to_toml().as_bytes(),
        )?;
    }
    Ok(IngestReport {
        occupations: imported,
        marks_added,
        notes,
    })
}
