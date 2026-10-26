//! Observation records, read one epoch at a time.

use std::io::BufRead;

use crate::header::{HeaderBuilder, ObsHeader};
use crate::text::{cols, number, text, trim_end};
use crate::time::GnssTime;
use crate::{RinexError, Source};

/// A satellite: its system's letter and its number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sv {
    /// `G` (GPS), `R` (GLONASS), `E` (Galileo), `C` (BeiDou), `J` (QZSS),
    /// `I` (NavIC) or `S` (SBAS).
    pub system: char,
    /// The satellite's number within its system.
    pub prn: u8,
}

impl std::fmt::Display for Sv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{:02}", self.system, self.prn)
    }
}

impl Sv {
    /// Reads a three-character satellite field such as `G05`, `R 7` or, in
    /// RINEX 2, ` 5` for GPS.
    pub(crate) fn parse(field: &[u8]) -> Option<Self> {
        let system = match field.first().copied().unwrap_or(b' ') {
            b' ' => 'G',
            s @ (b'G' | b'R' | b'E' | b'C' | b'J' | b'I' | b'S') => s as char,
            _ => return None,
        };
        let prn: u8 = std::str::from_utf8(cols(field, 1, 3))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        (prn > 0).then_some(Self { system, prn })
    }
}

/// One observation: a value, with its loss-of-lock indicator and signal
/// strength, as recorded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Observation {
    /// The value: metres for code, cycles for phase, hertz for Doppler, and
    /// dB-Hz (or the receiver's units) for signal strength.
    pub value: f64,
    /// The loss-of-lock indicator, 0 to 7, if given. Bit 0 set means lock was
    /// lost since the last epoch: a possible cycle slip.
    pub lli: Option<u8>,
    /// The signal strength, 1 (weakest) to 9, if given.
    pub ssi: Option<u8>,
}

/// One satellite's observations at an epoch, in the order of the header's
/// observation types for its system. A type it didn't record is `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct SatObservations {
    /// The satellite.
    pub sv: Sv,
    /// Its observations.
    pub observations: Vec<Option<Observation>>,
}

/// What an epoch record is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EpochFlag {
    /// 0: observations.
    Ok,
    /// 1: observations, after a power failure since the last epoch.
    PowerFailure,
    /// 2: the antenna starts moving.
    StartMoving,
    /// 3: a new site occupation (end of moving); header records follow.
    NewSite,
    /// 4: header records follow.
    HeaderInformation,
    /// 5: an external event.
    ExternalEvent,
    /// 6: cycle slips found and repaired, listed like observations.
    CycleSlips,
}

impl EpochFlag {
    fn parse(digit: u8) -> Option<Self> {
        Some(match digit {
            b'0' => Self::Ok,
            b'1' => Self::PowerFailure,
            b'2' => Self::StartMoving,
            b'3' => Self::NewSite,
            b'4' => Self::HeaderInformation,
            b'5' => Self::ExternalEvent,
            b'6' => Self::CycleSlips,
            _ => return None,
        })
    }

    /// Whether the record carries observations (flags 0, 1 and 6).
    pub fn has_observations(self) -> bool {
        matches!(self, Self::Ok | Self::PowerFailure | Self::CycleSlips)
    }
}

/// One epoch record.
#[derive(Debug, Clone, PartialEq)]
pub struct Epoch {
    /// When, on the file's time scale. An event (flags 2 to 5) may have no
    /// time.
    pub time: Option<GnssTime>,
    /// What the record is.
    pub flag: EpochFlag,
    /// The receiver clock offset in seconds, if given.
    pub clock_offset: Option<f64>,
    /// The satellites' observations: for flags 0 and 1, or the repaired
    /// cycle slips for flag 6.
    pub satellites: Vec<SatObservations>,
    /// An event's records (flags 2 to 5), as written. Header records among
    /// them also update [`ObsReader::header`].
    pub records: Vec<String>,
}

/// Something that gives a RINEX file's lines one at a time.
pub(crate) trait LineSource {
    /// The next line, without its ending, in `buf`. False at the end.
    fn read(&mut self, buf: &mut Vec<u8>) -> Result<bool, RinexError>;
    /// The number of the last line given, for messages.
    fn line(&self) -> u64;
}

