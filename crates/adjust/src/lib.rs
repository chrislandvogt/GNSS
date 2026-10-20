//! Least squares adjustment: statistical testing, reliability, variance components, uncertainty and loop closures.
//!
//! See `docs/ARCHITECTURE.md` (section 10) for this crate's place in the design.
//!
//! - [`Network`] holds marks and [`Observation`]s of them: GNSS baselines,
//!   baseline clusters and point clusters, each with its full covariance.
//! - [`Network::adjust`] adjusts it in a [`Datum`] and returns an
//!   [`Adjustment`]: positions with cofactors, residuals, the global test,
//!   w-tests in local east, north and up, vector tests, minimal detectable
//!   biases and external reliability.
//! - [`stats`] has the distributions behind the critical values, and
//!   [`sparse`] the block-sparse solver.
//! - [`Adjustment::positional_uncertainty`] and
//!   [`Adjustment::relative_uncertainty`] give SP1's PU and RU, with error
//!   ellipses and ellipsoids ([`uncertainty`]).
//! - [`Network::estimate_variance_components`] estimates a scale factor
//!   for each group of observations ([`vce`]).
//! - [`synthetic`] makes networks with known truth, for tests and benchmarks.
//!
//! Every result is the same bits natively and in the browser (D-024).

pub mod adjustment;
pub mod error;
pub mod network;
pub mod sparse;
pub mod stats;
pub mod synthetic;
pub mod uncertainty;
pub mod vce;

pub use adjustment::{
    AdjustedMark, Adjustment, ComponentResult, Criteria, ElementResult, GlobalTest,
    ObservationResult, Statistics,
};
pub use error::AdjustError;
pub use network::{Datum, Element, Mark, Network, Observation, Options};
pub use uncertainty::{Ellipse, Ellipsoid, Scaling, Uncertainty};
pub use vce::{GroupComponent, VarianceComponents, VceOptions};

#[cfg(all(test, target_arch = "wasm32"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);
