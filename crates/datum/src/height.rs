//! Heights, typed by what they're measured from (CONVENTIONS.md section 3).
//! There's no untyped height: each says whether it's above an ellipsoid or a
//! height datum, and which model, if any, it was computed through.

use std::fmt;

use crate::frame::FrameId;

/// What a height is measured from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeightKind {
    /// Above a frame's ellipsoid, along the normal.
    Ellipsoidal(FrameId),
    /// Above the Australian Height Datum (AHD71).
    Ahd,
}

impl fmt::Display for HeightKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeightKind::Ellipsoidal(frame) => write!(f, "ellipsoidal height ({frame})"),
            HeightKind::Ahd => f.write_str("AHD height"),
        }
    }
}

/// A height, with what it's measured from and how it was obtained.
#[derive(Debug, Clone, PartialEq)]
pub struct Height {
    /// The height, in metres.
    pub value_m: f64,
    /// One standard deviation, in metres, if every uncertainty that went
    /// into the height is known.
    pub sigma_m: Option<f64>,
    /// What it's measured from.
    pub kind: HeightKind,
    /// The model it was computed through, for example "AUSGeoid2020
    /// (20180201)", or `None` for a height that was measured or given directly.
    pub model: Option<String>,
}

impl Height {
    /// An AHD height as given, for example from levelling or a survey control
    /// database.
    pub fn ahd(value_m: f64, sigma_m: Option<f64>) -> Self {
        Height {
            value_m,
            sigma_m,
            kind: HeightKind::Ahd,
            model: None,
        }
    }
}

impl fmt::Display for Height {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.kind {
            HeightKind::Ellipsoidal(frame) => format!("{frame} ellipsoidal"),
            HeightKind::Ahd => "AHD".to_owned(),
        };
        write!(f, "{:.4} m {what}", self.value_m)?;
        if let Some(model) = &self.model {
            write!(f, " via {model}")?;
        }
        if let Some(sigma) = self.sigma_m {
            write!(f, ", sigma {sigma:.4} m")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn heights_say_what_they_are() {
        assert_eq!(Height::ahd(12.3456, None).to_string(), "12.3456 m AHD");
        let h = Height {
            value_m: 31.7,
            sigma_m: Some(0.0841),
            kind: HeightKind::Ellipsoidal(FrameId::Gda2020),
            model: Some("AUSGeoid2020 (20180201)".to_owned()),
        };
        assert_eq!(
            h.to_string(),
            "31.7000 m GDA2020 ellipsoidal via AUSGeoid2020 (20180201), sigma 0.0841 m"
        );
        assert_eq!(HeightKind::Ahd.to_string(), "AHD height");
    }
}
