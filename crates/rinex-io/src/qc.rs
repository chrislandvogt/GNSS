//! Quality metrics for an observation file, gathered an epoch at a time:
//! completeness and gaps, observations per satellite and signal, signal
//! strength, cycle slips and multipath.
//!
//! The definitions, so the numbers can be checked:
//! - **Interval:** the most common spacing between epochs. A **gap** is a
//!   spacing over 1.5 intervals. **Expected epochs** run from the first to
//!   the last at that interval.
//! - **Signal completeness:** a signal's observations over the epochs its
//!   system's satellites were tracked.
//! - **Cycle slips** are found on one dual-frequency pair per system (see
//!   [`crate::signal::pairs`]), along arcs that a satellite's data gap
//!   (over [`QcOptions::gap_seconds`]) ends without a slip:
//!   - Melbourne-Wübbena: the wide-lane phase minus the narrow-lane code, in
//!     wide-lane cycles, more than `mw_sigmas` standard deviations (and at
//!     least `mw_min_cycles`) from the arc's running mean;
//!   - geometry-free: the phase difference between the bands, in metres,
//!     stepping by more than `gf_metres` plus `gf_metres_per_second` times
//!     the time since the last epoch;
//!   - a loss-of-lock flag on either phase, counted separately as
//!     receiver-flagged.
//!
//!   A slip ends the arc and starts a new one.
//! - **Multipath**, MP1 and MP2 as teqc defines them: the code minus a
//!   combination of both phases that cancels geometry and ionosphere, less
//!   its mean over each arc; reported as the RMS over arcs of at least
//!   `min_arc_epochs` epochs, in metres.
//! - **Elevation coverage**, given broadcast orbits ([`Qc::with_orbits`]):
//!   the satellite-epochs expected are those the orbits put above
//!   [`QcOptions::elevation_cutoff`], healthy, at every expected epoch (so
//!   a gap in the file counts against it); the satellite-epochs tracked are
//!   those of them with observations. Each signal's coverage is its
//!   observations above the cutoff over the satellite-epochs expected.
//!   Only the systems the file has observation types for, and the orbits
//!   are for, are measured.
//!   Elevations come from the orbits every 30 s, interpolated between.

use std::collections::BTreeMap;

use crate::header::ObsHeader;
use crate::obs::{Epoch, EpochFlag, Sv};
use crate::orbit::{Orbits, Station};
use crate::signal::{Kind, SPEED_OF_LIGHT, frequency, pairs, parse_code};
use crate::time::{GnssTime, TICKS_PER_SECOND, TimeSystem};

/// Thresholds for slips and arcs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QcOptions {
    /// A satellite's data gap longer than this, in seconds, ends its arc.
    /// `None`: three nominal intervals, or 90 s if there's no interval.
    pub gap_seconds: Option<f64>,
    /// Melbourne-Wübbena: how many standard deviations from the arc's mean
    /// is a slip.
    pub mw_sigmas: f64,
    /// Melbourne-Wübbena: the least jump, in wide-lane cycles, that's a slip.
    pub mw_min_cycles: f64,
    /// Geometry-free: the step, in metres, that's a slip, plus
    /// `gf_metres_per_second` for each second between epochs (for the
    /// ionosphere's drift).
    pub gf_metres: f64,
    /// See `gf_metres`.
    pub gf_metres_per_second: f64,
    /// Arcs shorter than this many epochs don't count towards multipath.
    pub min_arc_epochs: usize,
    /// The elevation, in degrees, above which satellites are expected to
    /// be tracked.
    pub elevation_cutoff: f64,
}

impl Default for QcOptions {
    fn default() -> Self {
        Self {
            gap_seconds: None,
            mw_sigmas: 4.0,
            mw_min_cycles: 1.0,
            gf_metres: 0.03,
            gf_metres_per_second: 0.0005,
            min_arc_epochs: 10,
            elevation_cutoff: 10.0,
        }
    }
}

/// A gap between epochs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gap {
    /// The last epoch before it.
    pub after: GnssTime,
    /// The first epoch after it.
    pub before: GnssTime,
}

