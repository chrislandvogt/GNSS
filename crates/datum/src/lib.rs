//! Frames, epochs, transformations, plate motion, geoid models and projections.
//!
//! See `docs/ARCHITECTURE.md` (section 11) for this crate's place in the design.
//!
//! - [`Position`] is ECEF coordinates that always carry their [`FrameId`] and
//!   [`CoordinateEpoch`], with an optional [`Covariance`].
//! - [`transform`] moves a position between frames and epochs along an
//!   explicit path of published transformations, which it returns.
//! - [`tm`] projects to and from Transverse Mercator and the Map Grid of Australia.
//!
//! Every transcendental function comes from the `libm` crate (D-024), so
//! results are bit-identical natively and in the browser.

pub mod ellipsoid;
pub mod epoch;
pub mod error;
pub mod frame;
pub mod geodetic;
pub mod helmert;
pub mod position;
pub mod tm;
pub mod transform;

#[cfg(test)]
mod testing;

#[cfg(all(test, target_arch = "wasm32"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

pub use ellipsoid::{Ellipsoid, GRS80, WGS84};
pub use epoch::CoordinateEpoch;
pub use error::DatumError;
pub use frame::{FrameId, FrameKind};
pub use geodetic::Geodetic;
pub use position::{Covariance, Position};
pub use transform::{Step, TransformationGraph, Transformed, transform};

/// The time library, re-exported so every crate uses the same version.
pub use hifitime;
