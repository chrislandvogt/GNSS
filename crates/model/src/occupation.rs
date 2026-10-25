use serde::{Deserialize, Serialize};

use crate::error::from_toml;
use crate::{MarkId, ModelError, OccupationId, SCHEMA, Timestamp};

/// One set-up of one receiver and antenna over one mark for one span of
/// time: a file in `occupations/`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Occupation {
    /// The schema version: 1 (`SCHEMA`), raised only when a migration is needed.
    pub schema: u32,
    /// The occupation's id, which is also its file name.
    pub id: OccupationId,
    /// The mark occupied.
    pub mark: MarkId,
    /// When observation started, in UTC.
    pub start: Timestamp,
    /// When observation ended, in UTC.
    pub end: Timestamp,
    /// The occupation's RINEX files in `raw/`, as paths from the project folder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rinex: Vec<String>,
    /// The field log, in `fieldlogs/`, if there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_log: Option<String>,
    /// The receiver.
    pub receiver: Receiver,
    /// The antenna.
    pub antenna: Antenna,
    /// The antenna height, as measured.
    pub height: HeightMeasurement,
}

impl Occupation {
    /// Reads an occupation file, checking it.
    pub fn from_toml(text: &str) -> Result<Self, ModelError> {
        let occupation: Occupation = from_toml(text)?;
        occupation
            .check()
            .map_err(|e| ModelError::Invalid(format!("occupation {}: {e}", occupation.id)))?;
        Ok(occupation)
    }

    /// The occupation as TOML.
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// The occupation's length in seconds.
    pub fn duration_s(&self) -> f64 {
        self.start.seconds_until(self.end)
    }

    /// Checks the occupation on its own.
    pub fn check(&self) -> Result<(), ModelError> {
        if self.schema != SCHEMA {
            return Err(ModelError::Invalid(format!(
                "schema {} isn't one this version reads ({SCHEMA})",
                self.schema
            )));
        }
        if self.duration_s() <= 0.0 {
            return Err(ModelError::Invalid(format!(
                "it ends ({}) before it starts ({})",
                self.end, self.start
            )));
        }
        for file in &self.rinex {
            check_path(file, "raw/")?;
        }
        if let Some(log) = &self.field_log {
            check_path(log, "fieldlogs/")?;
        }
        self.receiver.check()?;
        self.antenna.check()?;
        self.height.check()
    }
}

/// A receiver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Receiver {
    /// The maker, such as `Leica`.
    pub make: String,
    /// The model, such as `GS10`.
    pub model: String,
    /// The serial number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    /// The firmware version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware: Option<String>,
}

impl Receiver {
    fn check(&self) -> Result<(), ModelError> {
        if self.make.trim().is_empty() || self.model.trim().is_empty() {
            return Err(ModelError::Invalid(
                "the receiver needs its make and model".into(),
            ));
        }
        Ok(())
    }
}

/// An antenna, named as in the IGS's `rcvr_ant.tab`, which is how its
/// calibration is found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Antenna {
    /// The IGS antenna code, such as `LEIAS10` or `TRM59800.00`.
    pub igs: String,
    /// The radome's IGS code; `NONE` without one.
    #[serde(default = "no_radome")]
    pub radome: String,
    /// The serial number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    /// Where the calibration comes from, such as `igs20.atx`, if not the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration: Option<String>,
}

fn no_radome() -> String {
    "NONE".into()
}

impl Antenna {
    fn check(&self) -> Result<(), ModelError> {
        // IGS codes are upper case, at most 15 characters for the antenna
        // and exactly 4 for the radome (rcvr_ant.tab).
        let code = |s: &str| {
            s.bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b"-_.+/".contains(&b))
        };
        if self.igs.is_empty() || self.igs.len() > 15 || !code(&self.igs) {
            return Err(ModelError::Invalid(format!(
                "antenna {:?} isn't an IGS antenna code: up to 15 upper-case characters, as in rcvr_ant.tab (such as LEIAS10)",
                self.igs
            )));
        }
        if self.radome.len() != 4 || !code(&self.radome) {
            return Err(ModelError::Invalid(format!(
                "radome {:?} isn't an IGS radome code: 4 upper-case characters, or NONE",
                self.radome
            )));
        }
        Ok(())
    }
}

/// How an antenna height was measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HeightMethod {
    /// Vertically, from the mark to the measure-up point.
    Vertical,
    /// On the slant, from the mark to a point on the antenna's edge. It's
    /// converted to vertical with the antenna geometry catalogue.
    Slant,
}

/// An antenna height, as measured in the field, with its evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HeightMeasurement {
    /// Vertical or slant.
    pub method: HeightMethod,
    /// The point measured to, such as `ARP` or a slant point named in the
    /// antenna geometry catalogue.
    pub to: String,
    /// The height in metres.
    pub value: f64,
    /// Who measured it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured_by: Option<String>,
    /// Photos of the tape, in `fieldlogs/`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub photos: Vec<String>,
}

