//! The header of an observation file: what was observed, where, and with
//! what equipment (RINEX 2.11, 3.0x and 4.0x).

use std::collections::BTreeMap;

use crate::RinexError;
use crate::text::{cols, label, number, text};
use crate::time::{GnssTime, TimeSystem};

/// A RINEX version, such as 3.04.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    /// 2, 3 or 4.
    pub major: u8,
    /// The minor version, in hundredths: 11 for 2.11, 4 for 3.04.
    pub minor: u8,
}

/// A receiver (`REC # / TYPE / VERS`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Receiver {
    /// Its serial number.
    pub number: String,
    /// Its type, such as `LEICA GR30`.
    pub kind: String,
    /// Its firmware version.
    pub version: String,
}

/// An antenna (`ANT # / TYPE`), named as in the IGS antenna files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Antenna {
    /// Its serial number.
    pub number: String,
    /// Its type, such as `LEIAS10` (columns 21 to 36).
    pub kind: String,
    /// Its radome, such as `NONE` (columns 37 to 40), if given.
    pub radome: Option<String>,
}

/// The observation types recorded for each system, in the order the
/// records give them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObsTypes {
    /// RINEX 2: one list for every system, with two-character codes such
    /// as `L1` and `C1`.
    Common(Vec<String>),
    /// RINEX 3 and 4: a list for each system, keyed by its letter (`G`,
    /// `R`, `E`, `C`, `J`, `I`, `S`), with three-character codes such as
    /// `C1C`.
    BySystem(BTreeMap<char, Vec<String>>),
}

impl ObsTypes {
    /// The types recorded for a system, or `None` if it records none.
    pub fn for_system(&self, system: char) -> Option<&[String]> {
        match self {
            Self::Common(types) => Some(types),
            Self::BySystem(map) => map.get(&system).map(Vec::as_slice),
        }
    }
}

/// An observation file's header.
#[derive(Debug, Clone, PartialEq)]
pub struct ObsHeader {
    /// The RINEX version.
    pub version: Version,
    /// The satellite system: `G`, `R`, `E`, `C`, `J`, `I`, `S` or `M` for
    /// mixed.
    pub system: char,
    /// The program that wrote the file, and when (`PGM / RUN BY / DATE`).
    pub program: String,
    /// The mark's name (`MARKER NAME`).
    pub marker_name: String,
    /// The mark's number (`MARKER NUMBER`), if given.
    pub marker_number: Option<String>,
    /// The receiver.
    pub receiver: Receiver,
    /// The antenna.
    pub antenna: Antenna,
    /// The approximate position of the antenna reference point, ECEF in
    /// metres (`APPROX POSITION XYZ`), if given.
    pub approx_position: Option<[f64; 3]>,
    /// The antenna reference point's height above the mark and its east and
    /// north offsets, in metres (`ANTENNA: DELTA H/E/N`).
    pub antenna_delta: Option<[f64; 3]>,
    /// The observation types.
    pub obs_types: ObsTypes,
    /// The nominal interval between epochs in seconds (`INTERVAL`), if given.
    pub interval: Option<f64>,
    /// The time scale of the epochs.
    pub time_system: TimeSystem,
    /// The first epoch (`TIME OF FIRST OBS`), if given.
    pub first_epoch: Option<GnssTime>,
    /// The last epoch (`TIME OF LAST OBS`), if given.
    pub last_epoch: Option<GnssTime>,
    /// The leap seconds at the time of the file (`LEAP SECONDS`), if given.
    pub leap_seconds: Option<i32>,
    /// The comments, in order.
    pub comments: Vec<String>,
}

/// A header being read: the records seen so far, and continuation state.
pub(crate) struct HeaderBuilder {
    header: ObsHeader,
    /// For a RINEX 2 `# / TYPES OF OBSERV` list: how many types it has.
    common_count: usize,
    /// For a RINEX 3 `SYS / # / OBS TYPES` list: its system and count.
    system_types: Option<(char, usize)>,
    /// `TIME OF FIRST OBS` without an explicit time system (RINEX 2).
    first_fields: Option<Vec<u8>>,
}

