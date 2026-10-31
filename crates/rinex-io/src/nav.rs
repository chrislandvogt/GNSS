//! Reading RINEX navigation files: 2.11 (GPS `N`, GLONASS `G` and SBAS
//! `H`), 3.0x and 4.0x (mixed). The broadcast ephemerides are kept; the
//! header's ionosphere and time-system corrections and RINEX 4's other
//! records (`STO`, `EOP`, `ION`) aren't needed here and are skipped, as are
//! RINEX 4's CNAV, CNV1 to CNV3 and L1OC messages, whose layouts differ.
//!
//! A navigation file is small (a day of every system is a few megabytes),
//! so it's read whole.

use std::io::BufRead;

use crate::header::parse_version;
use crate::lines::Lines;
use crate::obs::Sv;
use crate::orbit::{Ephemeris, Kepler, StateVector};
use crate::text::{self, label};
use crate::time::{GnssTime, TICKS_PER_SECOND, TimeSystem};
use crate::{RinexError, Version};

/// A navigation file's contents.
#[derive(Debug, Clone, PartialEq)]
pub struct NavFile {
    /// The RINEX version.
    pub version: Version,
    /// The ephemerides, in the file's order.
    pub ephemerides: Vec<Ephemeris>,
}

/// How well a navigation file's ephemerides of one constellation cover a
/// window ([`NavFile::coverage`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Coverage {
    /// The constellation: `G`, `R`, `E`, `C` or `J`.
    pub system: char,
    /// The satellites with ephemerides.
    pub satellites: usize,
    /// The parts of the window no ephemeris of the constellation covers.
    pub gaps: Vec<(GnssTime, GnssTime)>,
}

impl Coverage {
    /// Whether the whole window is covered.
    pub fn complete(&self) -> bool {
        self.satellites > 0 && self.gaps.is_empty()
    }
}

impl NavFile {
    /// For each constellation, whether its ephemerides cover a window (GPS
    /// time): every moment within some ephemeris's validity
    /// ([`Ephemeris::validity`]). A navigation file of one constellation
    /// leaves the others uncovered, which in the M0 spike silently gave
    /// degraded solutions, so baselines aren't processed without it.
    pub fn coverage(&self, systems: &[char], window: (GnssTime, GnssTime)) -> Vec<Coverage> {
        systems
            .iter()
            .map(|&system| {
                let mine: Vec<&Ephemeris> = self
                    .ephemerides
                    .iter()
                    .filter(|e| e.sv().system == system)
                    .collect();
                let mut satellites: Vec<u8> = mine.iter().map(|e| e.sv().prn).collect();
                satellites.sort_unstable();
                satellites.dedup();
                let mut spans: Vec<(i64, i64)> = mine
                    .iter()
                    .map(|e| {
                        let half = (e.validity() * TICKS_PER_SECOND as f64) as i64;
                        (e.toe().ticks() - half, e.toe().ticks() + half)
                    })
                    .collect();
                spans.sort_unstable();
                let (start, end) = (window.0.ticks(), window.1.ticks());
                let mut gaps = Vec::new();
                let mut reached = start;
                for (a, b) in spans {
                    if b < reached {
                        continue;
                    }
                    if a > reached {
                        gaps.push((reached, a.min(end)));
                    }
                    reached = reached.max(b);
                    if reached >= end {
                        break;
                    }
                }
                if reached < end {
                    gaps.push((reached, end));
                }
                Coverage {
                    system,
                    satellites: satellites.len(),
                    gaps: gaps
                        .into_iter()
                        .filter(|(a, b)| a < b)
                        .map(|(a, b)| (GnssTime::from_ticks(a), GnssTime::from_ticks(b)))
                        .collect(),
                }
            })
            .collect()
    }
}

/// One record's lines, and the RINEX 4 message type if there was one.
struct Record {
    line: u64,
    lines: Vec<Vec<u8>>,
    message: Option<String>,
}

