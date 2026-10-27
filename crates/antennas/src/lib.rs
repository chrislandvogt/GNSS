//! The antenna catalogue (ARCHITECTURE.md section 6): what the IGS says
//! about each antenna, and the geometry for converting slant heights.
//!
//! From the IGS (`data/antennas`, refreshed by `update.py` there):
//! - the antenna and radome codes, with descriptions (`rcvr_ant.json`);
//! - each antenna's antenna reference point (ARP) and north reference
//!   point, and its drawing with dimensions in metres (`antenna.gra`);
//! - which antenna and radome pairs have an IGS calibration
//!   (`calibrated.txt`, from `igs20.atx`).
//!
//! Slant-height geometry isn't in the IGS files, so it comes from
//! `slant.toml`, where every entry cites its source. Where it has none for
//! an antenna, a slant height can't be converted, and [`vertical_from_slant`]
//! says so rather than guessing.

#![cfg_attr(not(test), warn(clippy::indexing_slicing))]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use serde::Deserialize;

const RCVR_ANT: &str = include_str!("../../../data/antennas/rcvr_ant.json");
const ANTENNA_GRA: &str = include_str!("../../../data/antennas/antenna.gra");
const CALIBRATED: &str = include_str!("../../../data/antennas/calibrated.txt");
const SLANT: &str = include_str!("../../../data/antennas/slant.toml");
const SOURCES: &str = include_str!("../../../data/antennas/sources.toml");

/// Why a catalogue file couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogueError {
    /// A file isn't in the form expected.
    #[error("{file}: {message}")]
    Format {
        /// The file.
        file: &'static str,
        /// What's wrong.
        message: String,
    },
}

fn format_error(file: &'static str, message: impl Into<String>) -> CatalogueError {
    CatalogueError::Format {
        file,
        message: message.into(),
    }
}

/// A feature of an antenna, by its IGS abbreviation, such as `BPA`, the
/// bottom of the preamplifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feature {
    /// The abbreviation.
    pub code: String,
    /// What it means, from `antenna.gra`, if it defines it.
    pub meaning: Option<String>,
}

/// What the IGS says about one antenna.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AntennaInfo {
    /// The IGS code, such as `LEIAS10`.
    pub code: String,
    /// Its description.
    pub description: String,
    /// The feature that is its antenna reference point, to which heights
    /// and calibrations refer, if `antenna.gra` has it.
    pub arp: Option<Feature>,
    /// The feature to point north, if `antenna.gra` has it.
    pub north: Option<Feature>,
    /// Its drawing from `antenna.gra`, with dimensions in metres, if any.
    pub drawing: Option<String>,
}

/// Where a slant height is measured to on an antenna, and where that point
/// is relative to the ARP.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlantGeometry {
    /// The IGS antenna code.
    pub igs: String,
    /// The slant point's name, as an occupation's height names it.
    pub point: String,
    /// Metres, horizontally from the antenna's axis to the slant point.
    pub radius: f64,
    /// Metres, vertically from the ARP up to the slant point.
    pub above_arp: f64,
    /// Where these numbers come from.
    pub source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SlantFile {
    schema: u32,
    #[serde(default)]
    antenna: Vec<SlantGeometry>,
}

#[derive(Deserialize)]
struct RcvrAnt {
    #[serde(rename = "Antenna")]
    antenna: BTreeMap<String, String>,
    #[serde(rename = "Dome")]
    dome: BTreeMap<String, String>,
}

/// The antenna catalogue.
#[derive(Debug, Clone, PartialEq)]
pub struct Catalogue {
    antennas: BTreeMap<String, AntennaInfo>,
    radomes: BTreeMap<String, String>,
    calibrated: BTreeSet<(String, String)>,
    slant: Vec<SlantGeometry>,
    fetched: Option<String>,
}