impl Gap {
    /// Its length in seconds, from epoch to epoch.
    pub fn seconds(&self) -> f64 {
        self.before.seconds_since(self.after)
    }
}

/// Mean, least and greatest of some values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    /// How many.
    pub count: usize,
    /// Their mean.
    pub mean: f64,
    /// The least.
    pub min: f64,
    /// The greatest.
    pub max: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct Running {
    count: usize,
    sum: f64,
    min: f64,
    max: f64,
}

impl Running {
    fn add(&mut self, v: f64) {
        if self.count == 0 {
            self.min = v;
            self.max = v;
        }
        self.count += 1;
        self.sum += v;
        self.min = self.min.min(v);
        self.max = self.max.max(v);
    }

    fn stats(&self) -> Option<Stats> {
        (self.count > 0).then(|| Stats {
            count: self.count,
            mean: self.sum / self.count as f64,
            min: self.min,
            max: self.max,
        })
    }
}

/// One signal's observations.
#[derive(Debug, Clone, PartialEq)]
pub struct SignalQc {
    /// How many.
    pub observations: usize,
    /// Over the epochs the system's satellites were tracked, 0 to 1.
    pub completeness: f64,
    /// For a signal strength (`S`), its values.
    pub strength: Option<Stats>,
    /// Given orbits: its observations above the elevation cutoff over the
    /// satellite-epochs expected there, 0 to 1.
    pub coverage: Option<f64>,
}

/// How much of what was in the sky a system's satellites were tracked,
/// from broadcast orbits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coverage {
    /// The elevation cutoff, degrees.
    pub cutoff: f64,
    /// Healthy satellite-epochs above the cutoff, at every expected epoch.
    pub expected: usize,
    /// Of those, the ones with observations.
    pub tracked: usize,
    /// Satellite-epochs tracked below the cutoff.
    pub below_cutoff: usize,
    /// Satellite-epochs tracked whose ephemeris says they're unhealthy.
    pub unhealthy: usize,
    /// Satellite-epochs tracked with no ephemeris for the time.
    pub no_ephemeris: usize,
}

impl Coverage {
    /// Tracked over expected, 0 to 1.
    pub fn ratio(&self) -> Option<f64> {
        (self.expected > 0).then(|| self.tracked as f64 / self.expected as f64)
    }
}

/// Multipath on one band.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Multipath {
    /// RMS in metres.
    pub rms: f64,
    /// The observations it's from.
    pub observations: usize,
}

/// One system's metrics.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemQc {
    /// Its satellites seen.
    pub satellites: usize,
    /// Satellite-epochs: each satellite at each epoch it was tracked.
    pub satellite_epochs: usize,
    /// Each observation code's metrics.
    pub signals: BTreeMap<String, SignalQc>,
    /// The bands used for slips and multipath, if it had a pair.
    pub pair: Option<(u8, u8)>,
    /// Slips found from the observations.
    pub slips: usize,
    /// Losses of lock the receiver flagged.
    pub flagged: usize,
    /// Multipath by band.
    pub multipath: BTreeMap<u8, Multipath>,
    /// Elevation coverage, given orbits.
    pub coverage: Option<Coverage>,
}

/// One satellite's metrics.
#[derive(Debug, Clone, PartialEq)]
pub struct SatelliteQc {
    /// Epochs it was tracked.
    pub epochs: usize,
    /// Slips found from the observations.
    pub slips: usize,
    /// Losses of lock the receiver flagged.
    pub flagged: usize,
}

/// The metrics for a file.
#[derive(Debug, Clone, PartialEq)]
pub struct QcReport {
    /// The first and last epochs with observations.
    pub first: Option<GnssTime>,
    /// See `first`.
    pub last: Option<GnssTime>,
    /// The interval in seconds: the most common spacing between epochs.
    pub interval: Option<f64>,
    /// Epochs with observations.
    pub epochs: usize,
    /// Epochs there would be at the interval, without gaps.
    pub expected_epochs: Option<usize>,
    /// Gaps between epochs.
    pub gaps: Vec<Gap>,
    /// Events (epoch flags 2 to 5).
    pub events: usize,
    /// Epochs after a power failure (flag 1).
    pub power_failures: usize,
    /// By system letter.
    pub systems: BTreeMap<char, SystemQc>,
    /// By satellite.
    pub satellites: BTreeMap<Sv, SatelliteQc>,
}

