use std::fmt;

/// A BLAKE3 hash: a node's key, or the content hash of an input or output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key([u8; 32]);

impl Key {
    /// The hash of some bytes, such as a raw file or a node's output.
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// The key in lower-case hexadecimal.
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The first 4 hexadecimal digits, for messages and provenance, as in
    /// `baseline/9c1e`.
    pub fn short(&self) -> String {
        self.hex()[..4].to_owned()
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

/// Builds a key from labelled fields. Every field is written with its label
/// and its length, so no two different sets of fields can be written the
/// same way: `("a", "bc")` and `("ab", "c")` give different keys.
pub struct KeyBuilder(blake3::Hasher);

impl KeyBuilder {
    /// A builder for one kind of key. The domain keeps keys of different
    /// kinds apart even when their fields match.
    pub fn new(domain: &str) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"gnss-pipeline/1\0");
        let mut builder = Self(hasher);
        builder.field("domain", domain.as_bytes());
        builder
    }

    fn field(&mut self, label: &str, bytes: &[u8]) {
        for part in [label.as_bytes(), bytes] {
            self.0.update(&(part.len() as u64).to_le_bytes());
            self.0.update(part);
        }
    }

    /// Adds a text field.
    pub fn text(mut self, label: &str, value: &str) -> Self {
        self.field(label, value.as_bytes());
        self
    }

    /// Adds a bytes field.
    pub fn bytes(mut self, label: &str, value: &[u8]) -> Self {
        self.field(label, value);
        self
    }

    /// Adds another key, such as an upstream node's.
    pub fn key(mut self, label: &str, key: &Key) -> Self {
        self.field(label, &key.0);
        self
    }

    /// The key.
    pub fn finish(self) -> Key {
        Key(*self.0.finalize().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{Key, KeyBuilder};

    #[test]
    fn content_hashes_are_blake3() {
        // The BLAKE3 test vector for empty input.
        assert_eq!(
            Key::of(b"").hex(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert_eq!(Key::of(b"").short(), "af13");
    }

    #[test]
    fn fields_cant_run_into_each_other() {
        let a = KeyBuilder::new("t").text("x", "a").text("y", "bc").finish();
        let b = KeyBuilder::new("t").text("x", "ab").text("y", "c").finish();
        let c = KeyBuilder::new("t").text("xa", "").text("y", "bc").finish();
        let d = KeyBuilder::new("u").text("x", "a").text("y", "bc").finish();
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert_eq!(
            a,
            KeyBuilder::new("t").text("x", "a").text("y", "bc").finish()
        );
    }
}