impl Catalogue {
    /// The catalogue built into this version, from the IGS files in
    /// `data/antennas`.
    pub fn igs() -> &'static Catalogue {
        static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();
        CATALOGUE.get_or_init(|| {
            // The files are part of the build, and the tests read them, so
            // this can't fail in a released build.
            #[allow(clippy::expect_used)]
            Catalogue::parse(RCVR_ANT, ANTENNA_GRA, CALIBRATED, SLANT)
                .map(|c| c.with_date(SOURCES))
                .expect("the built-in antenna catalogue reads")
        })
    }

    /// A catalogue from the IGS files' contents and the slant table.
    pub fn parse(
        rcvr_ant_json: &str,
        antenna_gra: &str,
        calibrated: &str,
        slant_toml: &str,
    ) -> Result<Self, CatalogueError> {
        let codes: RcvrAnt = serde_json::from_str(rcvr_ant_json)
            .map_err(|e| format_error("rcvr_ant.json", e.to_string()))?;
        let gra = Gra::parse(antenna_gra, |code| codes.antenna.contains_key(code))?;
        let mut antennas = BTreeMap::new();
        for (code, description) in codes.antenna {
            let feature = |c: Option<&String>| {
                c.map(|c| Feature {
                    code: c.clone(),
                    meaning: gra.meanings.get(c).cloned(),
                })
            };
            let points = gra.points.get(&code);
            antennas.insert(
                code.clone(),
                AntennaInfo {
                    arp: feature(points.map(|p| &p.0)),
                    north: feature(points.map(|p| &p.1)),
                    drawing: gra.drawings.get(&code).cloned(),
                    code,
                    description,
                },
            );
        }
        for code in gra.points.keys() {
            if !antennas.contains_key(code) {
                return Err(format_error(
                    "antenna.gra",
                    format!("{code} isn't an antenna in rcvr_ant.json"),
                ));
            }
        }
        let calibrated = calibrated
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .map(|l| {
                let (antenna, radome) = l.split_at(l.len().min(16));
                (antenna.trim().to_owned(), radome.trim().to_owned())
            })
            .collect();
        let slant: SlantFile =
            toml::from_str(slant_toml).map_err(|e| format_error("slant.toml", e.to_string()))?;
        if slant.schema != 1 {
            return Err(format_error(
                "slant.toml",
                format!("schema {} isn't one this version reads (1)", slant.schema),
            ));
        }
        for g in &slant.antenna {
            if !antennas.contains_key(&g.igs) {
                return Err(format_error(
                    "slant.toml",
                    format!("{} isn't an IGS antenna code", g.igs),
                ));
            }
            let plausible = |v: f64| v.is_finite() && v.abs() < 1.0;
            if !plausible(g.radius) || g.radius <= 0.0 || !plausible(g.above_arp) {
                return Err(format_error(
                    "slant.toml",
                    format!(
                        "{}: the radius and offset should be metres, under 1 m",
                        g.igs
                    ),
                ));
            }
            if g.source.trim().is_empty() || g.point.trim().is_empty() {
                return Err(format_error(
                    "slant.toml",
                    format!("{}: every entry needs its point and its source", g.igs),
                ));
            }
        }
        Ok(Self {
            antennas,
            radomes: codes.dome,
            calibrated,
            slant: slant.antenna,
            fetched: None,
        })
    }

    fn with_date(mut self, sources: &str) -> Self {
        self.fetched = sources
            .lines()
            .find_map(|l| l.strip_prefix("fetched = \""))
            .and_then(|l| l.strip_suffix('"'))
            .map(str::to_owned);
        self
    }

    /// When the IGS files were fetched, as `2026-10-27`, if known.
    pub fn fetched(&self) -> Option<&str> {
        self.fetched.as_deref()
    }

    /// An antenna, by its IGS code.
    pub fn antenna(&self, code: &str) -> Option<&AntennaInfo> {
        self.antennas.get(code)
    }

    /// Every antenna, in code order.
    pub fn antennas(&self) -> impl Iterator<Item = &AntennaInfo> {
        self.antennas.values()
    }

    /// A radome's description, by its IGS code (`NONE` for none).
    pub fn radome(&self, code: &str) -> Option<&str> {
        self.radomes.get(code).map(String::as_str)
    }

    /// Whether the IGS calibrates this antenna with this radome.
    pub fn is_calibrated(&self, antenna: &str, radome: &str) -> bool {
        self.calibrated
            .contains(&(antenna.to_owned(), radome.to_owned()))
    }

    /// An antenna's slant point, if the catalogue has its geometry.
    pub fn slant(&self, antenna: &str, point: &str) -> Option<&SlantGeometry> {
        self.slant
            .iter()
            .find(|g| g.igs == antenna && g.point == point)
    }

    /// Checks an antenna and radome pair, as an occupation records it: the
    /// problem, if there is one.
    pub fn check(&self, antenna: &str, radome: &str) -> Option<String> {
        let when = self
            .fetched()
            .map(|d| format!(" (IGS list of {d})"))
            .unwrap_or_default();
        if self.antenna(antenna).is_none() {
            return Some(format!(
                "antenna {antenna:?} isn't an IGS antenna code{when}; check the spelling against the antenna's label or the IGS rcvr_ant.tab"
            ));
        }
        if self.radome(radome).is_none() {
            return Some(format!(
                "radome {radome:?} isn't an IGS radome code{when}; use NONE if there's no radome"
            ));
        }
        None
    }
}