impl QcReport {
    /// Epochs over expected epochs, 0 to 1.
    pub fn completeness(&self) -> Option<f64> {
        self.expected_epochs
            .filter(|&e| e > 0)
            .map(|e| self.epochs as f64 / e as f64)
    }
}

/// Multipath sums over an arc, relative to its first value so the sums
/// stay small.
#[derive(Debug, Clone, Copy, Default)]
struct MpArc {
    offset: f64,
    count: usize,
    sum: f64,
    sum_sq: f64,
}

impl MpArc {
    fn add(&mut self, v: f64) {
        if self.count == 0 {
            self.offset = v;
        }
        let d = v - self.offset;
        self.count += 1;
        self.sum += d;
        self.sum_sq += d * d;
    }

    /// The sum of squared residuals from the arc's mean.
    fn residual(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        (self.sum_sq - self.sum * self.sum / self.count as f64).max(0.0)
    }
}

/// A satellite's current arc on its system's pair.
#[derive(Debug, Clone, Copy)]
struct Arc {
    /// The (phase, code) indices it follows on each band.
    signals: [(usize, usize); 2],
    count: usize,
    last: GnssTime,
    mw_mean: f64,
    mw_m2: f64,
    gf: f64,
    /// The geometry-free rate, m/s, from the last two epochs, if known.
    gf_rate: Option<f64>,
    mp: [MpArc; 2],
}

#[derive(Debug, Default)]
struct SatState {
    epochs: usize,
    slips: usize,
    flagged: usize,
    arc: Option<Arc>,
}

#[derive(Debug, Default)]
struct SystemState {
    satellites: usize,
    satellite_epochs: usize,
    signals: BTreeMap<String, (usize, Running)>,
    slips: usize,
    flagged: usize,
    mp: [(f64, usize); 2],
    tracked: usize,
    below_cutoff: usize,
    unhealthy: usize,
    no_ephemeris: usize,
    signals_above: BTreeMap<String, usize>,
}

/// Gathers metrics an epoch at a time.
pub struct Qc {
    options: QcOptions,
    epoch_ticks: Vec<i64>,
    events: usize,
    power_failures: usize,
    sats: BTreeMap<Sv, SatState>,
    systems: BTreeMap<char, SystemState>,
    /// Per system: its pair, and for each band the indices of the phase and
    /// code among the system's types, with the frequency where it doesn't
    /// depend on the satellite.
    plan: BTreeMap<char, Plan>,
    glonass: BTreeMap<u8, i8>,
    nominal_interval: Option<f64>,
    time_system: TimeSystem,
    sky: Option<Sky>,
}

/// Elevations from orbits, every 30 s, kept for the two latest times of
/// each satellite (epochs come in order).
struct Sky {
    orbits: Orbits,
    station: Station,
    cache: BTreeMap<Sv, [(i64, Elevation); 2]>,
}

/// An elevation in degrees and whether the satellite is healthy, if
/// there's an ephemeris.
type Elevation = Option<(f64, bool)>;

/// The most expected epochs coverage is worked out for: 115 days at 1 Hz.
/// A file with more (or with a tiny interval across a long span) gets none.
const MAX_COVERAGE_EPOCHS: usize = 10_000_000;

/// The spacing of computed elevations, in ticks.
const SKY_STEP: i64 = 30 * TICKS_PER_SECOND;

impl Sky {
    /// A satellite's elevation and health at a multiple of the step.
    fn at_step(&mut self, sv: Sv, step: i64) -> Elevation {
        let slots = self.cache.entry(sv).or_insert([(i64::MIN, None); 2]);
        if let Some((_, v)) = slots.iter().find(|(s, _)| *s == step) {
            return *v;
        }
        let t = GnssTime::from_ticks(step.saturating_mul(SKY_STEP));
        let value = self
            .orbits
            .position(sv, t)
            .map(|p| (self.station.look(p.position).1, p.healthy));
        // Replace the older slot.
        let older = usize::from(slots[0].0 > slots[1].0);
        if let Some(slot) = slots.get_mut(older) {
            *slot = (step, value);
        }
        value
    }

