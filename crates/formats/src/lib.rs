//! Import and export of exchange formats: DynaML, STAR*NET, Trimble's TDEF, RTKLIB .pos, SINEX, LandXML, CSV and GeoPackage.
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
pub mod gpkg;
pub mod landxml;
pub mod rtklib;
pub mod sinex;
mod sqlite;
pub mod starnet;
pub mod survey;
pub mod tdef;
mod text;
mod xml;

pub use error::{FormatError, WriteError};
pub use survey::{
    Constraint, Coordinate, CoordinateValue, Element, HeightType, Measurement, MeasurementKind,
    Note, Origin, Reference, Scaling, Station, Survey,
};
pub use text::decode;

/// An epoch written as a decimal year (`2026.79`), an ISO date (`2026-10-14`)
/// or date and time in UTC (`2026-10-14T03:00:00Z`), or a DynaML date
/// (`14.10.2026`). `field` names it in the message if it isn't one.
pub fn parse_epoch(field: &str, text: &str) -> Result<gnss_datum::CoordinateEpoch, String> {
    text::epoch_text(field, text)
}
