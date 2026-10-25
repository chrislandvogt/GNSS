use gnss_datum::{CoordinateEpoch, FrameId};
use serde::{Deserialize, Serialize};

use crate::error::from_toml;
use crate::{MarkId, ModelError, SCHEMA};

/// `marks.toml`: the project's marks and their known coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkFile {
    /// The schema version: 1 (`SCHEMA`), raised only when a migration is needed.
    pub schema: u32,
    /// The marks, in the order written.
    #[serde(default, rename = "mark", skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<Mark>,
}

impl MarkFile {
    /// An empty file at the current schema.
    pub fn new() -> Self {
        Self {
            schema: SCHEMA,
            marks: Vec::new(),
        }
    }

    /// Reads `marks.toml`, checking every mark.
    pub fn from_toml(text: &str) -> Result<Self, ModelError> {
        let file: MarkFile = from_toml(text)?;
        for mark in &file.marks {
            mark.check()
                .map_err(|e| ModelError::Invalid(format!("mark {}: {e}", mark.id)))?;
        }
        Ok(file)
    }

    /// The file as TOML.
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// The mark with an id.
    pub fn get(&self, id: &MarkId) -> Option<&Mark> {
        self.marks.iter().find(|m| &m.id == id)
    }
}

impl Default for MarkFile {
    fn default() -> Self {
        Self::new()
    }
}

/// A physical survey mark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Mark {
    /// The mark's id, such as `PM12345`.
    pub id: MarkId,
    /// A name, if it has one besides its id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What kind of mark it is.
    #[serde(default)]
    pub kind: MarkKind,
    /// A rough position, used to check that a set-up was on the mark it
    /// claims (M9). Latitude and longitude in degrees, height in metres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approx: Option<[f64; 3]>,
    /// Coordinates known before this survey: control, each with its source.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub known: Vec<KnownCoordinate>,
}

impl Mark {
    /// A mark with only an id.
    pub fn new(id: MarkId) -> Self {
        Self {
            id,
            name: None,
            kind: MarkKind::default(),
            approx: None,
            known: Vec::new(),
        }
    }

    fn check(&self) -> Result<(), ModelError> {
        if let Some([lat, lon, h]) = self.approx {
            check_position("approx", lat, lon, h)?;
        }
        for known in &self.known {
            known.check()?;
        }
        Ok(())
    }
}

/// What kind of mark a mark is.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum MarkKind {
    /// A permanent mark, such as a PM or SSM.
    #[default]
    PermanentMark,
    /// A continuously operating reference station.
    Cors,
    /// A mark placed for this survey only.
    Temporary,
    /// Anything else.
    Other,
}

/// How a known coordinate takes part in the adjustment.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Constraint {
    /// Held fixed.
    Fixed,
    /// Weighted by its standard deviations.
    Weighted,
    /// A starting value only.
    #[default]
    Free,
}

/// A coordinate known before the survey, such as a control value from a
/// state database or an AUSPOS solution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnownCoordinate {
    /// The frame, such as `GDA2020` or `ITRF2014`.
    pub frame: String,
    /// The coordinate epoch as a decimal year, for a dynamic frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<f64>,
    /// Latitude in degrees, north positive.
    pub latitude: f64,
    /// Longitude in degrees, east positive.
    pub longitude: f64,
    /// Ellipsoidal height in metres.
    pub height: f64,
    /// Standard deviations east, north and up, in metres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sd: Option<[f64; 3]>,
    /// How the adjustment uses it.
    #[serde(default)]
    pub constraint: Constraint,
    /// Where the value came from, such as `SCIMS 2026-09-01` or `AUSPOS job 1234`.
    pub source: String,
}

impl KnownCoordinate {
    /// The frame, parsed.
    pub fn frame_id(&self) -> Result<FrameId, ModelError> {
        FrameId::parse(&self.frame).map_err(|e| ModelError::Invalid(e.to_string()))
    }

    /// The coordinate epoch: the one given for a dynamic frame, or a static
    /// frame's reference epoch.
    pub fn coordinate_epoch(&self) -> Result<CoordinateEpoch, ModelError> {
        let given = self
            .epoch
            .map(CoordinateEpoch::from_decimal_year)
            .transpose()
            .map_err(|e| ModelError::Invalid(e.to_string()))?;
        self.frame_id()?
            .resolve_epoch(given)
            .map_err(|e| ModelError::Invalid(e.to_string()))
    }