    /// A satellite's elevation (degrees) and health at `t`, GPS time.
    fn elevation(&mut self, sv: Sv, t: GnssTime) -> Elevation {
        let step = t.ticks().div_euclid(SKY_STEP);
        let into = t.ticks().rem_euclid(SKY_STEP);
        let (e0, h0) = self.at_step(sv, step)?;
        if into == 0 {
            return Some((e0, h0));
        }
        let (e1, h1) = self.at_step(sv, step + 1)?;
        let f = into as f64 / SKY_STEP as f64;
        Some((e0 + (e1 - e0) * f, h0 && h1))
    }
}

#[derive(Debug, Clone, Default)]
struct Plan {
    types: Vec<String>,
    pair: Option<(u8, u8)>,
    /// For each band of the pair, the (phase index, code index) pairs that
    /// can be used, best first: the same tracking mode (`L1C` with `C1C`),
    /// or in RINEX 2 `P` before `C`.
    candidates: [Vec<(usize, usize)>; 2],
}

/// The (phase, code) index pairs for a band, best first.
fn candidates(types: &[String], band: u8) -> Vec<(usize, usize)> {
    let of = |kind: Kind| {
        types
            .iter()
            .enumerate()
            .filter(move |(_, t)| parse_code(t) == Some((kind, band)))
            .collect::<Vec<_>>()
    };
    let (phases, mut codes) = (of(Kind::Phase), of(Kind::Code));
    // RINEX 2's P code before its C/A code.
    codes.sort_by_key(|(_, t)| !t.starts_with('P'));
    let mut out = Vec::new();
    for (pi, phase) in &phases {
        let mode = phase.get(2..3).unwrap_or("");
        // The same tracking mode first, then any code on the band.
        let same = codes
            .iter()
            .filter(|(_, c)| c.get(2..3).unwrap_or("") == mode);
        for (ci, _) in same.chain(codes.iter()) {
            if !out.contains(&(*pi, *ci)) {
                out.push((*pi, *ci));
            }
        }
    }
    out
}

impl Qc {
    /// Starts gathering for a file with this header.
    pub fn new(header: &ObsHeader, options: QcOptions) -> Self {
        let mut qc = Self {
            options,
            epoch_ticks: Vec::new(),
            events: 0,
            power_failures: 0,
            sats: BTreeMap::new(),
            systems: BTreeMap::new(),
            plan: BTreeMap::new(),
            glonass: header.glonass_channels.clone(),
            nominal_interval: header.interval,
            time_system: header.time_system,
            sky: None,
        };
        qc.replan(header);
        qc
    }

    /// Measures elevation coverage from broadcast orbits, seen from
    /// `position` (ECEF, metres), such as the header's approximate
    /// position. The orbits' GLONASS frequency channels fill in any the
    /// header doesn't give. Does nothing if the position is near the
    /// Earth's centre (a header's zeros).
    pub fn with_orbits(mut self, orbits: Orbits, position: [f64; 3]) -> Self {
        for (prn, channel) in orbits.glonass_channels() {
            self.glonass.entry(prn).or_insert(channel);
        }
        self.sky = Station::new(position).map(|station| Sky {
            orbits,
            station,
            cache: BTreeMap::new(),
        });
        self
    }

    /// Takes a new header, as after an event that changes observation types.
    pub fn replan(&mut self, header: &ObsHeader) {
        self.plan.clear();
        for system in ['G', 'R', 'E', 'C', 'J', 'I', 'S'] {
            let Some(types) = header.obs_types.for_system(system) else {
                continue;
            };
            let mut plan = Plan {
                types: types.to_vec(),
                ..Plan::default()
            };
            for &(a, b) in pairs(system) {
                let (ca, cb) = (candidates(types, a), candidates(types, b));
                if !ca.is_empty() && !cb.is_empty() {
                    plan.pair = Some((a, b));
                    plan.candidates = [ca, cb];
                    break;
                }
            }
            self.plan.insert(system, plan);
        }
        self.glonass.extend(&header.glonass_channels);
    }