/// Reads an observation file's epochs one at a time, holding only the
/// current epoch. Open one with [`read_obs`](crate::read_obs).
pub struct ObsReader<R> {
    source: Source<R>,
    header: ObsHeader,
    buf: Vec<u8>,
    done: bool,
}

impl<R: BufRead> ObsReader<R> {
    /// Reads the header, leaving the reader at the first epoch.
    pub(crate) fn new(mut source: Source<R>) -> Result<Self, RinexError> {
        let mut buf = Vec::new();
        if !source.read(&mut buf)? {
            return Err(RinexError::at(1, "the file is empty"));
        }
        let mut builder = HeaderBuilder::start(&buf, source.line())?;
        loop {
            if !source.read(&mut buf)? {
                return Err(RinexError::at(
                    source.line(),
                    "the file ends before END OF HEADER",
                ));
            }
            if builder.record(&buf, source.line())? {
                break;
            }
        }
        let header = builder.finish(source.line())?;
        Ok(Self {
            source,
            header,
            buf,
            done: false,
        })
    }

    /// The header, updated by any header records in events so far.
    pub fn header(&self) -> &ObsHeader {
        &self.header
    }

    /// The next epoch, or `None` at the end of the file. After an error,
    /// there are no more epochs.
    pub fn next_epoch(&mut self) -> Result<Option<Epoch>, RinexError> {
        if self.done {
            return Ok(None);
        }
        let result = self.read_epoch();
        if !matches!(result, Ok(Some(_))) {
            self.done = true;
        }
        result
    }

    fn read_epoch(&mut self) -> Result<Option<Epoch>, RinexError> {
        // Skip blank lines between records, as some writers leave at the end.
        loop {
            if !self.source.read(&mut self.buf)? {
                return Ok(None);
            }
            if !trim_end(&self.buf).is_empty() {
                break;
            }
        }
        if self.header.version.major == 2 {
            self.epoch_v2()
        } else {
            self.epoch_v3()
        }
    }

    fn err(&self, message: impl Into<String>) -> RinexError {
        RinexError::at(self.source.line(), message)
    }

    /// The next line, which must exist.
    fn more(&mut self, what: &str) -> Result<(), RinexError> {
        if self.source.read(&mut self.buf)? {
            Ok(())
        } else {
            Err(self.err(format!(
                "the file ends in the middle of an epoch, before {what}"
            )))
        }
    }

