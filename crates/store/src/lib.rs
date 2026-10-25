//! Storage interface: the file system natively; the File System Access API and OPFS in the browser.
//!
//! See `docs/ARCHITECTURE.md` (sections 6 and 14.2) for this crate's place in
//! the design.
//!
//! The core reads and writes a project through [`Store`], with paths relative
//! to the project folder written with `/` ([`ProjectPath`]). Natively that's
//! [`FsStore`], a folder on disk. In the browser the app copies between the
//! user's folder (or browser storage) and a [`MemoryStore`], so the core never
//! needs a browser API. Raw files are checked against `raw.lock` with
//! [`RawLock`].

mod lock;
mod path;

#[cfg(not(target_arch = "wasm32"))]
mod fs;

#[cfg(not(target_arch = "wasm32"))]
pub use fs::FsStore;
pub use lock::RawLock;
pub use path::ProjectPath;

use std::collections::BTreeMap;

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

/// Why a store operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// A path that isn't a plain path inside the project.
    #[error("{0:?} isn't a path inside the project (relative, with '/', and no '..')")]
    BadPath(String),
    /// The file doesn't exist.
    #[error("{0}: no such file")]
    NotFound(String),
    /// A raw file that already exists; raw files are never replaced.
    #[error(
        "{0}: already in the project, and raw files are never replaced; give the new file another name"
    )]
    RawExists(String),
    /// A raw file whose bytes no longer match `raw.lock`.
    #[error(
        "{0}: it has changed since it was imported (its SHA-256 no longer matches raw.lock). Raw files are never edited: restore it, or import the new version under another name"
    )]
    RawChanged(String),
    /// A raw file `raw.lock` doesn't list.
    #[error("{0}: not in raw.lock, so it can't be checked; import it again")]
    RawUnlocked(String),
    /// `raw.lock` itself is malformed.
    #[error("raw.lock, line {line}: {message}")]
    BadLock {
        /// The line, from 1.
        line: usize,
        /// What's wrong.
        message: String,
    },
    /// The storage underneath failed.
    #[error("{path}: {message}")]
    Io {
        /// The file.
        path: String,
        /// The error from the system.
        message: String,
    },
}

/// A project folder's files.
pub trait Store {
    /// A file's bytes, or `None` if it doesn't exist.
    fn read(&self, path: &ProjectPath) -> Result<Option<Vec<u8>>, StoreError>;

    /// Writes a file, creating its folders and replacing what was there.
    fn write(&mut self, path: &ProjectPath, bytes: &[u8]) -> Result<(), StoreError>;

    /// The files directly in a folder (not in its subfolders), sorted, as
    /// paths from the project folder. A missing folder has no files.
    fn list(&self, dir: &str) -> Result<Vec<ProjectPath>, StoreError>;

    /// A file's bytes; an error if it doesn't exist.
    fn read_existing(&self, path: &ProjectPath) -> Result<Vec<u8>, StoreError> {
        self.read(path)?
            .ok_or_else(|| StoreError::NotFound(path.to_string()))
    }

    /// A raw file's bytes, checked against `raw.lock`.
    fn read_raw(&self, path: &ProjectPath) -> Result<Vec<u8>, StoreError> {
        let bytes = self.read_existing(path)?;
        RawLock::load(self)?.check(path, &bytes)?;
        Ok(bytes)
    }

    /// Adds a file to `raw/` and its hash to `raw.lock`. Raw files are never
    /// replaced.
    fn add_raw(&mut self, path: &ProjectPath, bytes: &[u8]) -> Result<(), StoreError> {
        if !path.is_raw() {
            return Err(StoreError::BadPath(path.to_string()));
        }
        let mut lock = RawLock::load(self)?;
        if lock.get(path).is_some() || self.read(path)?.is_some() {
            return Err(StoreError::RawExists(path.to_string()));
        }
        self.write(path, bytes)?;
        lock.insert(path.clone(), sha256_hex(bytes));
        lock.save(self)
    }
}

/// A project held in memory: in the browser, and in tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryStore {
    files: BTreeMap<ProjectPath, Vec<u8>>,
}

impl MemoryStore {
    /// An empty project.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every file, sorted by path: for copying the project out.
    pub fn files(&self) -> impl Iterator<Item = (&ProjectPath, &[u8])> {
        self.files.iter().map(|(p, b)| (p, b.as_slice()))
    }
}

impl Store for MemoryStore {
    fn read(&self, path: &ProjectPath) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.files.get(path).cloned())
    }

    fn write(&mut self, path: &ProjectPath, bytes: &[u8]) -> Result<(), StoreError> {
        self.files.insert(path.clone(), bytes.to_vec());
        Ok(())
    }

    fn list(&self, dir: &str) -> Result<Vec<ProjectPath>, StoreError> {
        let prefix = format!("{}/", dir.trim_end_matches('/'));
        Ok(self
            .files
            .keys()
            .filter(|p| {
                p.as_str()
                    .strip_prefix(&prefix)
                    .is_some_and(|rest| !rest.contains('/'))
            })
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    use super::{MemoryStore, ProjectPath, Store, StoreError, sha256_hex};

    fn path(p: &str) -> ProjectPath {
        ProjectPath::new(p).unwrap()
    }

    #[test]
    fn sha256_of_known_inputs() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn listing_is_one_folder_deep_and_sorted() {
        let mut store = MemoryStore::new();
        for p in [
            "occupations/b.toml",
            "occupations/a.toml",
            "occupations/old/c.toml",
            "marks.toml",
        ] {
            store.write(&path(p), b"x").unwrap();
        }
        let names: Vec<String> = store
            .list("occupations")
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(names, ["occupations/a.toml", "occupations/b.toml"]);
        assert!(store.list("overlays").unwrap().is_empty());
    }

    #[test]
    fn raw_files_are_locked_and_never_replaced() {
        let mut store = MemoryStore::new();
        let file = path("raw/PM1.rnx");
        store.add_raw(&file, b"rinex").unwrap();
        assert_eq!(store.read_raw(&file).unwrap(), b"rinex");
        assert_eq!(
            String::from_utf8(store.read_existing(&path("raw.lock")).unwrap()).unwrap(),
            format!("{}  raw/PM1.rnx\n", sha256_hex(b"rinex"))
        );
        assert_eq!(
            store.add_raw(&file, b"other"),
            Err(StoreError::RawExists("raw/PM1.rnx".into()))
        );
        assert!(matches!(
            store.add_raw(&path("marks.toml"), b""),
            Err(StoreError::BadPath(_))
        ));

        store.write(&file, b"edited").unwrap();
        assert_eq!(
            store.read_raw(&file),
            Err(StoreError::RawChanged("raw/PM1.rnx".into()))
        );
        let unlocked = path("raw/sneaked-in.rnx");
        store.write(&unlocked, b"x").unwrap();
        assert_eq!(
            store.read_raw(&unlocked),
            Err(StoreError::RawUnlocked("raw/sneaked-in.rnx".into()))
        );
    }
}
