//! Data sources, downloads, the shared cache and offline data packs
//! (ARCHITECTURE.md section 13, milestone M7).
//!
//! Given the occupations, [`plan`] works out what's needed from outside the
//! project: CORS observations, broadcast navigation data, precise products
//! and ANTEX. A [`Fetcher`] finds each in the sources a jurisdiction pack
//! declares ([`sources`]) and keeps it in a [`Cache`] shared by every
//! project. Everything but the network client ([`native`], not built for
//! WebAssembly) is the same in the CLI and the web app.

pub mod cache;
pub mod fetcher;
pub mod ga;
#[cfg(not(target_arch = "wasm32"))]
pub mod native;
pub mod pack;
pub mod plan;
pub mod products;
pub mod sites;
pub mod sources;
pub mod time;

#[cfg(all(test, target_arch = "wasm32"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

pub use cache::{Cache, Entry};
pub use fetcher::{Fetched, Fetcher, Http, Problem, Report};
pub use plan::{Need, Options, Want, plan};
pub use sources::{Source, SourceKind, read_sources};

/// Why something couldn't be fetched.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// The pack's sources don't read.
    #[error("the pack's sources: {0}")]
    Pack(String),
    /// The pack has no source of a kind that's needed.
    #[error("the pack has no {0:?} source")]
    NoSource(SourceKind),
    /// A file's link points outside its source's hosts.
    #[error("{0} isn't on one of its source's hosts")]
    Host(String),
    /// A listing or site list that doesn't read.
    #[error("unexpected answer from the source: {0}")]
    Listing(String),
    /// A download shorter or longer than the source said.
    #[error("{name}: got {got} of {expected} bytes")]
    Truncated {
        /// The file.
        name: String,
        /// Bytes received.
        got: u64,
        /// Bytes expected.
        expected: u64,
    },
    /// The network or server failed.
    #[error("{url}: {message}")]
    Http {
        /// The URL, without any signature.
        url: String,
        /// What failed.
        message: String,
    },
    /// An offline data pack that can't be made or read.
    #[error("data pack: {0}")]
    DataPack(String),
    /// The cache failed.
    #[error(transparent)]
    Store(#[from] gnss_store::StoreError),
}
