//! The frame registry: the reference frames this crate knows and how they behave.

use std::fmt;

use crate::ellipsoid::{Ellipsoid, GRS80, WGS84};
use crate::epoch::CoordinateEpoch;
use crate::error::DatumError;

/// A reference frame (EPSG: a geocentric CRS).
///
/// Dynamic frames move with the Earth's crust, so a position in one is only
/// meaningful with a coordinate epoch. Static frames are fixed to a plate at a
/// reference epoch. WGS 84 realisations are kept as frames of their own, so a
/// position remembers what it was given in, and are tied to the ITRF frames
/// they're aligned with by identity transformations with a stated tolerance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum FrameId {
    /// International Terrestrial Reference Frame 2020 (EPSG:9988).
    Itrf2020,
    /// IGS20, the International GNSS Service's realisation of ITRF2020 (EPSG:10176).
    Igs20,
    /// International Terrestrial Reference Frame 2014 (EPSG:7789).
    Itrf2014,
    /// International Terrestrial Reference Frame 2008 (EPSG:5332).
    Itrf2008,
    /// International Terrestrial Reference Frame 2005 (EPSG:4896).
    Itrf2005,
    /// Australian Terrestrial Reference Frame 2014 (EPSG:9307), aligned with ITRF2014.
    Atrf2014,
    /// Geocentric Datum of Australia 2020 (EPSG:7842), static at 2020.0.
    Gda2020,
    /// Geocentric Datum of Australia 1994 (EPSG:4938), static at 1994.0.
    Gda94,
    /// WGS 84 (G2296) (EPSG:10604), aligned with ITRF2020.
    Wgs84G2296,
    /// WGS 84 (G2139) (EPSG:9753), aligned with ITRF2014.
    Wgs84G2139,
    /// WGS 84 without a stated realisation: the EPSG datum ensemble (EPSG:4978), good to about 2 m.
    Wgs84,
}

/// Whether a frame's coordinates move with time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameKind {
    /// Coordinates change with time, so they need a coordinate epoch.
    Dynamic,
    /// Coordinates are fixed at the frame's reference epoch.
    Static {
        /// The epoch the frame's coordinates refer to.
        reference_epoch: CoordinateEpoch,
    },
}

impl FrameId {
    /// Every frame, in a fixed order.
    pub const ALL: [FrameId; 11] = [
        FrameId::Itrf2020,
        FrameId::Igs20,
        FrameId::Itrf2014,
        FrameId::Itrf2008,
        FrameId::Itrf2005,
        FrameId::Atrf2014,
        FrameId::Gda2020,
        FrameId::Gda94,
        FrameId::Wgs84G2296,
        FrameId::Wgs84G2139,
        FrameId::Wgs84,
    ];