    /// RINEX 3 and 4: `> yyyy mm dd hh mm ss.sssssss  f nnn      clock`.
    fn epoch_v3(&mut self) -> Result<Option<Epoch>, RinexError> {
        let line = &self.buf;
        if line.first() != Some(&b'>') {
            return Err(self.err("expected an epoch record, starting with '>'"));
        }
        let flag =
            flag_at(line, 31).ok_or_else(|| self.err("the epoch flag (column 32) isn't 0 to 6"))?;
        let count: usize = number(line, 32, 35).ok_or_else(|| {
            self.err("the epoch has no satellite or record count (columns 33-35)")
        })?;
        let time = epoch_time(
            line,
            [(2, 6), (7, 9), (10, 12), (13, 15), (16, 18)],
            (18, 29),
            4,
        );
        let time = if flag.has_observations() || !trim_end(cols(line, 1, 29)).is_empty() {
            Some(time.ok_or_else(|| self.err("the epoch's date and time aren't valid"))?)
        } else {
            None
        };
        let clock_offset = optional_number(line, 41, 56)
            .map_err(|_| self.err("the receiver clock offset (columns 42-56) isn't a number"))?;
        let mut epoch = Epoch {
            time,
            flag,
            clock_offset,
            satellites: Vec::new(),
            records: Vec::new(),
        };
        if !flag.has_observations() {
            self.event_records(&mut epoch, count)?;
            return Ok(Some(epoch));
        }
        for _ in 0..count {
            self.more("a satellite's observations")?;
            let sv = Sv::parse(cols(&self.buf, 0, 3)).ok_or_else(|| {
                self.err(format!(
                    "{:?} isn't a satellite",
                    String::from_utf8_lossy(cols(&self.buf, 0, 3))
                ))
            })?;
            let n = self.types_for(sv)?;
            let observations = (0..n)
                .map(|i| field(&self.buf, 3 + 16 * i))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|bad| self.err(format!("{sv}: {bad:?} isn't a number")))?;
            epoch.satellites.push(SatObservations { sv, observations });
        }
        Ok(Some(epoch))
    }

    /// RINEX 2: ` yy mm dd hh mm ss.sssssss  f nnG01G02...clock`, with 12
    /// satellites a line, then each satellite's observations, 5 a line.
    fn epoch_v2(&mut self) -> Result<Option<Epoch>, RinexError> {
        let line = &self.buf;
        let flag =
            flag_at(line, 28).ok_or_else(|| self.err("the epoch flag (column 29) isn't 0 to 6"))?;
        let count: usize = number(line, 29, 32).ok_or_else(|| {
            self.err("the epoch has no satellite or record count (columns 30-32)")
        })?;
        let time = epoch_time(
            line,
            [(1, 3), (4, 6), (7, 9), (10, 12), (13, 15)],
            (15, 26),
            2,
        );
        let time = if flag.has_observations() || !trim_end(cols(line, 0, 26)).is_empty() {
            Some(time.ok_or_else(|| self.err("the epoch's date and time aren't valid"))?)
        } else {
            None
        };
        let clock_offset = optional_number(line, 68, 80)
            .map_err(|_| self.err("the receiver clock offset (columns 69-80) isn't a number"))?;
        let mut epoch = Epoch {
            time,
            flag,
            clock_offset,
            satellites: Vec::new(),
            records: Vec::new(),
        };
        if !flag.has_observations() {
            self.event_records(&mut epoch, count)?;
            return Ok(Some(epoch));
        }
        let mut svs = Vec::with_capacity(count);
        for i in 0..count {
            if i > 0 && i % 12 == 0 {
                self.more("the rest of the epoch's satellite list")?;
            }
            let at = 32 + 3 * (i % 12);
            let sv = Sv::parse(cols(&self.buf, at, at + 3)).ok_or_else(|| {
                self.err(format!(
                    "{:?} isn't a satellite",
                    String::from_utf8_lossy(cols(&self.buf, at, at + 3))
                ))
            })?;
            svs.push(sv);
        }
        for sv in svs {
            let n = self.types_for(sv)?;
            let mut observations = Vec::with_capacity(n);
            for i in 0..n {
                if i % 5 == 0 {
                    self.more("a satellite's observations")?;
                }
                observations.push(
                    field(&self.buf, 16 * (i % 5))
                        .map_err(|bad| self.err(format!("{sv}: {bad:?} isn't a number")))?,
                );
            }
            epoch.satellites.push(SatObservations { sv, observations });
        }
        Ok(Some(epoch))
    }

    /// How many observation types a satellite's system has.
    fn types_for(&self, sv: Sv) -> Result<usize, RinexError> {
        self.header
            .obs_types
            .for_system(sv.system)
            .map(<[String]>::len)
            .ok_or_else(|| {
                self.err(format!(
                    "{sv} is a {} satellite, but the header lists no observation types for that system",
                    sv.system
                ))
            })
    }

    /// An event's records, kept as written. Header records among them, as
    /// a new site occupation (flag 3) brings, update the header.
    fn event_records(&mut self, epoch: &mut Epoch, count: usize) -> Result<(), RinexError> {
        let mut builder = HeaderBuilder::resume(self.header.clone());
        for _ in 0..count {
            self.more("the event's records")?;
            let line = trim_end(&self.buf);
            if line.len() > 60 {
                builder.record(line, self.source.line())?;
            }
            epoch
                .records
                .push(String::from_utf8_lossy(line).into_owned());
        }
        self.header = builder.finish(self.source.line())?;
        Ok(())
    }
}

/// An epoch flag digit at a column.
fn flag_at(line: &[u8], at: usize) -> Option<EpochFlag> {
    EpochFlag::parse(cols(line, at, at + 1).first().copied().unwrap_or(b' '))
}

