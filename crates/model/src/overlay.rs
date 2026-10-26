use serde::{Deserialize, Serialize};

use crate::error::from_toml;
use crate::{ModelError, SCHEMA, Timestamp};

/// A recorded correction to a raw file: a file in `overlays/`. Raw files are
/// never edited (ARCHITECTURE.md section 6), so a wrong value in one, such as
/// an antenna type or height in a RINEX header, is corrected here, with who
/// made the correction, when and why. RINEX ingest (M6) applies overlays in
/// order of their ids, and the report lists every one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Overlay {
    /// The schema version: 1 (`SCHEMA`), raised only when a migration is needed.
    pub schema: u32,
    /// The overlay's id, which is also its file name, such as
    /// `2026-10-15-PM12345A-antenna`.
    pub id: crate::OverlayId,
    /// The raw file corrected, in `raw/`.
    pub file: String,
    /// Why: the evidence for the correction, such as the field log.
    pub reason: String,
    /// Who made the correction.
    pub by: String,
    /// When it was recorded, in UTC.
    pub recorded: Timestamp,
    /// Corrections to a RINEX file's header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rinex: Option<RinexHeader>,
}

/// Values that replace those in a RINEX header. Only the values given are
/// replaced.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RinexHeader {
    /// MARKER NAME.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker_name: Option<String>,
    /// The receiver type in REC # / TYPE / VERS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver: Option<String>,
    /// The antenna's IGS code in ANT # / TYPE.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub antenna: Option<String>,
    /// The radome's IGS code in ANT # / TYPE.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radome: Option<String>,
    /// ANTENNA: DELTA H/E/N: height, east and north eccentricities of the
    /// antenna reference point from the mark, in metres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub antenna_delta: Option<[f64; 3]>,
}

impl Overlay {
    /// Reads an overlay file, checking it.
    pub fn from_toml(text: &str) -> Result<Self, ModelError> {
        let overlay: Overlay = from_toml(text)?;
        overlay
            .check()
            .map_err(|e| ModelError::Invalid(format!("overlay {}: {e}", overlay.id)))?;
        Ok(overlay)
    }

    /// The overlay as TOML.
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    fn check(&self) -> Result<(), ModelError> {
        if self.schema != SCHEMA {
            return Err(ModelError::Invalid(format!(
                "schema {} isn't one this version reads ({SCHEMA})",
                self.schema
            )));
        }
        crate::occupation::check_path(&self.file, "raw/")?;
        if self.reason.trim().is_empty() {
            return Err(ModelError::Invalid(
                "a correction needs its reason: the evidence for it".into(),
            ));
        }
        if self.by.trim().is_empty() {
            return Err(ModelError::Invalid("a correction needs who made it".into()));
        }
        let Some(rinex) = &self.rinex else {
            return Err(ModelError::Invalid(
                "it corrects nothing; add a [rinex] table with the values to replace".into(),
            ));
        };
        if *rinex == RinexHeader::default() {
            return Err(ModelError::Invalid(
                "its [rinex] table is empty; give the values to replace".into(),
            ));
        }
        if let Some(delta) = rinex.antenna_delta
            && !(delta.iter().all(|d| d.is_finite())
                && (0.0..=30.0).contains(&delta[0])
                && delta[1].abs() <= 5.0
                && delta[2].abs() <= 5.0)
        {
            return Err(ModelError::Invalid(format!(
                "antenna_delta {delta:?} is implausible: height 0 to 30 m, east and north within 5 m"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::Overlay;

    const EXAMPLE: &str = r#"
schema = 1
id = "2026-10-15-PM12345A-antenna"
file = "raw/PM12345A.rnx"
reason = "The field log and tape photo show an AS10 at 1.532 m to the ARP; the receiver's header had the default antenna and no height"
by = "CL"
recorded = "2026-10-15T03:00:00Z"

[rinex]
antenna = "LEIAS10"
radome = "NONE"
antenna_delta = [1.532, 0.0, 0.0]
"#;

    #[test]
    fn overlays_read_and_round_trip() {
        let overlay = Overlay::from_toml(EXAMPLE).unwrap();
        let rinex = overlay.rinex.as_ref().unwrap();
        assert_eq!(rinex.antenna.as_deref(), Some("LEIAS10"));
        assert_eq!(rinex.marker_name, None);
        assert_eq!(Overlay::from_toml(&overlay.to_toml()).unwrap(), overlay);
    }

    #[test]
    fn a_correction_needs_its_reason_and_a_change() {
        let cases = [
            (EXAMPLE.replace("raw/PM12345A.rnx", "occupations/A.toml"), "must be a file in raw/"),
            (
                EXAMPLE.replace(
                    "reason = \"The field log and tape photo show an AS10 at 1.532 m to the ARP; the receiver's header had the default antenna and no height\"",
                    "reason = \"\"",
                ),
                "needs its reason",
            ),
            (EXAMPLE.replace("by = \"CL\"", "by = \" \""), "who made it"),
            (
                EXAMPLE[..EXAMPLE.find("[rinex]").unwrap()].to_string(),
                "corrects nothing",
            ),
            (
                EXAMPLE[..EXAMPLE.find("antenna = ").unwrap()].to_string(),
                "is empty",
            ),
            (EXAMPLE.replace("[1.532, 0.0, 0.0]", "[1532.0, 0.0, 0.0]"), "implausible"),
            (EXAMPLE.replace("radome = \"NONE\"", "radome = \"NONE\"\nheight = 1.5"), "unknown field"),
        ];
        for (text, expected) in cases {
            let err = Overlay::from_toml(&text).unwrap_err().to_string();
            assert!(err.contains(expected), "expected {expected:?} in {err:?}");
        }
    }
}