impl HeightMeasurement {
    fn check(&self) -> Result<(), ModelError> {
        if self.to.trim().is_empty() {
            return Err(ModelError::Invalid(
                "the height needs the point it was measured to (such as ARP)".into(),
            ));
        }
        if self.method == HeightMethod::Slant && self.to == "ARP" {
            return Err(ModelError::Invalid(
                "a slant height is measured to the antenna's edge, not the ARP; name the slant point".into(),
            ));
        }
        // Tripods and poles are a few metres; masts rarely more than 30.
        if !(self.value.is_finite() && (0.0..=30.0).contains(&self.value)) {
            return Err(ModelError::Invalid(format!(
                "antenna height {} m is implausible; heights are in metres",
                self.value
            )));
        }
        for photo in &self.photos {
            check_path(photo, "fieldlogs/")?;
        }
        Ok(())
    }
}

/// A path from the project folder, inside the given directory, written with
/// forward slashes and not leaving it.
pub(crate) fn check_path(path: &str, directory: &str) -> Result<(), ModelError> {
    let inside = path.strip_prefix(directory).is_some_and(|rest| {
        !rest.is_empty()
            && !path.contains('\\')
            && rest
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..")
    });
    if inside {
        Ok(())
    } else {
        Err(ModelError::Invalid(format!(
            "{path:?} must be a file in {directory}, such as {directory}name"
        )))
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{HeightMethod, Occupation};

    /// The example in ARCHITECTURE.md section 6.
    pub(crate) const EXAMPLE: &str = r#"
schema = 1
id = "2026-10-14-PM12345-A"
mark = "PM12345"
start = "2026-10-14T00:12:00Z"
end = "2026-10-14T02:31:30Z"
rinex = ["raw/PM12345A.rnx"]

[receiver]
make = "Leica"
model = "GS10"
serial = "1234567"

[antenna]
igs = "LEIAS10"
radome = "NONE"
serial = "7654321"

[height]
method = "vertical"
to = "ARP"
value = 1.5320           # metres
measured_by = "CL"
photos = ["fieldlogs/2026-10-14-PM12345-A-tape.jpg"]
"#;

    #[test]
    fn the_documented_example_reads_and_round_trips() {
        let occupation = Occupation::from_toml(EXAMPLE).unwrap();
        assert_eq!(occupation.mark.as_str(), "PM12345");
        assert_eq!(occupation.duration_s(), 2.0 * 3600.0 + 19.0 * 60.0 + 30.0);
        assert_eq!(occupation.height.method, HeightMethod::Vertical);
        assert_eq!(occupation.height.value, 1.532);
        assert_eq!(occupation.antenna.radome, "NONE");
        assert_eq!(
            Occupation::from_toml(&occupation.to_toml()).unwrap(),
            occupation
        );
    }

    #[test]
    fn the_radome_defaults_to_none() {
        let text = EXAMPLE.replace("radome = \"NONE\"\n", "");
        assert_eq!(Occupation::from_toml(&text).unwrap().antenna.radome, "NONE");
    }

    #[test]
    fn mistakes_are_explained() {
        let cases = [
            (EXAMPLE.replace("02:31:30Z", "00:11:00Z"), "ends"),
            (
                EXAMPLE.replace("raw/PM12345A.rnx", "PM12345A.rnx"),
                "must be a file in raw/",
            ),
            (
                EXAMPLE.replace("raw/PM12345A.rnx", "raw/../marks.toml"),
                "must be a file in raw/",
            ),
            (
                EXAMPLE.replace("raw/PM12345A.rnx", "raw\\\\PM.rnx"),
                "must be a file in raw/",
            ),
            (
                EXAMPLE.replace("\"LEIAS10\"", "\"leias10\""),
                "IGS antenna code",
            ),
            (EXAMPLE.replace("\"NONE\"", "\"NO\""), "IGS radome code"),
            (EXAMPLE.replace("1.5320", "1532.0"), "implausible"),
            (EXAMPLE.replace("1.5320", "-0.1"), "implausible"),
            (EXAMPLE.replace("\"vertical\"", "\"slant\""), "slant"),
            (
                EXAMPLE.replace("\"vertical\"", "\"diagonal\""),
                "unknown variant",
            ),
            (
                EXAMPLE.replace("make = \"Leica\"", "make = \"\""),
                "make and model",
            ),
            (EXAMPLE.replace("00:12:00Z", "00:12:00"), "isn't UTC"),
            (
                EXAMPLE.replace("[height]", "[height]\nunits = \"ft\""),
                "unknown field",
            ),
        ];
        for (text, expected) in cases {
            let err = Occupation::from_toml(&text).unwrap_err().to_string();
            assert!(err.contains(expected), "expected {expected:?} in {err:?}");
        }
    }
}