/// Reads a navigation file, gzipped or not.
pub fn read_nav<R: BufRead>(reader: R) -> Result<NavFile, RinexError> {
    let mut lines = Lines::new(crate::decoded(reader)?);
    let mut buf = Vec::new();
    if !lines.read(&mut buf)? {
        return Err(RinexError::at(1, "the file is empty"));
    }
    if label(&buf) != b"RINEX VERSION / TYPE" {
        return Err(RinexError::at(
            1,
            "this isn't a RINEX file: it doesn't start with RINEX VERSION / TYPE",
        ));
    }
    let version_text = text::text(&buf, 0, 9);
    let version = parse_version(&version_text)
        .ok_or_else(|| RinexError::at(1, format!("the version {version_text:?} isn't one")))?;
    let file_type = text::cols(&buf, 20, 21).first().copied().unwrap_or(b' ');
    // RINEX 2: the file type is the system.
    let v2_system = match (version.major, file_type) {
        (2, b'N') => 'G',
        (2, b'G') => 'R',
        (2, b'H') => 'S',
        (3 | 4, b'N' | b'G') => ' ',
        _ => {
            return Err(RinexError::at(
                1,
                format!(
                    "this is a RINEX {} file of type {:?}, not a navigation file",
                    version_text,
                    char::from(file_type)
                ),
            ));
        }
    };
    loop {
        if !lines.read(&mut buf)? {
            return Err(RinexError::at(
                lines.line(),
                "the file ended before END OF HEADER",
            ));
        }
        if label(&buf) == b"END OF HEADER" {
            break;
        }
    }

    let mut ephemerides = Vec::new();
    let mut record: Option<Record> = None;
    while lines.read(&mut buf)? {
        if text::trim_end(&buf).is_empty() {
            continue;
        }
        let starts = match version.major {
            // RINEX 2: an epoch line has the PRN's digit in column 2; an
            // orbit line is blank there.
            2 => buf.get(1).is_some_and(u8::is_ascii_digit),
            // RINEX 3: orbit lines are indented. RINEX 4: every record
            // starts with `>`.
            3 => buf.first().is_some_and(|&b| b != b' '),
            _ => buf.first() == Some(&b'>'),
        };
        if starts {
            if let Some(r) = record.take() {
                decode(&r, &version, v2_system, &mut ephemerides)?;
            }
            let message = (version.major >= 4).then(|| String::from_utf8_lossy(&buf).into_owned());
            record = Some(Record {
                line: lines.line(),
                lines: if message.is_some() {
                    Vec::new()
                } else {
                    vec![buf.clone()]
                },
                message,
            });
        } else if let Some(r) = record.as_mut() {
            r.lines.push(buf.clone());
        } else {
            return Err(RinexError::at(
                lines.line(),
                "an orbit line before any record's first line",
            ));
        }
    }
    if let Some(r) = record.take() {
        decode(&r, &version, v2_system, &mut ephemerides)?;
    }
    Ok(NavFile {
        version,
        ephemerides,
    })
}

/// A field: 19 columns, Fortran `D` exponents allowed. Blank is zero.
fn field(line: &[u8], start: usize, at: u64) -> Result<f64, RinexError> {
    let raw = String::from_utf8_lossy(text::cols(line, start, start + 19));
    let t = raw.trim();
    if t.is_empty() {
        return Ok(0.0);
    }
    t.replace(['D', 'd'], "E").parse::<f64>().map_err(|_| {
        RinexError::at(
            at,
            format!(
                "{t:?} in columns {}-{} isn't a number",
                start + 1,
                start + 19
            ),
        )
    })
}