/// Why a slant height couldn't be converted.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SlantError {
    /// The catalogue has no geometry for this antenna and point.
    #[error(
        "the antenna catalogue has no slant geometry for {antenna} to {point}; measure vertically to the ARP, or add the antenna's geometry with its source (data/antennas/README.md)"
    )]
    NoGeometry {
        /// The antenna.
        antenna: String,
        /// The slant point.
        point: String,
    },
    /// The slant height is shorter than the point's radius.
    #[error("a slant height of {slant} m can't reach a point {radius} m from the antenna's axis")]
    TooShort {
        /// The slant height.
        slant: f64,
        /// The point's radius.
        radius: f64,
    },
}

/// The vertical height of the ARP above the mark from a slant height to an
/// antenna's slant point: √(slant² − radius²) − above_arp.
pub fn vertical_from_slant(
    catalogue: &Catalogue,
    antenna: &str,
    point: &str,
    slant: f64,
) -> Result<f64, SlantError> {
    let g = catalogue
        .slant(antenna, point)
        .ok_or_else(|| SlantError::NoGeometry {
            antenna: antenna.to_owned(),
            point: point.to_owned(),
        })?;
    if !(slant.is_finite() && slant > g.radius) {
        return Err(SlantError::TooShort {
            slant,
            radius: g.radius,
        });
    }
    Ok((slant * slant - g.radius * g.radius).sqrt() - g.above_arp)
}

/// What's read from `antenna.gra`.
struct Gra {
    /// Abbreviation to meaning, such as `BPA` to `bottom of preamplifier`.
    meanings: BTreeMap<String, String>,
    /// Antenna to (ARP feature, north reference feature).
    points: BTreeMap<String, (String, String)>,
    /// Antenna to drawing.
    drawings: BTreeMap<String, String>,
}

const QUICK_REFERENCE: &str = "Machine-readable quick reference section begins here.";

impl Gra {
    /// Reads the file; `known` says whether a word is an IGS antenna code,
    /// to find the lines that name the drawings.
    fn parse(text: &str, known: impl Fn(&str) -> bool) -> Result<Self, CatalogueError> {
        let (body, quick) = text.split_once(QUICK_REFERENCE).ok_or_else(|| {
            format_error(
                "antenna.gra",
                "there's no machine-readable quick reference section",
            )
        })?;
        let mut points = BTreeMap::new();
        for line in quick.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if let [code, arp, north] = fields.as_slice() {
                points.insert((*code).to_owned(), ((*arp).to_owned(), (*north).to_owned()));
            }
        }
        if points.is_empty() {
            return Err(format_error(
                "antenna.gra",
                "the quick reference lists no antennas",
            ));
        }

        // The abbreviations, as `XXX: meaning`, before the drawings start.
        let mut meanings = BTreeMap::new();
        let head = body.split_once("\nDimensions\n").map_or(body, |(h, _)| h);
        for line in head.lines() {
            if let Some((code, meaning)) = line.split_once(": ")
                && code.len() == 3
                && code.bytes().all(|b| b.is_ascii_uppercase())
            {
                meanings
                    .entry(code.to_owned())
                    .or_insert_with(|| meaning.trim().to_owned());
            }
        }