    fn check(&self) -> Result<(), ModelError> {
        self.coordinate_epoch()?;
        check_position(
            "known coordinate",
            self.latitude,
            self.longitude,
            self.height,
        )?;
        if let Some(sd) = self.sd
            && !sd.iter().all(|s| s.is_finite() && *s > 0.0)
        {
            return Err(ModelError::Invalid(format!(
                "known coordinate's standard deviations must be positive, got {sd:?}"
            )));
        }
        if self.constraint == Constraint::Weighted && self.sd.is_none() {
            return Err(ModelError::Invalid(
                "a weighted known coordinate needs standard deviations (sd = [east, north, up])"
                    .into(),
            ));
        }
        if self.source.trim().is_empty() {
            return Err(ModelError::Invalid(
                "a known coordinate needs its source".into(),
            ));
        }
        Ok(())
    }
}

fn check_position(what: &str, lat: f64, lon: f64, h: f64) -> Result<(), ModelError> {
    if !(lat.is_finite() && lon.is_finite() && h.is_finite()) {
        return Err(ModelError::Invalid(format!(
            "{what} has a non-finite value"
        )));
    }
    if lat.abs() > 90.0 || lon.abs() > 180.0 {
        return Err(ModelError::Invalid(format!(
            "{what} is out of range: latitude {lat}, longitude {lon} (degrees)"
        )));
    }
    // Lower than the deepest mine, higher than any mountain or most aircraft.
    if !(-11_000.0..=20_000.0).contains(&h) {
        return Err(ModelError::Invalid(format!(
            "{what}'s height {h} m is implausible; heights are in metres"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{Constraint, MarkFile, MarkKind};
    use crate::{MarkId, ModelError};

    const MARKS: &str = r#"
schema = 1

[[mark]]
id = "BEEC"
kind = "cors"
approx = [-36.3464, 146.6577, 443]

[[mark.known]]
frame = "GDA2020"
latitude = -36.346434060
longitude = 146.657743009
height = 442.9339
constraint = "fixed"
source = "GA Recognised Value Standard 2026"

[[mark]]
id = "PM12345"
name = "Bright PM"
"#;

    #[test]
    fn marks_round_trip() {
        let file = MarkFile::from_toml(MARKS).unwrap();
        assert_eq!(file.marks.len(), 2);
        let beec = file.get(&MarkId::new("BEEC").unwrap()).unwrap();
        assert_eq!(beec.kind, MarkKind::Cors);
        assert_eq!(beec.known[0].constraint, Constraint::Fixed);
        assert_eq!(
            beec.known[0].coordinate_epoch().unwrap().decimal_year(),
            2020.0
        );
        let pm = &file.marks[1];
        assert_eq!(pm.kind, MarkKind::PermanentMark);
        assert!(pm.known.is_empty());
        assert_eq!(MarkFile::from_toml(&file.to_toml()).unwrap(), file);
    }

    #[test]
    fn mistakes_are_explained() {
        let cases = [
            (
                MARKS.replace("442.9339", "442.9339\nsd = [0.01, 0.01, 0.02]\nweight = 1"),
                "unknown field `weight`",
            ),
            (
                MARKS.replace("constraint = \"fixed\"", "constraint = \"weighted\""),
                "needs standard deviations",
            ),
            (MARKS.replace("\"GDA2020\"", "\"ITRF2014\""), "epoch"),
            (
                MARKS.replace("frame = \"GDA2020\"", "frame = \"GDA2020\"\nepoch = 2026.5"),
                "epoch",
            ),
            (MARKS.replace("-36.346434060", "-136.3"), "out of range"),
            (MARKS.replace("442.9339", "442933.9"), "implausible"),
            (MARKS.replace("\"BEEC\"", "\"BE EC\""), "mark id"),
            (
                MARKS.replace("\"GA Recognised Value Standard 2026\"", "\" \""),
                "source",
            ),
        ];
        for (text, expected) in cases {
            let err = MarkFile::from_toml(&text).unwrap_err().to_string();
            assert!(err.contains(expected), "expected {expected:?} in {err:?}");
        }
    }

    #[test]
    fn a_newer_schema_says_so() {
        let text = MARKS.replace("schema = 1", "schema = 2\nnew_thing = true");
        assert_eq!(
            MarkFile::from_toml(&text).unwrap_err(),
            ModelError::NewerSchema {
                found: 2,
                supported: 1
            }
        );
    }
}