    fn gap_limit(&self) -> f64 {
        self.options
            .gap_seconds
            .or(self.nominal_interval.map(|i| 3.0 * i))
            .unwrap_or(90.0)
    }

    /// Adds an epoch.
    pub fn add(&mut self, epoch: &Epoch) {
        match epoch.flag {
            EpochFlag::Ok | EpochFlag::PowerFailure => {}
            EpochFlag::CycleSlips => return,
            _ => {
                self.events += 1;
                return;
            }
        }
        if epoch.flag == EpochFlag::PowerFailure {
            self.power_failures += 1;
        }
        let Some(time) = epoch.time else { return };
        self.epoch_ticks.push(time.ticks());
        let gap_limit = self.gap_limit();
        let gps_time = self.time_system.to_gps(time);
        let cutoff = self.options.elevation_cutoff;
        for sat in &epoch.satellites {
            let sv = sat.sv;
            let Some(plan) = self.plan.get(&sv.system) else {
                continue;
            };
            let system = self.systems.entry(sv.system).or_default();
            let state = self.sats.entry(sv).or_insert_with(|| {
                system.satellites += 1;
                SatState::default()
            });
            state.epochs += 1;
            system.satellite_epochs += 1;
            // Above the cutoff and healthy: counts towards coverage.
            let mut above = false;
            if let Some(sky) = self.sky.as_mut() {
                match sky.elevation(sv, gps_time) {
                    None => system.no_ephemeris += 1,
                    Some((_, false)) => system.unhealthy += 1,
                    Some((e, true)) if e >= cutoff => {
                        system.tracked += 1;
                        above = true;
                    }
                    Some(_) => system.below_cutoff += 1,
                }
            }
            for (code, obs) in plan.types.iter().zip(&sat.observations) {
                let Some(obs) = obs else { continue };
                let entry = system.signals.entry(code.clone()).or_default();
                entry.0 += 1;
                if parse_code(code).is_some_and(|(k, _)| k == Kind::Strength) {
                    entry.1.add(obs.value);
                }
                if above {
                    *system.signals_above.entry(code.clone()).or_default() += 1;
                }
            }

            // Slips and multipath on the system's pair.
            let Some((a, b)) = plan.pair else {
                continue;
            };
            let channel = self.glonass.get(&sv.prn).copied();
            let (Some(fa), Some(fb)) = (
                frequency(sv.system, a, channel),
                frequency(sv.system, b, channel),
            ) else {
                continue;
            };
            let get = |i: usize| sat.observations.get(i).copied().flatten();
            // The first candidate on each band with both values.
            let pick = |list: &[(usize, usize)]| {
                list.iter()
                    .find_map(|&(p, c)| Some(((p, c), get(p)?, get(c)?)))
            };
            let (Some((sig_a, la, ca)), Some((sig_b, lb, cb))) =
                (pick(&plan.candidates[0]), pick(&plan.candidates[1]))
            else {
                continue;
            };
            let signals = [sig_a, sig_b];
            let flagged = |o: crate::obs::Observation| o.lli.is_some_and(|l| l & 1 == 1);
            let (wa, wb) = (SPEED_OF_LIGHT / fa, SPEED_OF_LIGHT / fb);
            let (phase_a, phase_b) = (la.value * wa, lb.value * wb);
            let wide = SPEED_OF_LIGHT / (fa - fb);
            let mw = ((fa * phase_a - fb * phase_b) / (fa - fb)
                - (fa * ca.value + fb * cb.value) / (fa + fb))
                / wide;
            let gf = phase_a - phase_b;
            let alpha = (fa / fb) * (fa / fb);
            let k = 2.0 / (alpha - 1.0);
            let mp = [
                ca.value - (1.0 + k) * phase_a + k * phase_b,
                cb.value - (alpha * k) * phase_a + (alpha * k - 1.0) * phase_b,
            ];

            // Does this epoch continue the arc?
            let mut new_arc = false;
            if let Some(arc) = &state.arc {
                let dt = time.seconds_since(arc.last);
                if dt > gap_limit || dt <= 0.0 || arc.signals != signals {
                    // A gap, or a change of tracked signal: a new arc.
                    new_arc = true;
                } else if flagged(la) || flagged(lb) {
                    state.flagged += 1;
                    system.flagged += 1;
                    new_arc = true;
                } else if arc.count >= 2 {
                    let sigma = (arc.mw_m2 / (arc.count - 1) as f64).sqrt();
                    let mw_jump = (mw - arc.mw_mean).abs()
                        > (self.options.mw_sigmas * sigma).max(self.options.mw_min_cycles);
                    // Predicted from the last value and rate, so the
                    // ionosphere's steady drift isn't taken for a slip.
                    let predicted = arc.gf + arc.gf_rate.unwrap_or(0.0) * dt;
                    let gf_jump = (gf - predicted).abs()
                        > self.options.gf_metres + self.options.gf_metres_per_second * dt;
                    if mw_jump || gf_jump {
                        state.slips += 1;
                        system.slips += 1;
                        new_arc = true;
                    }
                }
            }
            if new_arc && let Some(arc) = state.arc.take() {
                finish_arc(&arc, system, self.options.min_arc_epochs);
            }
            let arc = state.arc.get_or_insert(Arc {
                signals,
                count: 0,
                last: time,
                mw_mean: 0.0,
                mw_m2: 0.0,
                gf,
                gf_rate: None,
                mp: [MpArc::default(); 2],
            });
            if arc.count > 0 {
                let dt = time.seconds_since(arc.last);
                if dt > 0.0 {
                    arc.gf_rate = Some((gf - arc.gf) / dt);
                }
            }
            arc.count += 1;
            let delta = mw - arc.mw_mean;
            arc.mw_mean += delta / arc.count as f64;
            arc.mw_m2 += delta * (mw - arc.mw_mean);
            arc.gf = gf;
            arc.last = time;
            for (acc, v) in arc.mp.iter_mut().zip(mp) {
                acc.add(v);
            }
        }
    }

