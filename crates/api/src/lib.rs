//! Typed command API used by every front end.
//!
//! See `docs/ARCHITECTURE.md` section 8. Every front end (web app, CLI,
//! Python, server) calls the core only through this crate, so anything a user
//! can do in one front end can be done in all of them.
//!
//! So far it holds build information, file hashing, and [`build`], which
//! turns imported files into a network to adjust.

pub mod build;
pub mod export;
pub mod folder;
pub mod project;

/// Version of the core.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Which build of the core is running. Recorded in provenance, because results
/// are only bit-identical within one build (CONVENTIONS.md, determinism).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildTarget {
    /// The WebAssembly build used by the web app.
    Web,
    /// A native build (CLI, Python bindings, server).
    Native,
}

impl BuildTarget {
    /// The build this code was compiled for.
    pub const fn current() -> Self {
        if cfg!(target_arch = "wasm32") {
            Self::Web
        } else {
            Self::Native
        }
    }

    /// Short name used in provenance records.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Native => "native",
        }
    }
}

/// SHA-256 of a file's contents, as recorded in `raw.lock`.
pub fn hash_file(contents: &[u8]) -> String {
    gnss_store::sha256_hex(contents)
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    use super::*;

    #[test]
    fn reports_the_build_it_was_compiled_for() {
        let expected = if cfg!(target_arch = "wasm32") {
            "web"
        } else {
            "native"
        };
        assert_eq!(BuildTarget::current().as_str(), expected);
    }

    #[test]
    fn hashes_files_like_raw_lock() {
        assert_eq!(
            hash_file(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
