//! Checking RINEX headers against the occupations: the occupation file is
//! the field record (the log, the tape photo), so where a header says
//! otherwise, the header is wrong. A header is read as corrected by the
//! project's overlays, in order of their ids.
//!
//! What's compared, for each of an occupation's files:
//! - the marker name and the mark (a RINEX 3 long name, such as
//!   `SWTC00AUS`, matches the mark `SWTC`);
//! - the antenna and radome codes, and serial numbers where both give one;
//! - the receiver type (make and model) and serial number;
//! - the antenna height: the header's ARP height against the occupation's,
//!   vertical to the ARP, or converted from a slant height with the antenna
//!   geometry catalogue;
//! - the span: the occupation should lie within the files' first and last
//!   epochs, where the headers give them.
//!
//! Differences in the marker name, codes, receiver type and height can be
//! fixed with an overlay ([`fix`]); serial numbers and spans need a person.

use gnss_antennas::{Catalogue, vertical_from_slant};
pub use gnss_model::Timestamp;
use gnss_model::{HeightMethod, Occupation, Overlay, OverlayId, RinexHeader, SCHEMA};
use gnss_rinex_io::{HeaderPatch, ObsHeader, RinexError, read_obs, rewrite_obs};
use gnss_store::{ProjectPath, Store, StoreError};
use serde::Serialize;

use crate::folder::{LoadError, Loaded, load};

/// One way a header and its occupation disagree.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    /// The occupation.
    pub occupation: String,
    /// The RINEX file, in `raw/`.
    pub file: String,
    /// What's compared, such as `antenna`.
    pub what: String,
    /// What the header says (as corrected by overlays).
    pub header: String,
    /// What the occupation says.
    pub occupation_value: String,
    /// Whether an overlay can make the header agree.
    pub fixable: bool,
}

/// Why the headers couldn't be checked.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ReconcileError {
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
    /// An overlay couldn't be made.
    #[error("{0}")]
    Invalid(String),
}

/// The overlays for a file, in order of their ids, as one patch.
pub(crate) fn patch_for(overlays: &[Overlay], file: &str) -> HeaderPatch {
    let mut patch = HeaderPatch::default();
    for o in overlays.iter().filter(|o| o.file == file) {
        if let Some(r) = &o.rinex {
            patch.then(&HeaderPatch {
                marker_name: r.marker_name.clone(),
                receiver: r.receiver.clone(),
                antenna: r.antenna.clone(),
                radome: r.radome.clone(),
                antenna_delta: r.antenna_delta,
            });
        }
    }
    patch
}

/// A file's header, as corrected by its overlays.
fn header<S: Store + ?Sized>(
    store: &S,
    overlays: &[Overlay],
    file: &str,
) -> Result<ObsHeader, ReconcileError> {
    let bytes = store.read_raw(&ProjectPath::new(file)?)?;
    let mut h = read_obs(&bytes[..])
        .map_err(|error| ReconcileError::Rinex {
            file: file.to_owned(),
            error,
        })?
        .header()
        .clone();
    h.patch(&patch_for(overlays, file));
    Ok(h)
}