        // Drawings: one or more lines naming antennas, each alone on its
        // line, then the drawing they share, up to the next names.
        let mut drawings = BTreeMap::new();
        let mut names: Vec<&str> = Vec::new();
        let mut drawing: Vec<&str> = Vec::new();
        let mut finish = |names: &mut Vec<&str>, drawing: &mut Vec<&str>| {
            while drawing.last().is_some_and(|l| l.trim().is_empty()) {
                drawing.pop();
            }
            let start = drawing.iter().position(|l| !l.trim().is_empty());
            let text = start
                .and_then(|s| drawing.get(s..))
                .map(|lines| lines.join("\n"))
                .unwrap_or_default();
            if !text.trim().is_empty() {
                for name in names.iter() {
                    drawings
                        .entry((*name).to_owned())
                        .or_insert_with(|| text.clone());
                }
            }
            names.clear();
            drawing.clear();
        };
        for line in body.lines() {
            // A name may carry the radome drawn with it, as in
            // `TRM115000.00+S  SCIT`.
            let name = line
                .split_whitespace()
                .next()
                .filter(|n| !line.starts_with(' ') && known(n))
                .filter(|_| {
                    line.split_whitespace()
                        .nth(1)
                        .is_none_or(|r| r.len() == 4 && line.split_whitespace().nth(2).is_none())
                });
            if let Some(name) = name {
                if !drawing.iter().all(|l| l.trim().is_empty()) {
                    finish(&mut names, &mut drawing);
                }
                drawing.clear();
                names.push(name);
            } else if !names.is_empty() {
                drawing.push(line);
            }
        }
        // The last drawing runs up to a line of dashes before the quick
        // reference.
        while drawing
            .last()
            .is_some_and(|l| l.trim().is_empty() || l.starts_with("----"))
        {
            drawing.pop();
        }
        finish(&mut names, &mut drawing);
        Ok(Self {
            meanings,
            points,
            drawings,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    use super::{Catalogue, SlantError, vertical_from_slant};

    #[test]
    fn the_built_in_catalogue_reads_the_igs_files() {
        let c = Catalogue::igs();
        assert_eq!(c.fetched(), Some("2026-10-27"));
        assert!(c.antennas().count() > 500);
        let as10 = c.antenna("LEIAS10").unwrap();
        assert_eq!(as10.arp.as_ref().unwrap().code, "BAM");
        assert_eq!(
            as10.arp.as_ref().unwrap().meaning.as_deref(),
            Some("bottom of antenna mount")
        );
        assert_eq!(as10.north.as_ref().unwrap().code, "RXC");
        let drawing = as10.drawing.as_deref().unwrap();
        assert!(drawing.contains("BAM=ARP"), "{drawing}");
        assert!(drawing.contains("0.1700"), "{drawing}");
        // Antennas named together share their drawing.
        assert_eq!(
            c.antenna("LEIAS05").unwrap().drawing.as_deref(),
            Some(drawing)
        );
        let tpsg5 = c.antenna("TRM59800.00").unwrap();
        assert_eq!(tpsg5.arp.as_ref().unwrap().code, "BPA");
        assert!(tpsg5.drawing.as_deref().unwrap().contains("BPA=ARP"));
        assert!(c.radome("NONE").is_some());
        assert!(c.radome("SCIS").is_some());
        assert!(c.is_calibrated("LEIAS10", "NONE"));
        assert!(!c.is_calibrated("LEIAS10", "SCIS"));
    }

    #[test]
    fn the_igs_files_are_as_fetched() {
        use sha2::{Digest, Sha256};
        let recorded = |name: &str| {
            let block = super::SOURCES
                .split("[[file]]")
                .find(|b| b.contains(&format!("name = \"{name}\"")))
                .unwrap();
            block
                .lines()
                .find_map(|l| l.strip_prefix("sha256 = \""))
                .unwrap()
                .trim_end_matches('"')
                .to_owned()
        };
        for (name, text) in [
            ("rcvr_ant.json", super::RCVR_ANT),
            ("antenna.gra", super::ANTENNA_GRA),
        ] {
            let hash: String = Sha256::digest(text.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert_eq!(
                hash,
                recorded(name),
                "{name} differs from the IGS file update.py recorded"
            );
        }
    }

    #[test]
    fn every_quick_reference_antenna_has_its_drawing() {
        let c = Catalogue::igs();
        let missing: Vec<&str> = c
            .antennas()
            .filter(|a| a.arp.is_some() && a.drawing.is_none())
            .map(|a| a.code.as_str())
            .collect();
        // antenna.gra lists TNVT20PRO in its quick reference, but has no
        // drawing for it.
        assert_eq!(missing, ["TNVT20PRO"], "antennas without a drawing");
        let s = c.antenna("TRM115000.00+S").unwrap();
        assert!(s.drawing.as_deref().unwrap().contains("BPA=ARP"));
        // A drawing never runs into the next antenna's names or the quick
        // reference.
        for a in c.antennas() {
            if let Some(d) = &a.drawing {
                assert!(!d.contains("Machine-readable"), "{}", a.code);
                let names_another = |l: &str| {
                    !l.starts_with(' ')
                        && l.split_whitespace()
                            .next()
                            .is_some_and(|n| c.antenna(n).is_some())
                };
                assert!(!d.lines().any(names_another), "{}", a.code);
            }
        }
    }

    #[test]
    fn codes_are_checked() {
        let c = Catalogue::igs();
        assert_eq!(c.check("LEIAS10", "NONE"), None);
        assert_eq!(c.check("TRM59800.00", "SCIS"), None);
        let err = c.check("LEIAS1O", "NONE").unwrap();
        assert!(
            err.starts_with(
                "antenna \"LEIAS1O\" isn't an IGS antenna code (IGS list of 2026-10-27)"
            ),
            "{err}"
        );
        let err = c.check("LEIAS10", "XXXX").unwrap();
        assert!(err.contains("radome \"XXXX\""), "{err}");
    }

    const GRA: &str = "ARP abbreviations\n=================\nBPA: bottom of preamplifier\n\nDimensions\n==========\n\nANT1\nANT2\n\n   drawing\n   x  <--  0.0000  BPA=ARP\n\n\nANT3\n\n   other\n\n-----\nMachine-readable quick reference section begins here.\nANT1 BPA NOM\nANT2 BPA NOM\nANT3 BAM UNK\n";
    const JSON: &str = r#"{"Antenna": {"ANT1": "one", "ANT2": "two", "ANT3": "three"}, "Dome": {"NONE": "none"}, "Receiver": {}, "Satellite": {}}"#;
    const SLANT: &str = "schema = 1\n[[antenna]]\nigs = \"ANT1\"\npoint = \"SHMP\"\nradius = 0.3\nabove_arp = 0.05\nsource = \"Test, page 1\"\n";

    #[test]
    fn slant_heights_convert_only_with_sourced_geometry() {
        let c = Catalogue::parse(JSON, GRA, "ANT1            NONE\n", SLANT).unwrap();
        assert_eq!(
            c.antenna("ANT2").unwrap().drawing.as_deref(),
            Some("   drawing\n   x  <--  0.0000  BPA=ARP")
        );
        assert_eq!(
            c.antenna("ANT3").unwrap().drawing.as_deref(),
            Some("   other")
        );
        assert_eq!(
            c.antenna("ANT3").unwrap().arp.as_ref().unwrap().meaning,
            None
        );
        let v = vertical_from_slant(&c, "ANT1", "SHMP", 1.5).unwrap();
        assert!((v - ((1.5f64 * 1.5 - 0.09).sqrt() - 0.05)).abs() < 1e-12);
        assert_eq!(
            vertical_from_slant(&c, "ANT1", "SHMP", 0.2),
            Err(SlantError::TooShort {
                slant: 0.2,
                radius: 0.3
            })
        );
        let err = vertical_from_slant(&c, "ANT2", "SHMP", 1.5).unwrap_err();
        assert!(
            err.to_string().contains("no slant geometry for ANT2"),
            "{err}"
        );
        // The built-in table has no entries until someone adds sourced ones.
        assert!(vertical_from_slant(Catalogue::igs(), "LEIAS10", "SHMP", 1.5).is_err());
    }

    #[test]
    fn bad_catalogue_files_say_which_and_why() {
        let bad = |json: &str, gra: &str, slant: &str| {
            Catalogue::parse(json, gra, "", slant)
                .unwrap_err()
                .to_string()
        };
        assert!(bad("{", GRA, "schema = 1").starts_with("rcvr_ant.json:"));
        assert!(
            bad(JSON, "no section", "schema = 1")
                .starts_with("antenna.gra: there's no machine-readable")
        );
        let extra = GRA.replace("ANT3 BAM UNK", "ANT9 BAM UNK");
        assert_eq!(
            bad(JSON, &extra, "schema = 1"),
            "antenna.gra: ANT9 isn't an antenna in rcvr_ant.json"
        );
        assert!(bad(JSON, GRA, "schema = 2").contains("schema 2"));
        assert!(
            bad(JSON, GRA, &SLANT.replace("ANT1", "ANT9"))
                .contains("ANT9 isn't an IGS antenna code")
        );
        assert!(bad(JSON, GRA, &SLANT.replace("0.3", "30")).contains("under 1 m"));
        assert!(
            bad(JSON, GRA, &SLANT.replace("Test, page 1", " "))
                .contains("needs its point and its source")
        );
    }
}