fn decode(
    r: &Record,
    version: &Version,
    v2_system: char,
    out: &mut Vec<Ephemeris>,
) -> Result<(), RinexError> {
    // RINEX 4: only the ephemeris messages laid out as in RINEX 3.
    let line0 = if let Some(message) = &r.message {
        let words: Vec<&str> = message.split_whitespace().collect();
        let known = matches!(
            words.as_slice(),
            [">", "EPH", sat, kind]
                if matches!(
                    (sat.chars().next(), *kind),
                    (Some('G' | 'J' | 'I'), "LNAV")
                        | (Some('E'), "INAV" | "FNAV")
                        | (Some('C'), "D1" | "D2")
                        | (Some('R'), "FDMA")
                        | (Some('S'), "SBAS")
                )
        );
        if !known {
            return Ok(());
        }
        r.line + 1
    } else {
        r.line
    };
    let Some(first) = r.lines.first() else {
        return Err(RinexError::at(r.line, "the record has no lines"));
    };
    // Where the satellite, the epoch and the fields are.
    let v2 = version.major == 2;
    let (epoch_cols, firsts, rest) = if v2 {
        ((2, 22), [22, 41, 60], [3, 22, 41, 60])
    } else {
        ((3, 23), [23, 42, 61], [4, 23, 42, 61])
    };
    let sv = if v2 {
        // SBAS satellites are numbered PRN - 100, as in RINEX 3.
        let prn: Option<u8> = text::number(first, 0, 2);
        prn.map(|prn| Sv {
            system: v2_system,
            prn,
        })
    } else {
        let id = text::text(first, 0, 3);
        let mut chars = id.chars();
        chars.next().and_then(|system| {
            let prn = chars.as_str().trim().parse().ok()?;
            Some(Sv { system, prn })
        })
    };
    let Some(sv) = sv else {
        return Err(RinexError::at(
            line0,
            format!("{:?} isn't a satellite", text::text(first, 0, 3)),
        ));
    };
    let scale = match sv.system {
        'G' | 'E' | 'J' | 'I' | 'S' => TimeSystem::Gps,
        'C' => TimeSystem::BeiDou,
        'R' => TimeSystem::Utc,
        // A system this doesn't know.
        _ => return Ok(()),
    };
    let epoch_text = text::text(first, epoch_cols.0, epoch_cols.1);
    let parts: Vec<&str> = epoch_text.split_whitespace().collect();
    let epoch = match parts.as_slice() {
        [y, mo, d, h, mi, s] => (|| {
            let mut year: i64 = y.parse().ok()?;
            if v2 {
                year += if year < 80 { 2000 } else { 1900 };
            }
            GnssTime::from_fields(
                year,
                mo.parse().ok()?,
                d.parse().ok()?,
                h.parse().ok()?,
                mi.parse().ok()?,
                s,
            )
        })(),
        _ => None,
    };
    let Some(epoch) = epoch else {
        return Err(RinexError::at(
            line0,
            format!("{epoch_text:?} isn't an epoch"),
        ));
    };
    let toc = scale.to_gps(epoch);
    let mut f = Vec::with_capacity(32);
    for &c in &firsts {
        f.push(field(first, c, line0)?);
    }
    for (n, line) in (line0 + 1..).zip(r.lines.iter().skip(1)) {
        for &c in &rest {
            f.push(field(line, c, n)?);
        }
    }
    let need = if matches!(sv.system, 'R' | 'S') {
        15
    } else {
        25
    };
    if f.len() < need {
        return Err(RinexError::at(
            line0,
            format!(
                "the record for {}{:02} has {} orbit lines; it needs {}",
                sv.system,
                sv.prn,
                r.lines.len().saturating_sub(1),
                (need - 3).div_ceil(4)
            ),
        ));
    }
    let v = |i: usize| f.get(i).copied().unwrap_or(0.0);
    let health = |x: f64| {
        if x.is_finite() && x >= 0.0 {
            x as u32
        } else {
            u32::MAX
        }
    };
    out.push(match sv.system {
        'R' | 'S' => {
            let state = StateVector {
                sv,
                toe: toc,
                clock: [v(0), v(1)],
                position: [v(3) * 1e3, v(7) * 1e3, v(11) * 1e3],
                velocity: [v(4) * 1e3, v(8) * 1e3, v(12) * 1e3],
                acceleration: [v(5) * 1e3, v(9) * 1e3, v(13) * 1e3],
                health: health(v(6)),
                channel: (sv.system == 'R')
                    .then(|| v(10))
                    .filter(|c| (-7.0..=13.0).contains(c))
                    .map(|c| c as i8),
            };
            if sv.system == 'R' {
                Ephemeris::Glonass(state)
            } else {
                Ephemeris::Sbas(state)
            }
        }
        _ => {
            let toe_seconds = v(11);
            let mut week = v(21);
            if sv.system == 'C' {
                // BeiDou weeks count from 2006-01-01, 1356 GPS weeks on.
                week += 1356.0;
            }
            let week_ticks = 604_800 * TICKS_PER_SECOND;
            let mut toe_ticks = week_ticks
                .saturating_mul(week as i64)
                .saturating_add((toe_seconds * TICKS_PER_SECOND as f64).round() as i64);
            if sv.system == 'C' {
                toe_ticks = toe_ticks.saturating_add(14 * TICKS_PER_SECOND);
            }
            // The week can be the next or last one's near a week's end:
            // take the one nearest the clock's reference time.
            let near_toc = |ticks: i64| {
                let d = toc
                    .ticks()
                    .checked_sub(ticks)?
                    .checked_add(week_ticks / 2)?;
                ticks.checked_add(d.div_euclid(week_ticks).checked_mul(week_ticks)?)
            };
            let Some(toe_ticks) = near_toc(toe_ticks) else {
                return Err(RinexError::at(
                    line0,
                    format!("the week {week} and time of ephemeris {toe_seconds} are out of range"),
                ));
            };
            let mut sent_ticks = week_ticks
                .saturating_mul(week as i64)
                .saturating_add((v(27) * TICKS_PER_SECOND as f64).round() as i64);
            if sv.system == 'C' {
                sent_ticks = sent_ticks.saturating_add(14 * TICKS_PER_SECOND);
            }
            Ephemeris::Kepler(Kepler {
                sv,
                toc,
                toe: GnssTime::from_ticks(toe_ticks),
                transmitted: GnssTime::from_ticks(near_toc(sent_ticks).unwrap_or(toe_ticks)),
                toe_seconds,
                af: [v(0), v(1), v(2)],
                crs: v(4),
                delta_n: v(5),
                m0: v(6),
                cuc: v(7),
                e: v(8),
                cus: v(9),
                sqrt_a: v(10),
                cic: v(12),
                omega0: v(13),
                cis: v(14),
                i0: v(15),
                crc: v(16),
                omega: v(17),
                omega_dot: v(18),
                idot: v(19),
                health: health(v(24)),
                // Data source bit 8: F/NAV.
                fnav: sv.system == 'E' && (v(20) as u32) & (1 << 8) != 0,
            })
        }
    });
    Ok(())
}