/// An epoch's time from its fields. A two-digit year is 1980 to 2079.
fn epoch_time(
    line: &[u8],
    fields: [(usize, usize); 5],
    seconds: (usize, usize),
    year_digits: usize,
) -> Option<GnssTime> {
    let [y, mo, d, h, mi] = fields.map(|(a, b)| number::<i64>(line, a, b));
    let mut year = y?;
    if year_digits == 2 {
        year += if year < 80 { 2000 } else { 1900 };
    }
    GnssTime::from_fields(year, mo?, d?, h?, mi?, &text(line, seconds.0, seconds.1))
}

/// A number that may be blank.
fn optional_number(line: &[u8], start: usize, end: usize) -> Result<Option<f64>, ()> {
    let field = text(line, start, end);
    if field.is_empty() {
        Ok(None)
    } else {
        field.parse().map(Some).map_err(|_| ())
    }
}

/// One observation field, F14.3 then the indicator and strength digits,
/// starting at a column. A blank value is no observation.
fn field(line: &[u8], at: usize) -> Result<Option<Observation>, String> {
    let value = text(line, at, at + 14);
    if value.is_empty() {
        return Ok(None);
    }
    let digit = |i: usize| {
        cols(line, at + i, at + i + 1)
            .first()
            .filter(|b| b.is_ascii_digit())
            .map(|b| b - b'0')
    };
    let value: f64 = value.parse().map_err(|_| value.clone())?;
    if !value.is_finite() {
        return Err(value.to_string());
    }
    Ok(Some(Observation {
        value,
        lli: digit(14),
        ssi: digit(15),
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{EpochFlag, Sv};
    use crate::read_obs;

    #[test]
    fn satellites_read_in_both_styles() {
        assert_eq!(
            Sv::parse(b"G05"),
            Some(Sv {
                system: 'G',
                prn: 5
            })
        );
        assert_eq!(
            Sv::parse(b"R 7"),
            Some(Sv {
                system: 'R',
                prn: 7
            })
        );
        assert_eq!(
            Sv::parse(b" 12"),
            Some(Sv {
                system: 'G',
                prn: 12
            })
        );
        assert_eq!(Sv::parse(b"X01"), None);
        assert_eq!(Sv::parse(b"G00"), None);
        assert_eq!(Sv::parse(b"G"), None);
        assert_eq!(
            Sv {
                system: 'E',
                prn: 3
            }
            .to_string(),
            "E03"
        );
    }

    const V3: &str =
        "     3.04           OBSERVATION DATA    M                   RINEX VERSION / TYPE
G    3 C1C L1C S1C                                          SYS / # / OBS TYPES
E    2 C1C L1C                                              SYS / # / OBS TYPES
                                                            END OF HEADER
> 2025 06 15 00 00  0.0000000  0  2      -0.000123456789
G05  21000000.123   110000000.12345        45.000
E11  23000000.4567
> 2025 06 15 00 00 30.0000000  3  2
TRIG                                                        MARKER NAME
NEW SITE                                                    COMMENT
> 2025 06 15 00 01  0.0000000  0  1
G05  21000100.123   110000500.12345
";

    #[test]
    fn rinex3_epochs_read() {
        let mut r = read_obs(V3.as_bytes()).unwrap();
        let e = r.next_epoch().unwrap().unwrap();
        assert_eq!(e.flag, EpochFlag::Ok);
        assert_eq!(e.time.unwrap().to_string(), "2025-06-15T00:00:00.0000000");
        assert_eq!(e.clock_offset, Some(-0.000123456789));
        assert_eq!(e.satellites.len(), 2);
        let g = &e.satellites[0];
        assert_eq!(g.sv.to_string(), "G05");
        let l1 = g.observations[1].unwrap();
        assert_eq!(
            (l1.value, l1.lli, l1.ssi),
            (110000000.123, Some(4), Some(5))
        );
        assert_eq!(g.observations[2].unwrap().value, 45.0);
        let e11 = &e.satellites[1];
        assert_eq!(e11.observations.len(), 2);
        assert_eq!(e11.observations[0].unwrap().lli, Some(7));
        assert_eq!(e11.observations[1], None);

        // A new site: its records are kept and the header follows them.
        let event = r.next_epoch().unwrap().unwrap();
        assert_eq!(event.flag, EpochFlag::NewSite);
        assert_eq!(event.records.len(), 2);
        assert_eq!(r.header().marker_name, "TRIG");
        let e = r.next_epoch().unwrap().unwrap();
        assert_eq!(e.satellites[0].observations[2], None);
        assert!(r.next_epoch().unwrap().is_none());
        assert!(r.next_epoch().unwrap().is_none());
    }

    /// An observation field: F14.3, then the indicator and strength.
    fn obs(value: &str, lli: char, ssi: char) -> String {
        format!("{value:>14}{lli}{ssi}")
    }

    #[test]
    fn rinex2_epochs_read_across_continuation_lines() {
        let mut text = String::from(
            "     2.11           OBSERVATION DATA    M (MIXED)           RINEX VERSION / TYPE
     6    L1    L2    C1    P2    S1    S2                  # / TYPES OF OBSERV
                                                            END OF HEADER
",
        );
        let svs = "G01G02G03G04G05G06G07G08G09G10G11R01";
        text.push_str(&format!(
            " 20  1  5  0  0  0.0000000  0 13{svs}{:>12}\n",
            ".123456789"
        ));
        text.push_str(&format!("{:32}G12\n", ""));
        let first = [
            obs("110000000.123", '6', ' '),
            obs("85000000.456", ' ', ' '),
            obs("21000000.789", ' ', ' '),
            obs("21000001.012", ' ', ' '),
            obs("45.000", ' ', ' '),
        ];
        text.push_str(&format!(
            "{}\n{}\n",
            first.concat(),
            obs("30.000", ' ', ' ')
        ));
        // The other 12 satellites: one value each.
        for _ in 0..12 {
            text.push_str(&format!("{}\n\n", obs("1.000", ' ', ' ')));
        }
        let mut r = read_obs(text.as_bytes()).unwrap();
        let e = r.next_epoch().unwrap().unwrap();
        assert_eq!(e.time.unwrap().to_string(), "2020-01-05T00:00:00.0000000");
        assert_eq!(e.clock_offset, Some(0.123456789));
        assert_eq!(e.satellites.len(), 13);
        assert_eq!(e.satellites[11].sv.to_string(), "R01");
        assert_eq!(e.satellites[12].sv.to_string(), "G12");
        let first = &e.satellites[0].observations;
        assert_eq!(first[0].unwrap().lli, Some(6));
        assert_eq!(first[0].unwrap().ssi, None);
        assert_eq!(first[4].unwrap().value, 45.0);
        assert_eq!(first[5].unwrap().value, 30.0);
        assert_eq!(e.satellites[12].observations[0].unwrap().value, 1.0);
        assert_eq!(e.satellites[12].observations[1], None);
        assert!(r.next_epoch().unwrap().is_none());
    }

    #[test]
    fn damaged_epochs_name_their_line() {
        let cases = [
            (
                V3.replace("> 2025 06 15 00 00  0", "  2025 06 15 00 00  0"),
                "line 5: expected an epoch record",
            ),
            (
                V3.replace("00  0.0000000  0  2", "00  0.0000000  9  2"),
                "line 5: the epoch flag",
            ),
            (
                V3.replace("2025 06 15 00 00  0", "2025 06 31 00 00  0"),
                "line 5: the epoch's date",
            ),
            (
                V3.replace("G05  21000000", "X05  21000000"),
                "line 6: \"X05\" isn't a satellite",
            ),
            (
                V3.replace("21000000.123", "21000000.1x3"),
                "line 6: G05: \"21000000.1x3\" isn't a number",
            ),
            (
                V3.replace("E11", "R11"),
                "line 7: R11 is a R satellite, but the header lists no",
            ),
            (
                V3.replace("-0.000123456789", "  clock?       "),
                "line 5: the receiver clock offset",
            ),
            (
                V3.lines().take(6).map(|l| format!("{l}\n")).collect(),
                "line 6: the file ends in the middle",
            ),
        ];
        for (text, expected) in cases {
            let mut r = read_obs(text.as_bytes()).unwrap();
            let err = loop {
                match r.next_epoch() {
                    Ok(Some(_)) => continue,
                    Ok(None) => panic!("no error for {expected}"),
                    Err(e) => break e.to_string(),
                }
            };
            assert!(err.starts_with(expected), "{err}\n  expected {expected}");
            assert!(r.next_epoch().unwrap().is_none());
        }
    }
}
