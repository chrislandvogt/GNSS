use std::collections::BTreeMap;

use crate::{ProjectPath, Store, StoreError, sha256_hex};

/// `raw.lock`: the SHA-256 of every file in `raw/`, one per line as
/// `<hash>  <path>`, the format `sha256sum` writes and checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawLock {
    hashes: BTreeMap<ProjectPath, String>,
}

impl RawLock {
    /// The lock file's path.
    pub const FILE: &str = "raw.lock";

    /// Reads `raw.lock` from a store; a project without one has no raw files.
    pub fn load<S: Store + ?Sized>(store: &S) -> Result<Self, StoreError> {
        let path = ProjectPath::new(Self::FILE)?;
        match store.read(&path)? {
            None => Ok(Self::default()),
            Some(bytes) => Self::parse(&String::from_utf8_lossy(&bytes)),
        }
    }

    /// Parses the lock file's text.
    pub fn parse(text: &str) -> Result<Self, StoreError> {
        let mut hashes = BTreeMap::new();
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let bad = |message: &str| StoreError::BadLock {
                line: index + 1,
                message: message.into(),
            };
            let (hash, file) = line
                .split_once("  ")
                .ok_or_else(|| bad("expected a SHA-256 and a path, separated by two spaces"))?;
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(bad("the hash isn't a SHA-256 in hexadecimal"));
            }
            let file = ProjectPath::new(file).map_err(|e| bad(&e.to_string()))?;
            if !file.is_raw() {
                return Err(bad("only files in raw/ are locked"));
            }
            if hashes.insert(file, hash.to_ascii_lowercase()).is_some() {
                return Err(bad("the file is listed twice"));
            }
        }
        Ok(Self { hashes })
    }

    /// The lock file's text, sorted by path.
    pub fn to_text(&self) -> String {
        self.hashes
            .iter()
            .map(|(file, hash)| format!("{hash}  {file}\n"))
            .collect()
    }

    /// Writes `raw.lock` to a store.
    pub fn save<S: Store + ?Sized>(&self, store: &mut S) -> Result<(), StoreError> {
        store.write(&ProjectPath::new(Self::FILE)?, self.to_text().as_bytes())
    }

    /// A file's recorded hash.
    pub fn get(&self, file: &ProjectPath) -> Option<&str> {
        self.hashes.get(file).map(String::as_str)
    }

    /// Records a file's hash.
    pub fn insert(&mut self, file: ProjectPath, sha256: String) {
        self.hashes.insert(file, sha256);
    }

    /// Every locked file, sorted.
    pub fn files(&self) -> impl Iterator<Item = &ProjectPath> {
        self.hashes.keys()
    }

    /// Checks a raw file's bytes against its recorded hash.
    pub fn check(&self, file: &ProjectPath, bytes: &[u8]) -> Result<(), StoreError> {
        match self.get(file) {
            None => Err(StoreError::RawUnlocked(file.to_string())),
            Some(expected) if expected == sha256_hex(bytes) => Ok(()),
            Some(_) => Err(StoreError::RawChanged(file.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::RawLock;
    use crate::{ProjectPath, StoreError, sha256_hex};

    #[test]
    fn lock_files_round_trip_sorted() {
        let mut lock = RawLock::default();
        lock.insert(ProjectPath::new("raw/b.rnx").unwrap(), sha256_hex(b"b"));
        lock.insert(ProjectPath::new("raw/a.rnx").unwrap(), sha256_hex(b"a"));
        let text = lock.to_text();
        assert!(text.starts_with(&format!("{}  raw/a.rnx\n", sha256_hex(b"a"))));
        assert_eq!(RawLock::parse(&text).unwrap(), lock);
    }

    #[test]
    fn malformed_lines_are_reported_with_their_line() {
        let good = format!("{}  raw/a.rnx\n", sha256_hex(b"a"));
        for (text, line, expected) in [
            (format!("{good}\nnot a lock line"), 3, "two spaces"),
            (format!("{good}abc  raw/b.rnx"), 2, "isn't a SHA-256"),
            (
                format!("{}  marks.toml", sha256_hex(b"m")),
                1,
                "only files in raw/",
            ),
            (format!("{}  raw/../x", sha256_hex(b"x")), 1, "isn't a path"),
            (format!("{good}{good}"), 2, "listed twice"),
        ] {
            match RawLock::parse(&text) {
                Err(StoreError::BadLock { line: l, message }) => {
                    assert_eq!(l, line, "{text}");
                    assert!(message.contains(expected), "{message}");
                }
                other => panic!("{text}: {other:?}"),
            }
        }
    }
}