/// Upper case, with runs of spaces as one: how receiver types compare.
fn squash(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

/// The occupation's antenna height, vertical to the ARP, if it can be
/// worked out.
fn arp_height(o: &Occupation, catalogue: &Catalogue) -> Result<f64, String> {
    match o.height.method {
        HeightMethod::Vertical if o.height.to == "ARP" => Ok(o.height.value),
        HeightMethod::Vertical => Err(format!(
            "measured vertically to {}, which only the ARP compares with",
            o.height.to
        )),
        HeightMethod::Slant => {
            vertical_from_slant(catalogue, &o.antenna.igs, &o.height.to, o.height.value)
                .map_err(|e| e.to_string())
        }
    }
}

fn utc(t: gnss_rinex_io::GnssTime, h: &ObsHeader) -> Option<Timestamp> {
    Timestamp::parse(&format!("{}Z", h.time_system.to_utc(t))).ok()
}

/// Compares every occupation's files with it.
pub fn reconcile<S: Store + ?Sized>(store: &S) -> Result<Vec<Finding>, ReconcileError> {
    let loaded = load(store)?;
    compare(store, &loaded)
}

fn compare<S: Store + ?Sized>(store: &S, loaded: &Loaded) -> Result<Vec<Finding>, ReconcileError> {
    let catalogue = Catalogue::igs();
    let mut findings = Vec::new();
    for o in &loaded.occupations {
        let mut span: Option<(Timestamp, Timestamp)> = None;
        let mut span_known = true;
        for file in &o.rinex {
            let h = header(store, &loaded.overlays, file)?;
            let mut add = |what: &str, header: String, occupation_value: String, fixable| {
                findings.push(Finding {
                    occupation: o.id.to_string(),
                    file: file.clone(),
                    what: what.to_owned(),
                    header,
                    occupation_value,
                    fixable,
                });
            };
            let marker = h.marker_name.trim().to_uppercase();
            let mark = o.mark.as_str().to_uppercase();
            let long_name = marker.len() == 9 && marker.starts_with(&mark) && mark.len() == 4;
            if marker != mark && !long_name {
                add(
                    "marker name",
                    h.marker_name.clone(),
                    o.mark.to_string(),
                    true,
                );
            }
            if h.antenna.kind != o.antenna.igs {
                add(
                    "antenna",
                    h.antenna.kind.clone(),
                    o.antenna.igs.clone(),
                    true,
                );
            }
            let radome = h.antenna.radome.clone().unwrap_or_else(|| "NONE".into());
            if radome != o.antenna.radome {
                add("radome", radome, o.antenna.radome.clone(), true);
            }
            if let Some(serial) = &o.antenna.serial
                && !h.antenna.number.trim().is_empty()
                && h.antenna.number.trim() != serial
            {
                add(
                    "antenna serial",
                    h.antenna.number.clone(),
                    serial.clone(),
                    false,
                );
            }
            let receiver = format!("{} {}", o.receiver.make, o.receiver.model);
            if squash(&h.receiver.kind) != squash(&receiver)
                && squash(&h.receiver.kind) != squash(&o.receiver.model)
            {
                add("receiver", h.receiver.kind.clone(), receiver, true);
            }
            if let Some(serial) = &o.receiver.serial
                && !h.receiver.number.trim().is_empty()
                && h.receiver.number.trim() != serial
            {
                add(
                    "receiver serial",
                    h.receiver.number.clone(),
                    serial.clone(),
                    false,
                );
            }
            let header_height = h
                .antenna_delta
                .map_or_else(|| "none".to_owned(), |d| format!("{:.4} m", d[0]));
            match arp_height(o, catalogue) {
                Ok(arp) => {
                    if h.antenna_delta.is_none_or(|d| (d[0] - arp).abs() > 0.0005) {
                        add("antenna height", header_height, format!("{arp:.4} m"), true);
                    }
                }
                Err(why) => add(
                    "antenna height",
                    header_height,
                    format!("can't compare: {why}"),
                    false,
                ),
            }
            match (
                h.first_epoch.and_then(|t| utc(t, &h)),
                h.last_epoch.and_then(|t| utc(t, &h)),
            ) {
                (Some(a), Some(b)) => {
                    span = Some(match span {
                        Some((x, y)) => (if a < x { a } else { x }, if b > y { b } else { y }),
                        None => (a, b),
                    });
                }
                _ => span_known = false,
            }
        }
        // The occupation within its files, to a second.
        if let (true, Some((first, last))) = (span_known, span)
            && (first.seconds_until(o.start) < -1.0 || o.end.seconds_until(last) < -1.0)
        {
            findings.push(Finding {
                occupation: o.id.to_string(),
                file: o.rinex.join(", "),
                what: "span".into(),
                header: format!("{first} to {last}"),
                occupation_value: format!("{} to {}", o.start, o.end),
                fixable: false,
            });
        }
    }
    Ok(findings)
}

/// Writes an overlay for each file whose header can be made to agree with
/// its occupation, recorded `by` someone at `now`. Returns the overlays'
/// ids. What can't be fixed this way is left for a person.
pub fn fix<S: Store + ?Sized>(
    store: &mut S,
    by: &str,
    now: Timestamp,
) -> Result<Vec<String>, ReconcileError> {
    let loaded = load(store)?;
    let findings = compare(store, &loaded)?;
    let mut taken: Vec<String> = loaded.overlays.iter().map(|o| o.id.to_string()).collect();
    let mut files: Vec<&str> = findings
        .iter()
        .filter(|f| f.fixable)
        .map(|f| f.file.as_str())
        .collect();
    files.sort_unstable();
    files.dedup();
    let mut written = Vec::new();
    for file in files {
        let Some(o) = loaded
            .occupations
            .iter()
            .find(|o| o.rinex.iter().any(|r| r == file))
        else {
            continue;
        };
        let h = header(store, &loaded.overlays, file)?;
        let mine: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.fixable && f.file == file)
            .collect();
        let has = |what: &str| mine.iter().any(|f| f.what == what);
        let height = arp_height(o, Catalogue::igs()).ok();
        let rinex = RinexHeader {
            marker_name: has("marker name").then(|| o.mark.to_string()),
            receiver: has("receiver").then(|| format!("{} {}", o.receiver.make, o.receiver.model)),
            antenna: has("antenna").then(|| o.antenna.igs.clone()),
            radome: has("radome").then(|| o.antenna.radome.clone()),
            antenna_delta: has("antenna height")
                .then(|| {
                    let [_, e, n] = h.antenna_delta.unwrap_or_default();
                    height.map(|v| [v, e, n])
                })
                .flatten(),
        };
        let reason = format!(
            "The occupation {} (the field record) says otherwise: {}",
            o.id,
            mine.iter()
                .map(|f| format!("{} {} (header {})", f.what, f.occupation_value, f.header))
                .collect::<Vec<_>>()
                .join("; ")
        );
        let stem: String = file
            .trim_start_matches("raw/")
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let date = now.to_string().get(..10).unwrap_or_default().to_owned();
        let id = (1..)
            .map(|n| match n {
                1 => format!("{date}-{stem}-header"),
                n => format!("{date}-{stem}-header-{n}"),
            })
            .map(|id| id.chars().take(60).collect::<String>())
            .find(|id| !taken.contains(id))
            .unwrap_or_default();
        let overlay = Overlay {
            schema: SCHEMA,
            id: OverlayId::new(id.clone()).map_err(|e| ReconcileError::Invalid(e.to_string()))?,
            file: file.to_owned(),
            reason,
            by: by.to_owned(),
            recorded: now,
            rinex: Some(rinex),
        };
        store.write(
            &ProjectPath::new(format!("overlays/{id}.toml"))?,
            overlay.to_toml().as_bytes(),
        )?;
        taken.push(id.clone());
        written.push(id);
    }
    Ok(written)
}

/// A raw RINEX file as plain RINEX with its overlays applied, and a
/// `COMMENT` for each, naming it and its reason: what an export delivers.
pub fn corrected_rinex<S: Store + ?Sized>(
    store: &S,
    file: &str,
) -> Result<Vec<u8>, ReconcileError> {
    let loaded = load(store)?;
    let bytes = store.read_raw(&ProjectPath::new(file)?)?;
    let comments: Vec<String> = loaded
        .overlays
        .iter()
        .filter(|o| o.file == file)
        .map(|o| format!("Corrected by overlay {} ({}): {}", o.id, o.by, o.reason))
        .collect();
    let mut out = Vec::new();
    rewrite_obs(
        &bytes[..],
        &mut out,
        &patch_for(&loaded.overlays, file),
        &comments,
    )
    .map_err(|error| ReconcileError::Rinex {
        file: file.to_owned(),
        error,
    })?;
    Ok(out)
}
