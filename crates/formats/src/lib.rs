//! Import and export of exchange formats: DynaML, STAR*NET, RTKLIB .pos, SINEX, LandXML, CSV and GeoPackage.
//!
//! See `docs/ARCHITECTURE.md` for this crate's place in the design.
//!
//! Readers turn a file's text into a [`Survey`]: stations and GNSS
//! measurements as the file gives them, in their own frames and epochs, with
//! notes on anything skipped. They never panic on bad input; errors say which
//! file and line, and what's wrong. Writers turn a [`Survey`] (or results)
//! back into a format. Nothing here touches the file system: callers pass text
//! and get text back, so the same code runs natively and in the browser.

pub mod csv;
pub mod dynaml;
pub mod error;
pub mod rtklib;
pub mod sinex;
pub mod starnet;
pub mod survey;
mod text;
mod xml;

pub use error::{FormatError, WriteError};
pub use survey::{
    Constraint, Coordinate, CoordinateValue, Element, HeightType, Measurement, MeasurementKind,
    Note, Origin, Reference, Scaling, Station, Survey,
};
pub use text::decode;