impl HeaderBuilder {
    /// Starts a header from its first line, `RINEX VERSION / TYPE`.
    pub(crate) fn start(line: &[u8], line_no: u64) -> Result<Self, RinexError> {
        if label(line) != b"RINEX VERSION / TYPE" {
            return Err(RinexError::at(
                line_no,
                "a RINEX file starts with RINEX VERSION / TYPE; this isn't a RINEX file",
            ));
        }
        let version_text = text(line, 0, 9);
        let version = parse_version(&version_text).ok_or_else(|| {
            RinexError::at(
                line_no,
                format!("{version_text:?} isn't a RINEX version this reads (2.xx, 3.xx or 4.xx)"),
            )
        })?;
        let kind = cols(line, 20, 21).first().copied().unwrap_or(b' ');
        if kind != b'O' {
            return Err(RinexError::at(
                line_no,
                format!(
                    "this is a RINEX {} file, not an observation file",
                    match kind {
                        b'N' | b'G' | b'H' => "navigation",
                        b'M' => "meteorological",
                        b'C' => "clock",
                        _ => "non-observation",
                    }
                ),
            ));
        }
        let system = match cols(line, 40, 41).first().copied().unwrap_or(b' ') {
            b' ' => 'G',
            s => s as char,
        };
        let obs_types = if version.major == 2 {
            ObsTypes::Common(Vec::new())
        } else {
            ObsTypes::BySystem(BTreeMap::new())
        };
        Ok(Self {
            header: ObsHeader {
                version,
                system,
                program: String::new(),
                marker_name: String::new(),
                marker_number: None,
                receiver: Receiver::default(),
                antenna: Antenna::default(),
                approx_position: None,
                antenna_delta: None,
                obs_types,
                interval: None,
                time_system: default_time_system(system),
                first_epoch: None,
                last_epoch: None,
                leap_seconds: None,
                comments: Vec::new(),
            },
            common_count: 0,
            system_types: None,
            first_fields: None,
        })
    }

    /// Continues a header that's already been read, as when an event in the
    /// file's body carries header records.
    pub(crate) fn resume(header: ObsHeader) -> Self {
        Self {
            header,
            common_count: 0,
            system_types: None,
            first_fields: None,
        }
    }

    /// Reads one header record. True at `END OF HEADER`.
    pub(crate) fn record(&mut self, line: &[u8], line_no: u64) -> Result<bool, RinexError> {
        let h = &mut self.header;
        match label(line) {
            b"END OF HEADER" => return Ok(true),
            b"PGM / RUN BY / DATE" => h.program = text(line, 0, 60),
            b"COMMENT" => h.comments.push(text(line, 0, 60)),
            b"MARKER NAME" => h.marker_name = text(line, 0, 60),
            b"MARKER NUMBER" => {
                h.marker_number = Some(text(line, 0, 20)).filter(|n| !n.is_empty());
            }
            b"REC # / TYPE / VERS" => {
                h.receiver = Receiver {
                    number: text(line, 0, 20),
                    kind: text(line, 20, 40),
                    version: text(line, 40, 60),
                };
            }
            b"ANT # / TYPE" => {
                h.antenna = Antenna {
                    number: text(line, 0, 20),
                    kind: text(line, 20, 36),
                    radome: Some(text(line, 36, 40)).filter(|r| !r.is_empty()),
                };
            }
            b"APPROX POSITION XYZ" => {
                h.approx_position = Some(three(line, line_no, "APPROX POSITION XYZ")?);
            }
            b"ANTENNA: DELTA H/E/N" => {
                h.antenna_delta = Some(three(line, line_no, "ANTENNA: DELTA H/E/N")?);
            }
            b"INTERVAL" => h.interval = number(line, 0, 10),
            b"LEAP SECONDS" => h.leap_seconds = number(line, 0, 6),
            b"TIME OF FIRST OBS" => {
                self.first_fields = Some(line.to_vec());
                if let Some(system) = TimeSystem::parse(&text(line, 48, 51)) {
                    h.time_system = system;
                }
            }
            b"TIME OF LAST OBS" => {
                h.last_epoch = Some(header_time(line, line_no, "TIME OF LAST OBS")?);
            }
            b"# / TYPES OF OBSERV" => self.common_types(line, line_no)?,
            b"SYS / # / OBS TYPES" => self.system_types(line, line_no)?,
            _ => {}
        }
        Ok(false)
    }

