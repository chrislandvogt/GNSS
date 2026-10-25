//! Build graph, content-addressed cache, provenance and verification.
//!
//! See `docs/ARCHITECTURE.md` (section 7) for this crate's place in the
//! design. Processing is a graph of [`Node`]s, as in a build system. Each
//! node's [`Key`] is the hash of its tool, settings and inputs, with its
//! upstream nodes' keys standing in for their outputs. A node whose key is
//! already in the [`Cache`] isn't run again, so changing one input re-runs
//! only what depends on it. [`verify`] re-runs everything and compares.
//!
//! The crate runs nodes one after another and never touches the file
//! system, the clock or threads: the host supplies the cache (usually a
//! project's `.cache/` through [`StoreCache`]), the work each node does,
//! and progress and cancellation through [`Hooks`].

mod graph;
mod key;
mod run;

pub use graph::{Graph, GraphError, Input, Node, NodeId};
pub use key::{Key, KeyBuilder};
pub use run::{
    Cache, Compute, Event, Hooks, Mismatch, NodeRun, RunError, Status, StoreCache, run, verify,
};
