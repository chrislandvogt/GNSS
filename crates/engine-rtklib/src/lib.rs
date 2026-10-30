//! RTKLIB-EX driver: configuration generation and output parsing.
//!
//! See `docs/ARCHITECTURE.md` section 9.3 for this crate's place in the
//! design. The configuration is generated here, one file per run, from the
//! project's processing settings (`[processing]` in `project.toml`) and what
//! the run is: its baseline, its base position, its orbits and its
//! direction. Solutions are read by `gnss_formats::rtklib`.

pub mod config;

pub use config::{Atmosphere, Base, ConfigError, Direction, Orbits, Run, Settings, config};