    /// The finished header, checked.
    pub(crate) fn finish(mut self, line_no: u64) -> Result<ObsHeader, RinexError> {
        if let Some(fields) = self.first_fields.take() {
            self.header.first_epoch = Some(header_time(&fields, line_no, "TIME OF FIRST OBS")?);
        }
        let empty = match &self.header.obs_types {
            ObsTypes::Common(types) => types.is_empty(),
            ObsTypes::BySystem(map) => map.is_empty(),
        };
        if empty {
            return Err(RinexError::at(
                line_no,
                "the header lists no observation types (# / TYPES OF OBSERV or SYS / # / OBS TYPES)",
            ));
        }
        Ok(self.header)
    }

    /// A RINEX 2 `# / TYPES OF OBSERV` record: a count, then nine types a
    /// line, continued on further records.
    fn common_types(&mut self, line: &[u8], line_no: u64) -> Result<(), RinexError> {
        let ObsTypes::Common(types) = &mut self.header.obs_types else {
            return Err(RinexError::at(
                line_no,
                "# / TYPES OF OBSERV belongs in a RINEX 2 file",
            ));
        };
        if let Some(count) = number::<usize>(line, 0, 6) {
            types.clear();
            self.common_count = count;
        }
        for i in 0..9 {
            if types.len() >= self.common_count {
                break;
            }
            let code = text(line, 10 + 6 * i, 12 + 6 * i);
            if code.is_empty() {
                return Err(RinexError::at(
                    line_no,
                    format!(
                        "# / TYPES OF OBSERV promises {} types but lists {}",
                        self.common_count,
                        types.len()
                    ),
                ));
            }
            types.push(code);
        }
        Ok(())
    }

    /// A RINEX 3 `SYS / # / OBS TYPES` record: a system, a count, then 13
    /// types a line, continued on records with a blank system.
    fn system_types(&mut self, line: &[u8], line_no: u64) -> Result<(), RinexError> {
        let ObsTypes::BySystem(map) = &mut self.header.obs_types else {
            return Err(RinexError::at(
                line_no,
                "SYS / # / OBS TYPES belongs in a RINEX 3 or 4 file",
            ));
        };
        let system = cols(line, 0, 1).first().copied().unwrap_or(b' ');
        if system != b' ' {
            let count = number::<usize>(line, 3, 6).ok_or_else(|| {
                RinexError::at(line_no, "SYS / # / OBS TYPES has no count of types")
            })?;
            map.insert(system as char, Vec::new());
            self.system_types = Some((system as char, count));
        }
        let Some((system, count)) = self.system_types else {
            return Err(RinexError::at(
                line_no,
                "a continuation of SYS / # / OBS TYPES comes before the record it continues",
            ));
        };
        let types = map.entry(system).or_default();
        for i in 0..13 {
            if types.len() >= count {
                break;
            }
            let code = text(line, 7 + 4 * i, 10 + 4 * i);
            if code.is_empty() {
                return Err(RinexError::at(
                    line_no,
                    format!(
                        "SYS / # / OBS TYPES promises {count} types for {system} but lists {}",
                        types.len()
                    ),
                ));
            }
            types.push(code);
        }
        Ok(())
    }
}

/// `2.11`, `3.04` or `4.01` as a version.
fn parse_version(text: &str) -> Option<Version> {
    let (major, minor) = text.split_once('.').unwrap_or((text, "0"));
    let major: u8 = major.trim().parse().ok()?;
    let minor_digits: String = minor.chars().take(2).collect();
    let minor: u8 = if minor_digits.is_empty() {
        0
    } else {
        format!("{minor_digits:0<2}").parse().ok()?
    };
    (2..=4).contains(&major).then_some(Version { major, minor })
}