    /// The metrics.
    pub fn finish(mut self) -> QcReport {
        let options = self.options;
        for state in self.sats.iter_mut() {
            if let Some(arc) = state.1.arc.take()
                && let Some(system) = self.systems.get_mut(&state.0.system)
            {
                finish_arc(&arc, system, options.min_arc_epochs);
            }
        }
        let mut ticks = self.epoch_ticks;
        ticks.sort_unstable();
        ticks.dedup();
        let mut spacings: BTreeMap<i64, usize> = BTreeMap::new();
        for w in ticks.windows(2) {
            if let [a, b] = w {
                *spacings.entry(b - a).or_default() += 1;
            }
        }
        // The most common spacing; the shortest among equals.
        let interval = spacings
            .iter()
            .max_by(|x, y| x.1.cmp(y.1).then(y.0.cmp(x.0)))
            .map(|(&t, _)| t);
        let mut gaps = Vec::new();
        if let Some(step) = interval {
            for w in ticks.windows(2) {
                if let [a, b] = w {
                    // Over 1.5 intervals, in whole ticks.
                    if (b - a) * 2 > step * 3 {
                        gaps.push(Gap {
                            after: tick_time(*a),
                            before: tick_time(*b),
                        });
                    }
                }
            }
        }
        let first = ticks.first().copied();
        let last = ticks.last().copied();
        let expected_epochs = match (first, last, interval) {
            (Some(f), Some(l), Some(step)) if step > 0 => {
                usize::try_from((l - f + step / 2) / step + 1).ok()
            }
            (Some(_), Some(_), _) => Some(ticks.len()),
            _ => None,
        };
        // The healthy satellite-epochs above the cutoff at every expected
        // epoch, by system.
        let mut expected: BTreeMap<char, usize> = BTreeMap::new();
        if let (Some(sky), Some(f), Some(n)) = (self.sky.as_mut(), first, expected_epochs)
            && n <= MAX_COVERAGE_EPOCHS
        {
            let step = interval.unwrap_or(0);
            let svs: Vec<Sv> = sky
                .orbits
                .satellites()
                .filter(|sv| self.plan.contains_key(&sv.system))
                .collect();
            for sv in svs {
                let count = expected.entry(sv.system).or_default();
                for k in 0..n {
                    let Some(t) = i64::try_from(k)
                        .ok()
                        .and_then(|k| k.checked_mul(step))
                        .and_then(|d| d.checked_add(f))
                    else {
                        break;
                    };
                    let t = self.time_system.to_gps(GnssTime::from_ticks(t));
                    if sky
                        .elevation(sv, t)
                        .is_some_and(|(e, ok)| ok && e >= options.elevation_cutoff)
                    {
                        *count += 1;
                    }
                }
            }
        }
        // The systems the orbits are for; coverage only for those.
        let orbit_systems: Vec<char> = self
            .sky
            .as_ref()
            .map(|sky| sky.orbits.satellites().map(|sv| sv.system).collect())
            .unwrap_or_default();
        let systems = self
            .systems
            .into_iter()
            .map(|(letter, s)| {
                let pair = self.plan.get(&letter).and_then(|p| p.pair);
                let expected = expected.get(&letter).copied().unwrap_or(0);
                let with_orbits = orbit_systems.contains(&letter);
                let coverage = with_orbits.then_some(Coverage {
                    cutoff: options.elevation_cutoff,
                    expected,
                    tracked: s.tracked,
                    below_cutoff: s.below_cutoff,
                    unhealthy: s.unhealthy,
                    no_ephemeris: s.no_ephemeris,
                });
                let signals = s
                    .signals
                    .into_iter()
                    .map(|(code, (n, strength))| {
                        let completeness = if s.satellite_epochs > 0 {
                            n as f64 / s.satellite_epochs as f64
                        } else {
                            0.0
                        };
                        let above = s.signals_above.get(&code).copied().unwrap_or(0);
                        (
                            code,
                            SignalQc {
                                observations: n,
                                completeness,
                                strength: strength.stats(),
                                coverage: (with_orbits && expected > 0)
                                    .then(|| above as f64 / expected as f64),
                            },
                        )
                    })
                    .collect();
                let mut multipath = BTreeMap::new();
                if let Some((a, b)) = pair {
                    for (band, (sum, n)) in [a, b].into_iter().zip(s.mp) {
                        if n > 0 {
                            multipath.insert(
                                band,
                                Multipath {
                                    rms: (sum / n as f64).sqrt(),
                                    observations: n,
                                },
                            );
                        }
                    }
                }
                (
                    letter,
                    SystemQc {
                        satellites: s.satellites,
                        satellite_epochs: s.satellite_epochs,
                        signals,
                        pair,
                        slips: s.slips,
                        flagged: s.flagged,
                        multipath,
                        coverage,
                    },
                )
            })
            .collect();
        let satellites = self
            .sats
            .into_iter()
            .map(|(sv, s)| {
                (
                    sv,
                    SatelliteQc {
                        epochs: s.epochs,
                        slips: s.slips,
                        flagged: s.flagged,
                    },
                )
            })
            .collect();
        QcReport {
            first: first.map(tick_time),
            last: last.map(tick_time),
            interval: interval.map(|t| t as f64 / TICKS_PER_SECOND as f64),
            epochs: ticks.len(),
            expected_epochs,
            gaps,
            events: self.events,
            power_failures: self.power_failures,
            systems,
            satellites,
        }
    }
}

fn finish_arc(arc: &Arc, system: &mut SystemState, min_epochs: usize) {
    if arc.count < min_epochs.max(2) {
        return;
    }
    for (total, acc) in system.mp.iter_mut().zip(&arc.mp) {
        total.0 += acc.residual();
        total.1 += acc.count;
    }
}

fn tick_time(ticks: i64) -> GnssTime {
    GnssTime::from_ticks(ticks)
}
