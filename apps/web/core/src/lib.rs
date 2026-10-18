//! The core compiled to WebAssembly, for the web app's core worker.
//!
//! This crate only adapts the `api` crate to JavaScript. It holds no domain
//! logic (ARCHITECTURE.md section 8). Natively it compiles to nothing.

#[cfg(target_arch = "wasm32")]
mod bindings {
    use wasm_bindgen::prelude::wasm_bindgen;

    /// Version of the core.
    #[wasm_bindgen]
    pub fn version() -> String {
        gnss_api::VERSION.to_owned()
    }

    /// Which build of the core is running: always "web" here.
    #[wasm_bindgen(js_name = buildTarget)]
    pub fn build_target() -> String {
        gnss_api::BuildTarget::current().as_str().to_owned()
    }

    /// SHA-256 of a file's contents, as recorded in `raw.lock`.
    #[wasm_bindgen(js_name = hashFile)]
    pub fn hash_file(contents: &[u8]) -> String {
        gnss_api::hash_file(contents)
    }

    #[cfg(test)]
    mod tests {
        use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

        wasm_bindgen_test_configure!(run_in_browser);

        #[wasm_bindgen_test]
        fn reports_the_web_build() {
            assert_eq!(super::build_target(), "web");
        }

        #[wasm_bindgen_test]
        fn hashes_through_the_bindings() {
            assert_eq!(
                super::hash_file(b"abc"),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            );
        }
    }
}