    /// The frame's name, as EPSG writes it.
    pub const fn name(self) -> &'static str {
        match self {
            FrameId::Itrf2020 => "ITRF2020",
            FrameId::Igs20 => "IGS20",
            FrameId::Itrf2014 => "ITRF2014",
            FrameId::Itrf2008 => "ITRF2008",
            FrameId::Itrf2005 => "ITRF2005",
            FrameId::Atrf2014 => "ATRF2014",
            FrameId::Gda2020 => "GDA2020",
            FrameId::Gda94 => "GDA94",
            FrameId::Wgs84G2296 => "WGS 84 (G2296)",
            FrameId::Wgs84G2139 => "WGS 84 (G2139)",
            FrameId::Wgs84 => "WGS 84",
        }
    }

    /// The EPSG code of the frame's geocentric (ECEF) CRS.
    pub const fn epsg_code(self) -> u32 {
        match self {
            FrameId::Itrf2020 => 9988,
            FrameId::Igs20 => 10176,
            FrameId::Itrf2014 => 7789,
            FrameId::Itrf2008 => 5332,
            FrameId::Itrf2005 => 4896,
            FrameId::Atrf2014 => 9307,
            FrameId::Gda2020 => 7842,
            FrameId::Gda94 => 4938,
            FrameId::Wgs84G2296 => 10604,
            FrameId::Wgs84G2139 => 9753,
            FrameId::Wgs84 => 4978,
        }
    }

    /// Whether the frame is static or dynamic.
    pub const fn kind(self) -> FrameKind {
        match self {
            FrameId::Gda2020 => FrameKind::Static {
                reference_epoch: CoordinateEpoch::from_const(2020.0),
            },
            FrameId::Gda94 => FrameKind::Static {
                reference_epoch: CoordinateEpoch::from_const(1994.0),
            },
            _ => FrameKind::Dynamic,
        }
    }

    /// True for static frames.
    pub const fn is_static(self) -> bool {
        matches!(self.kind(), FrameKind::Static { .. })
    }

    /// A static frame's reference epoch, or `None` for a dynamic frame.
    pub const fn reference_epoch(self) -> Option<CoordinateEpoch> {
        match self.kind() {
            FrameKind::Static { reference_epoch } => Some(reference_epoch),
            FrameKind::Dynamic => None,
        }
    }

    /// The coordinate epoch of a position in this frame: the epoch given, for
    /// a dynamic frame, which needs one, or the reference epoch, for a static
    /// frame, which accepts no other.
    pub fn resolve_epoch(
        self,
        epoch: Option<CoordinateEpoch>,
    ) -> Result<CoordinateEpoch, DatumError> {
        match (self.kind(), epoch) {
            (FrameKind::Dynamic, Some(e)) => Ok(e),
            (FrameKind::Dynamic, None) => Err(DatumError::MissingEpoch { frame: self }),
            (FrameKind::Static { reference_epoch }, None) => Ok(reference_epoch),
            (FrameKind::Static { reference_epoch }, Some(e)) if e == reference_epoch => Ok(e),
            (FrameKind::Static { reference_epoch }, Some(given)) => {
                Err(DatumError::EpochOnStaticFrame {
                    frame: self,
                    reference: reference_epoch,
                    given,
                })
            }
        }
    }

    /// The ellipsoid used for the frame's geodetic coordinates.
    pub const fn ellipsoid(self) -> Ellipsoid {
        match self {
            FrameId::Wgs84G2296 | FrameId::Wgs84G2139 | FrameId::Wgs84 => WGS84,
            _ => GRS80,
        }
    }

    /// A frame from its name or EPSG code.
    ///
    /// Accepts the names from [`FrameId::name`] in any case and with or
    /// without spaces (`"gda2020"`, `"WGS84 (G2139)"`), and `"EPSG:7842"`.
    pub fn parse(text: &str) -> Result<FrameId, DatumError> {
        let wanted = squash(text);
        FrameId::ALL
            .into_iter()
            .find(|f| squash(f.name()) == wanted || wanted == format!("EPSG:{}", f.epsg_code()))
            .ok_or_else(|| DatumError::UnknownFrame {
                name: text.to_owned(),
                known: FrameId::ALL.map(FrameId::name).join(", "),
            })
    }
}

/// Upper case without spaces, for forgiving name matching.
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

impl fmt::Display for FrameId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn static_and_dynamic_frames() {
        assert_eq!(
            FrameId::Gda2020
                .reference_epoch()
                .map(CoordinateEpoch::decimal_year),
            Some(2020.0)
        );
        assert_eq!(
            FrameId::Gda94
                .reference_epoch()
                .map(CoordinateEpoch::decimal_year),
            Some(1994.0)
        );
        for f in [
            FrameId::Itrf2020,
            FrameId::Igs20,
            FrameId::Itrf2014,
            FrameId::Atrf2014,
        ] {
            assert!(!f.is_static(), "{f}");
        }
    }

    #[test]
    fn parses_names_and_epsg_codes() {
        assert_eq!(FrameId::parse("gda2020").unwrap(), FrameId::Gda2020);
        assert_eq!(
            FrameId::parse("WGS84 (G2139)").unwrap(),
            FrameId::Wgs84G2139
        );
        assert_eq!(FrameId::parse("EPSG:9988").unwrap(), FrameId::Itrf2020);
        assert_eq!(FrameId::parse(" atrf2014 ").unwrap(), FrameId::Atrf2014);
        let err = FrameId::parse("AGD66").unwrap_err().to_string();
        assert!(err.contains("AGD66") && err.contains("GDA2020"), "{err}");
    }

    #[test]
    fn names_and_codes_are_unique() {
        for (i, a) in FrameId::ALL.iter().enumerate() {
            for b in &FrameId::ALL[i + 1..] {
                assert_ne!(a.name(), b.name());
                assert_ne!(a.epsg_code(), b.epsg_code());
            }
        }
    }
}
