//! Occupations in observation files: one set-up of one receiver and
//! antenna over one mark, without moving.
//!
//! A file is split into [`Segment`]s where the set-up changes:
//! - at a new site occupation (epoch flag 3);
//! - where the antenna starts moving (flag 2): what follows, until a new
//!   site occupation, is kinematic;
//! - where header records in an event (flag 4) change the marker, the
//!   receiver, the antenna or its height.
//!
//! Then [`merge`] joins segments of the same set-up that follow on from
//! each other, as a day logged in hourly files is, into one [`Occupation`].
//!
//! A gap within a set-up doesn't split it: the receiver may only have lost
//! power. A long one is worth a look, though, since an antenna set up
//! again over the same mark, without a new header, looks the same.

use std::io::BufRead;

use crate::header::{Antenna, ObsHeader, Receiver};
use crate::qc::{Gap, spacing};
use crate::time::{GnssTime, TICKS_PER_SECOND, TimeSystem};
use crate::{EpochFlag, RinexError, read_obs};

/// What a set-up is, from the header.
#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    /// The marker name.
    pub marker: String,
    /// The marker number, if given.
    pub marker_number: Option<String>,
    /// The receiver.
    pub receiver: Receiver,
    /// The antenna.
    pub antenna: Antenna,
    /// The antenna's height and offsets (H/E/N), metres.
    pub antenna_delta: Option<[f64; 3]>,
    /// The approximate position, ECEF metres.
    pub approx_position: Option<[f64; 3]>,
}

impl Setup {
    fn from_header(h: &ObsHeader) -> Self {
        Self {
            marker: h.marker_name.clone(),
            marker_number: h.marker_number.clone(),
            receiver: h.receiver.clone(),
            antenna: h.antenna.clone(),
            antenna_delta: h.antenna_delta,
            approx_position: h.approx_position,
        }
    }

    /// Whether two set-ups are the same: the same marker, receiver,
    /// antenna and radome (and their serial numbers), and the same antenna
    /// height and offsets to a millimetre. The approximate position isn't
    /// compared: writers update it as they go.
    pub fn same_as(&self, other: &Setup) -> bool {
        let delta = match (self.antenna_delta, other.antenna_delta) {
            (Some(a), Some(b)) => a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.0005),
            (a, b) => a.is_none() && b.is_none(),
        };
        self.marker == other.marker
            && self.marker_number == other.marker_number
            && self.receiver.kind == other.receiver.kind
            && self.receiver.number == other.receiver.number
            && self.antenna.kind == other.antenna.kind
            && self.antenna.radome == other.antenna.radome
            && self.antenna.number == other.antenna.number
            && delta
    }
}

/// A run of a file's epochs with one set-up.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// The set-up.
    pub setup: Setup,
    /// The epochs' time scale.
    pub time_system: TimeSystem,
    /// The first and last epochs with observations.
    pub first: GnssTime,
    /// See `first`.
    pub last: GnssTime,
    /// Epochs with observations.
    pub epochs: usize,
    /// The interval in seconds: the most common spacing between epochs.
    pub interval: Option<f64>,
    /// Gaps between epochs (over 1.5 intervals).
    pub gaps: Vec<Gap>,
    /// Whether the antenna was moving (between flags 2 and 3).
    pub kinematic: bool,
}

/// What's being gathered into a segment.
struct Open {
    setup: Setup,
    kinematic: bool,
    ticks: Vec<i64>,
}

impl Open {
    fn close(self, time_system: TimeSystem, out: &mut Vec<Segment>) {
        let mut ticks = self.ticks;
        ticks.sort_unstable();
        ticks.dedup();
        let (Some(&first), Some(&last)) = (ticks.first(), ticks.last()) else {
            return;
        };
        let (interval, gaps) = spacing(&ticks);
        out.push(Segment {
            setup: self.setup,
            time_system,
            first: GnssTime::from_ticks(first),
            last: GnssTime::from_ticks(last),
            epochs: ticks.len(),
            interval: interval.map(|t| t as f64 / TICKS_PER_SECOND as f64),
            gaps,
            kinematic: self.kinematic,
        });
    }
}