/// The time scale a system's files use when the header doesn't say.
fn default_time_system(system: char) -> TimeSystem {
    match system {
        'R' => TimeSystem::Glonass,
        'E' => TimeSystem::Galileo,
        'C' => TimeSystem::BeiDou,
        'J' => TimeSystem::Qzss,
        'I' => TimeSystem::Irnss,
        _ => TimeSystem::Gps,
    }
}

/// Three F14.4 numbers, as in `APPROX POSITION XYZ`.
fn three(line: &[u8], line_no: u64, what: &str) -> Result<[f64; 3], RinexError> {
    let mut out = [0.0; 3];
    for (i, value) in out.iter_mut().enumerate() {
        *value = number(line, 14 * i, 14 * (i + 1)).ok_or_else(|| {
            RinexError::at(
                line_no,
                format!("{what} needs three numbers, each in 14 columns"),
            )
        })?;
    }
    Ok(out)
}

/// `TIME OF FIRST OBS` or `TIME OF LAST OBS`: 5I6, F13.7.
fn header_time(line: &[u8], line_no: u64, what: &str) -> Result<GnssTime, RinexError> {
    let field = |i: usize| number::<i64>(line, 6 * i, 6 * (i + 1));
    let time = match (field(0), field(1), field(2), field(3), field(4)) {
        (Some(y), Some(mo), Some(d), Some(h), Some(mi)) => {
            GnssTime::from_fields(y, mo, d, h, mi, &text(line, 30, 43))
        }
        _ => None,
    };
    time.ok_or_else(|| {
        RinexError::at(
            line_no,
            format!("{what} isn't a valid time (year, month, day, hour, minute in 6 columns each, then seconds)"),
        )
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{HeaderBuilder, ObsHeader, ObsTypes, Version, parse_version};
    use crate::RinexError;
    use crate::time::TimeSystem;

    pub(crate) fn read(text: &str) -> Result<ObsHeader, RinexError> {
        let mut lines = text
            .lines()
            .enumerate()
            .map(|(i, l)| (i as u64 + 1, l.as_bytes()));
        let (n, first) = lines.next().unwrap();
        let mut builder = HeaderBuilder::start(first, n)?;
        for (n, line) in lines {
            if builder.record(line, n)? {
                return builder.finish(n);
            }
        }
        panic!("no END OF HEADER")
    }

    const RINEX3: &str =
        "     3.03           OBSERVATION DATA    M: MIXED            RINEX VERSION / TYPE
GR30 V4.82          DTMR                20250614 235943 UTC PGM / RUN BY / DATE
SWTC00AUS                                                   MARKER NAME
50186M001                                                   MARKER NUMBER
4108K313084         LEICA GR30          4.82/7.403          REC # / TYPE / VERS
20251070            LEIAS10         NONE                    ANT # / TYPE
 -5031176.5013  2577123.9327 -2944479.4888                  APPROX POSITION XYZ
        0.0000        0.0000        0.0000                  ANTENNA: DELTA H/E/N
G   15 C1C L1C D1C S1C C2W L2W D2W S2W C2L L2L D2L S2L C5Q  SYS / # / OBS TYPES
       L5Q D5Q                                              SYS / # / OBS TYPES
E    4 C1C L1C C5Q L5Q                                      SYS / # / OBS TYPES
    30.000                                                  INTERVAL
  2025     6    15     0     0    0.0000000     GPS         TIME OF FIRST OBS
  2025     6    15    23    59   30.0000000     GPS         TIME OF LAST OBS
    18  2371     1                                          LEAP SECONDS
                                                            END OF HEADER";

    #[test]
    fn a_rinex3_header_reads() {
        let h = read(RINEX3).unwrap();
        assert_eq!(h.version, Version { major: 3, minor: 3 });
        assert_eq!(h.system, 'M');
        assert_eq!(h.marker_name, "SWTC00AUS");
        assert_eq!(h.marker_number.as_deref(), Some("50186M001"));
        assert_eq!(h.receiver.kind, "LEICA GR30");
        assert_eq!(h.receiver.version, "4.82/7.403");
        assert_eq!(h.antenna.kind, "LEIAS10");
        assert_eq!(h.antenna.radome.as_deref(), Some("NONE"));
        assert_eq!(
            h.approx_position,
            Some([-5031176.5013, 2577123.9327, -2944479.4888])
        );
        assert_eq!(h.antenna_delta, Some([0.0; 3]));
        let gps = h.obs_types.for_system('G').unwrap();
        assert_eq!(gps.len(), 15);
        assert_eq!((gps[0].as_str(), gps[14].as_str()), ("C1C", "D5Q"));
        assert_eq!(
            h.obs_types.for_system('E').unwrap(),
            ["C1C", "L1C", "C5Q", "L5Q"]
        );
        assert!(h.obs_types.for_system('R').is_none());
        assert_eq!(h.interval, Some(30.0));
        assert_eq!(h.time_system, TimeSystem::Gps);
        assert_eq!(
            h.first_epoch.unwrap().to_string(),
            "2025-06-15T00:00:00.0000000"
        );
        assert_eq!(
            h.last_epoch.unwrap().to_string(),
            "2025-06-15T23:59:30.0000000"
        );
        // The rest of LEAP SECONDS is optional, and often absent.
        assert_eq!(h.leap_seconds, Some(18));
    }

    #[test]
    fn a_rinex2_header_reads() {
        let h = read(
            "     2.10           OBSERVATION DATA    G (GPS)             RINEX VERSION / TYPE
0759                                                        MARKER NAME
                    TRM29659.00                             ANT # / TYPE
    10    L1    L2    C1    P1    P2    D1    D2    S1    S2# / TYPES OF OBSERV
          C2                                                # / TYPES OF OBSERV
  2005     4     2     0     0    0.000000                  TIME OF FIRST OBS
                                                            END OF HEADER",
        )
        .unwrap();
        assert_eq!(
            h.version,
            Version {
                major: 2,
                minor: 10
            }
        );
        assert_eq!(h.antenna.kind, "TRM29659.00");
        assert_eq!(h.antenna.radome, None);
        let ObsTypes::Common(types) = &h.obs_types else {
            panic!()
        };
        assert_eq!(types.len(), 10);
        assert_eq!(types[9], "C2");
        assert_eq!(h.obs_types.for_system('R').unwrap().len(), 10);
        assert_eq!(
            h.first_epoch.unwrap().to_string(),
            "2005-04-02T00:00:00.0000000"
        );
    }

    #[test]
    fn bad_headers_say_why() {
        let nav =
            "     3.04           NAVIGATION DATA     M                   RINEX VERSION / TYPE\n";
        assert!(
            read(nav)
                .unwrap_err()
                .to_string()
                .contains("navigation file")
        );
        let wrong = "hello\n";
        assert!(
            read(wrong)
                .unwrap_err()
                .to_string()
                .starts_with("line 1: a RINEX file starts")
        );
        let short = RINEX3.replace("C5Q  SYS", "     SYS");
        let err = read(&short).unwrap_err().to_string();
        assert_eq!(
            err,
            "line 9: SYS / # / OBS TYPES promises 15 types for G but lists 12"
        );
        let no_types: String = RINEX3
            .lines()
            .filter(|l| !l.contains("OBS TYPES"))
            .map(|l| format!("{l}\n"))
            .collect();
        assert!(
            read(&no_types)
                .unwrap_err()
                .to_string()
                .contains("no observation types")
        );
        let bad_pos = RINEX3.replace(" -5031176.5013", "        banana");
        assert!(
            read(&bad_pos)
                .unwrap_err()
                .to_string()
                .starts_with("line 7: APPROX POSITION XYZ")
        );
    }

    #[test]
    fn versions_read() {
        assert_eq!(
            parse_version("2.11"),
            Some(Version {
                major: 2,
                minor: 11
            })
        );
        assert_eq!(parse_version("3.05"), Some(Version { major: 3, minor: 5 }));
        assert_eq!(parse_version("4.0"), Some(Version { major: 4, minor: 0 }));
        assert_eq!(parse_version("3"), Some(Version { major: 3, minor: 0 }));
        assert_eq!(parse_version("1.0"), None);
        assert_eq!(parse_version("x"), None);
    }
}
