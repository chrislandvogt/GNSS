//! `rnx2rtkp` configuration files.
//!
//! Option names and values are RTKLIB-EX v2.5.1's (`src/options.c`). Every
//! option that changes a result is written, so a run never depends on
//! RTKLIB's built-in defaults, and the file is recorded with the run's
//! provenance.
//!
//! - **Antennas** are taken from the RINEX headers (`*`): the type and the
//!   height and offsets. The driver passes RTKLIB the reconciled RINEX
//!   (the original with its overlays), so these are the corrected ones.
//! - **The base position** is the known coordinate in the orbits' frame at
//!   the observation epoch (ARCHITECTURE.md section 11.2), or the RINEX
//!   header's approximate position only when nothing better is known.
//! - **The atmosphere** is modelled by baseline length ([`Atmosphere`]).

use std::fmt::Write as _;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How baselines are processed: `[processing]` in `project.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Satellites below this elevation are left out, in degrees.
    #[serde(default = "elevation_mask")]
    pub elevation_mask: f64,
    /// The constellations used: "gps", "glonass", "galileo", "beidou" and
    /// "qzss".
    #[serde(default = "constellations")]
    pub constellations: Vec<Constellation>,
    /// The frequencies used.
    #[serde(default)]
    pub frequencies: Frequencies,
    /// How integer ambiguities are resolved.
    #[serde(default)]
    pub ambiguities: Ambiguities,
    /// The ratio test's threshold for accepting a fix.
    #[serde(default = "ratio")]
    pub ratio: f64,
    /// Resolve GLONASS ambiguities too. Off by default: inter-frequency
    /// biases between receiver brands make it unreliable (D-019).
    #[serde(default)]
    pub glonass_ambiguities: bool,
    /// Signals weaker than this are left out, in dB-Hz, if set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snr_mask: Option<f64>,
}

fn elevation_mask() -> f64 {
    15.0
}

fn constellations() -> Vec<Constellation> {
    vec![
        Constellation::Gps,
        Constellation::Galileo,
        Constellation::Beidou,
    ]
}

fn ratio() -> f64 {
    3.0
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            elevation_mask: elevation_mask(),
            constellations: constellations(),
            frequencies: Frequencies::default(),
            ambiguities: Ambiguities::default(),
            ratio: ratio(),
            glonass_ambiguities: false,
            snr_mask: None,
        }
    }
}

impl Settings {
    /// Whether these are the defaults, which `project.toml` leaves out.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// A constellation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Constellation {
    /// GPS.
    Gps,
    /// GLONASS.
    Glonass,
    /// Galileo.
    Galileo,
    /// BeiDou.
    Beidou,
    /// QZSS.
    Qzss,
}

impl Constellation {
    /// Its letter in RINEX: `G`, `R`, `E`, `C` or `J`.
    pub fn letter(self) -> char {
        match self {
            Constellation::Gps => 'G',
            Constellation::Glonass => 'R',
            Constellation::Galileo => 'E',
            Constellation::Beidou => 'C',
            Constellation::Qzss => 'J',
        }
    }

    /// Its bit in `pos1-navsys`.
    fn bit(self) -> u32 {
        match self {
            Constellation::Gps => 1,
            Constellation::Glonass => 4,
            Constellation::Galileo => 8,
            Constellation::Qzss => 16,
            Constellation::Beidou => 32,
        }
    }
}

/// The frequencies used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Frequencies {
    /// L1 only, as single-frequency receivers give.
    #[serde(rename = "l1")]
    L1,
    /// L1 and L2.
    #[default]
    #[serde(rename = "l1+l2")]
    L1L2,
    /// L1, L2 and L5.
    #[serde(rename = "l1+l2+l5")]
    L1L2L5,
}

/// How integer ambiguities are resolved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Ambiguities {
    /// Continuously, keeping them while satellites are tracked: the usual
    /// choice for static work.
    #[default]
    Continuous,
    /// Fixed and then held.
    FixAndHold,
    /// Epoch by epoch.
    Instantaneous,
    /// Not resolved: a float solution.
    Off,
}

/// Which way through the data a run goes. The driver runs forward and
/// backward as well as combined, to compare them (RTKLIB-EX doesn't, in
/// static mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Forward and backward, smoothed.
    Combined,
    /// Forward only.
    Forward,
    /// Backward only.
    Backward,
}

/// The base receiver's position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Base {
    /// Known: ECEF, in metres, in the orbits' frame at the observation
    /// epoch.
    Known([f64; 3]),
    /// The approximate position in the base's RINEX header, when nothing
    /// better is known. Its error scales the baseline (about 1 ppm per 6 m).
    RinexHeader,
}

/// The orbits and clocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orbits {
    /// Broadcast, from the navigation files.
    Broadcast,
    /// Precise (SP3 and clock files), passed as inputs.
    Precise,
}

/// One run of `rnx2rtkp`: a baseline, one way through its data.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    /// The baseline's approximate length, in metres, which decides how the
    /// atmosphere is modelled.
    pub length: f64,
    /// Which way through the data.
    pub direction: Direction,
    /// The base's position.
    pub base: Base,
    /// The orbits.
    pub orbits: Orbits,
    /// The ANTEX file's name as the run sees it, uncompressed (RTKLIB
    /// can't read gzip without a shell).
    pub antex: String,
    /// Satellites to leave out, such as `G05`, as a retry may.
    pub exclude: Vec<String>,
    /// Write residuals (`.stat`) for the inspector.
    pub residuals: bool,
}

/// How the atmosphere is modelled, which depends on the baseline's length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Atmosphere {
    /// `pos1-ionoopt`.
    pub ionosphere: &'static str,
    /// `pos1-tropopt`.
    pub troposphere: &'static str,
    /// `pos1-tidecorr`: 0 off, 1 solid Earth tides.
    pub tides: u32,
}

/// Baselines this long or longer have solid Earth tides applied. Below it
/// they cancel between the receivers.
pub const LONG: f64 = 10_000.0;

impl Atmosphere {
    /// The models for a baseline of a length, in metres: the broadcast
    /// ionosphere and the Saastamoinen troposphere at every length, with
    /// solid Earth tides from [`LONG`].
    ///
    /// Estimating the atmosphere was tried and isn't used yet. On three
    /// of the M0 spike's 2-hour baselines of 26 to 50 km (2025-06-15), with
    /// GPS, Galileo and BeiDou on L1 and L2, estimating the ionosphere
    /// (`est-stec`) left all three float, where the broadcast model fixed
    /// all three. Estimating the troposphere (`est-ztd`) fixed two of them,
    /// 3 to 7 cm from the broadcast-model fix, and left the 50 km one float.
    /// Which is closer to the truth needs reference coordinates: the
    /// engine benchmark (M10) decides. Tides moved the fixes by about 1 mm.
    /// `frequencies` will matter then: a single-frequency baseline can't
    /// estimate the ionosphere.
    pub fn for_baseline(length: f64, _frequencies: Frequencies) -> Self {
        Self {
            ionosphere: "brdc",
            troposphere: "saas",
            tides: u32::from(length >= LONG),
        }
    }
}

/// Settings or a run that can't be turned into a configuration.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ConfigError {
    /// A setting outside what makes sense.
    #[error("processing settings: {0}")]
    Settings(String),
    /// A run that can't be described.
    #[error("{0}")]
    Run(String),
}

impl Settings {
    /// Checks the settings, so a bad value is reported, not passed on.
    pub fn check(&self) -> Result<(), ConfigError> {
        let bad = |m: String| Err(ConfigError::Settings(m));
        if !(0.0..90.0).contains(&self.elevation_mask) {
            return bad(format!(
                "elevation_mask is {}°; it must be from 0 to below 90",
                self.elevation_mask
            ));
        }
        if self.constellations.is_empty() {
            return bad("constellations is empty".into());
        }
        if !(self.ratio >= 1.0 && self.ratio.is_finite()) {
            return bad(format!("ratio is {}; it must be at least 1", self.ratio));
        }
        if let Some(snr) = self.snr_mask
            && !(0.0..=60.0).contains(&snr)
        {
            return bad(format!("snr_mask is {snr} dB-Hz; it must be from 0 to 60"));
        }
        Ok(())
    }
}

/// The configuration file for a run.
pub fn config(settings: &Settings, run: &Run) -> Result<String, ConfigError> {
    settings.check()?;
    if !(run.length.is_finite() && run.length >= 0.0) {
        return Err(ConfigError::Run(format!(
            "the baseline's length is {}",
            run.length
        )));
    }
    if run.antex.is_empty() || run.antex.contains(['\n', '\r']) {
        return Err(ConfigError::Run("the ANTEX file has no usable name".into()));
    }
    let satellite = |s: &String| {
        s.len() == 3
            && s.starts_with(['G', 'R', 'E', 'C', 'J'])
            && s.get(1..)
                .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
    };
    if let Some(s) = run.exclude.iter().find(|s| !satellite(s)) {
        return Err(ConfigError::Run(format!(
            "{s:?} isn't a satellite, such as G05"
        )));
    }
    let atmosphere = Atmosphere::for_baseline(run.length, settings.frequencies);
    let navsys: u32 = settings
        .constellations
        .iter()
        .fold(0, |bits, c| bits | c.bit());

    let mut out = String::from(
        "# rnx2rtkp configuration, generated from the project's processing settings.\n",
    );
    let mut set = |name: &str, value: &dyn std::fmt::Display| {
        let _ = writeln!(out, "{name:<18} ={value}");
    };
    set("pos1-posmode", &"static");
    set(
        "pos1-frequency",
        &match settings.frequencies {
            Frequencies::L1 => "l1",
            Frequencies::L1L2 => "l1+l2",
            Frequencies::L1L2L5 => "l1+l2+l5",
        },
    );
    set(
        "pos1-soltype",
        &match run.direction {
            Direction::Combined => "combined",
            Direction::Forward => "forward",
            Direction::Backward => "backward",
        },
    );
    set("pos1-elmask", &settings.elevation_mask);
    match settings.snr_mask {
        Some(snr) => {
            // One value for each of RTKLIB's nine elevation bins.
            let bins = vec![snr.to_string(); 9].join(",");
            set("pos1-snrmask_r", &"on");
            set("pos1-snrmask_b", &"on");
            set("pos1-snrmask_L1", &bins);
            set("pos1-snrmask_L2", &bins);
            set("pos1-snrmask_L5", &bins);
        }
        None => {
            set("pos1-snrmask_r", &"off");
            set("pos1-snrmask_b", &"off");
        }
    }
    set("pos1-dynamics", &"off");
    set("pos1-tidecorr", &atmosphere.tides);
    set("pos1-ionoopt", &atmosphere.ionosphere);
    set("pos1-tropopt", &atmosphere.troposphere);
    set(
        "pos1-sateph",
        &match run.orbits {
            Orbits::Broadcast => "brdc",
            Orbits::Precise => "precise",
        },
    );
    set("pos1-exclsats", &run.exclude.join(" "));
    set("pos1-navsys", &navsys);
    set(
        "pos2-armode",
        &match settings.ambiguities {
            Ambiguities::Continuous => "continuous",
            Ambiguities::FixAndHold => "fix-and-hold",
            Ambiguities::Instantaneous => "instantaneous",
            Ambiguities::Off => "off",
        },
    );
    let glonass = settings.glonass_ambiguities && settings.ambiguities != Ambiguities::Off;
    set("pos2-gloarmode", &if glonass { "on" } else { "off" });
    let beidou = settings.ambiguities != Ambiguities::Off;
    set("pos2-bdsarmode", &if beidou { "on" } else { "off" });
    set("pos2-arthres", &settings.ratio);
    set("out-solformat", &"xyz");
    set("out-outhead", &"on");
    set("out-outopt", &"on");
    set("out-timesys", &"gpst");
    set("out-solstatic", &"single");
    set(
        "out-outstat",
        &if run.residuals { "residual" } else { "off" },
    );
    set("ant1-postype", &"single");
    set("ant1-anttype", &"*");
    set("ant2-anttype", &"*");
    match run.base {
        Base::Known([x, y, z]) => {
            set("ant2-postype", &"xyz");
            set("ant2-pos1", &format_args!("{x:.4}"));
            set("ant2-pos2", &format_args!("{y:.4}"));
            set("ant2-pos3", &format_args!("{z:.4}"));
        }
        Base::RinexHeader => set("ant2-postype", &"rinexhead"),
    }
    set("file-satantfile", &run.antex);
    set("file-rcvantfile", &run.antex);
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;

    fn run(length: f64) -> Run {
        Run {
            length,
            direction: Direction::Combined,
            base: Base::RinexHeader,
            orbits: Orbits::Broadcast,
            antex: "igs20_2434.atx".into(),
            exclude: Vec::new(),
            residuals: false,
        }
    }

    /// An option's value in a configuration.
    fn value<'a>(config: &'a str, name: &str) -> Option<&'a str> {
        config.lines().find_map(|line| {
            let (n, v) = line.split_once('=')?;
            (n.trim() == name).then_some(v)
        })
    }

    #[test]
    fn a_short_baseline_gets_the_spike_configuration() {
        // The M0 spike's settings (benchmarks/spike-m0/spike.conf), which
        // matched native builds byte for byte, with the ANTEX of D-028.
        let spike = include_str!("../../../benchmarks/spike-m0/spike.conf");
        let ours = config(&Settings::default(), &run(5_000.0)).unwrap();
        for line in spike.lines().filter(|l| !l.starts_with('#')) {
            let Some((name, expected)) = line.split_once('=') else {
                continue;
            };
            let name = name.trim();
            let expected = match name {
                "file-satantfile" | "file-rcvantfile" => "igs20_2434.atx",
                // An integer bitmask to RTKLIB, which reads "off" as 0.
                "pos1-tidecorr" => "0",
                // The spike skipped residuals and took the base from the
                // header; both are settings of a run here.
                _ => expected.trim(),
            };
            assert_eq!(value(&ours, name), Some(expected), "{name}");
        }
        assert_eq!(value(&ours, "ant2-postype"), Some("rinexhead"));
    }

    #[test]
    fn long_baselines_add_solid_earth_tides() {
        let ours = config(&Settings::default(), &run(25_000.0)).unwrap();
        assert_eq!(value(&ours, "pos1-tidecorr"), Some("1"));
        // The atmosphere is modelled, not estimated (see Atmosphere).
        assert_eq!(value(&ours, "pos1-ionoopt"), Some("brdc"));
        assert_eq!(value(&ours, "pos1-tropopt"), Some("saas"));
        let l1 = Settings {
            frequencies: Frequencies::L1,
            ..Settings::default()
        };
        let ours = config(&l1, &run(25_000.0)).unwrap();
        assert_eq!(value(&ours, "pos1-frequency"), Some("l1"));
    }

    #[test]
    fn runs_set_the_direction_base_orbits_and_exclusions() {
        let settings = Settings {
            constellations: vec![Constellation::Gps, Constellation::Glonass],
            glonass_ambiguities: true,
            snr_mask: Some(35.0),
            ..Settings::default()
        };
        let backward = Run {
            direction: Direction::Backward,
            base: Base::Known([-4_646_398.123_46, 2_557_543.0, -3_525_684.987_65]),
            orbits: Orbits::Precise,
            exclude: vec!["G05".into(), "R12".into()],
            residuals: true,
            ..run(1_000.0)
        };
        let ours = config(&settings, &backward).unwrap();
        assert_eq!(value(&ours, "pos1-soltype"), Some("backward"));
        assert_eq!(value(&ours, "pos1-navsys"), Some("5"));
        assert_eq!(value(&ours, "pos2-gloarmode"), Some("on"));
        assert_eq!(value(&ours, "pos1-sateph"), Some("precise"));
        assert_eq!(value(&ours, "pos1-exclsats"), Some("G05 R12"));
        assert_eq!(
            value(&ours, "pos1-snrmask_L2"),
            Some("35,35,35,35,35,35,35,35,35")
        );
        assert_eq!(value(&ours, "out-outstat"), Some("residual"));
        assert_eq!(value(&ours, "ant2-postype"), Some("xyz"));
        assert_eq!(value(&ours, "ant2-pos1"), Some("-4646398.1235"));
        assert_eq!(value(&ours, "ant2-pos3"), Some("-3525684.9877"));
        // Float only: no ambiguities are resolved, GLONASS's included.
        let float = Settings {
            ambiguities: Ambiguities::Off,
            ..settings
        };
        let ours = config(&float, &backward).unwrap();
        assert_eq!(value(&ours, "pos2-armode"), Some("off"));
        assert_eq!(value(&ours, "pos2-gloarmode"), Some("off"));
    }

    #[test]
    fn bad_settings_and_runs_are_refused() {
        let bad = |s: Settings| config(&s, &run(1.0)).unwrap_err();
        assert!(matches!(
            bad(Settings {
                elevation_mask: 90.0,
                ..Settings::default()
            }),
            ConfigError::Settings(_)
        ));
        bad(Settings {
            constellations: Vec::new(),
            ..Settings::default()
        });
        bad(Settings {
            ratio: f64::NAN,
            ..Settings::default()
        });
        bad(Settings {
            snr_mask: Some(99.0),
            ..Settings::default()
        });
        let settings = Settings::default();
        assert!(config(&settings, &run(-1.0)).is_err());
        assert!(
            config(
                &settings,
                &Run {
                    antex: "a\nfile-x =y".into(),
                    ..run(1.0)
                }
            )
            .is_err()
        );
        assert!(
            config(
                &settings,
                &Run {
                    exclude: vec!["G05 =x".into()],
                    ..run(1.0)
                }
            )
            .is_err()
        );
    }

    #[test]
    fn settings_read_from_toml_with_defaults() {
        let settings: Settings = toml::from_str(
            r#"elevation_mask = 10
constellations = ["gps", "galileo"]
ambiguities = "fix-and-hold"
"#,
        )
        .unwrap();
        assert_eq!(settings.elevation_mask, 10.0);
        assert_eq!(settings.ratio, 3.0);
        assert_eq!(settings.frequencies, Frequencies::L1L2);
        assert!(!settings.is_default());
        assert!(toml::from_str::<Settings>("").unwrap().is_default());
        assert!(toml::from_str::<Settings>("elevation = 10").is_err());
    }
}
