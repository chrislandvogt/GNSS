//! Storage interface: the file system natively; the File System Access API and OPFS in the browser.
//!
//! See `docs/ARCHITECTURE.md` for this crate's place in the design.
//!
//! So far this holds only content hashing. The project format uses SHA-256 to
//! verify raw files against `raw.lock` (ARCHITECTURE.md section 6).

use sha2::{Digest, Sha256};

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Lower-case hexadecimal SHA-256 of `data`, in the form written to `raw.lock`.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut out = String::with_capacity(64);
    for byte in Sha256::digest(data) {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    use super::sha256_hex;

    #[test]
    fn hashes_empty_input() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hashes_fips_180_2_example() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