/// Reads an observation file, plain or Hatanaka-compressed, a line at a
/// time, into its segments, in the file's order. Runs without epochs are
/// left out.
pub fn segments<R: BufRead>(reader: R) -> Result<Vec<Segment>, RinexError> {
    let mut reader = read_obs(reader)?;
    let time_system = reader.header().time_system;
    let mut out = Vec::new();
    let mut open = Open {
        setup: Setup::from_header(reader.header()),
        kinematic: false,
        ticks: Vec::new(),
    };
    while let Some(epoch) = reader.next_epoch()? {
        let restart = |open: &mut Open, kinematic: bool, setup: Setup, out: &mut Vec<Segment>| {
            let old = std::mem::replace(
                open,
                Open {
                    setup,
                    kinematic,
                    ticks: Vec::new(),
                },
            );
            old.close(time_system, out);
        };
        match epoch.flag {
            EpochFlag::Ok | EpochFlag::PowerFailure => {
                if let Some(t) = epoch.time {
                    open.ticks.push(t.ticks());
                }
            }
            EpochFlag::StartMoving => {
                let setup = open.setup.clone();
                restart(&mut open, true, setup, &mut out);
            }
            EpochFlag::NewSite => {
                restart(
                    &mut open,
                    false,
                    Setup::from_header(reader.header()),
                    &mut out,
                );
            }
            EpochFlag::HeaderInformation => {
                let setup = Setup::from_header(reader.header());
                if !setup.same_as(&open.setup) {
                    let kinematic = open.kinematic;
                    restart(&mut open, kinematic, setup, &mut out);
                } else {
                    open.setup = setup;
                }
            }
            EpochFlag::ExternalEvent | EpochFlag::CycleSlips => {}
        }
    }
    open.close(time_system, &mut out);
    Ok(out)
}

/// One set-up over one span, from one or more files' segments.
#[derive(Debug, Clone, PartialEq)]
pub struct Occupation {
    /// The set-up.
    pub setup: Setup,
    /// The epochs' time scale.
    pub time_system: TimeSystem,
    /// The first and last epochs.
    pub first: GnssTime,
    /// See `first`.
    pub last: GnssTime,
    /// Epochs with observations.
    pub epochs: usize,
    /// The shortest of its segments' intervals, seconds.
    pub interval: Option<f64>,
    /// Gaps within segments, and between segments that follow on.
    pub gaps: Vec<Gap>,
    /// The segments it's made of, as (file, segment) indices into what
    /// [`merge`] was given, in time order.
    pub parts: Vec<(usize, usize)>,
    /// Seconds by which its segments overlap, if they do: the same data in
    /// two files, perhaps.
    pub overlap_s: f64,
}

impl Occupation {
    /// Its length in seconds, first epoch to last.
    pub fn duration_s(&self) -> f64 {
        self.last.seconds_since(self.first)
    }
}

/// Joins static segments of the same set-up and time scale that follow on
/// within `join_s` seconds (or overlap) into occupations, in time order.
/// `files` holds each file's segments, as [`segments`] gives them;
/// kinematic segments are left out.
pub fn merge(files: &[Vec<Segment>], join_s: f64) -> Vec<Occupation> {
    let mut all: Vec<(usize, usize, &Segment)> = files
        .iter()
        .enumerate()
        .flat_map(|(f, segs)| segs.iter().enumerate().map(move |(s, seg)| (f, s, seg)))
        .filter(|(_, _, seg)| !seg.kinematic)
        .collect();
    all.sort_by_key(|(f, s, seg)| (seg.first, *f, *s));
    let mut out: Vec<Occupation> = Vec::new();
    for (f, s, seg) in all {
        // The latest occupation of the same set-up that this follows on.
        let joins = out.iter_mut().rev().find(|o| {
            o.setup.same_as(&seg.setup)
                && o.time_system == seg.time_system
                && seg.first.seconds_since(o.last) <= join_s
        });
        match joins {
            Some(o) => {
                let after = seg.first.seconds_since(o.last);
                if after < 0.0 {
                    o.overlap_s += (-after).min(seg.last.seconds_since(seg.first));
                } else {
                    let step = o
                        .interval
                        .into_iter()
                        .chain(seg.interval)
                        .fold(f64::INFINITY, f64::min);
                    if after > 1.5 * step {
                        o.gaps.push(Gap {
                            after: o.last,
                            before: seg.first,
                        });
                    }
                }
                o.gaps.extend(seg.gaps.iter().copied());
                o.gaps.sort_by_key(|g| g.after);
                o.epochs += seg.epochs;
                if seg.last > o.last {
                    o.last = seg.last;
                }
                o.interval = match (o.interval, seg.interval) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                // The newest set-up's details, such as its approximate
                // position.
                o.setup = seg.setup.clone();
                o.parts.push((f, s));
            }
            None => out.push(Occupation {
                setup: seg.setup.clone(),
                time_system: seg.time_system,
                first: seg.first,
                last: seg.last,
                epochs: seg.epochs,
                interval: seg.interval,
                gaps: seg.gaps.clone(),
                parts: vec![(f, s)],
                overlap_s: 0.0,
            }),
        }
    }
    out
}
