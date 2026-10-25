use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::{ProjectPath, Store, StoreError};

/// A project folder on disk.
#[derive(Debug, Clone)]
pub struct FsStore {
    root: PathBuf,
}

impl FsStore {
    /// The project in a folder, which needn't exist yet.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The project folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where a project file is on disk.
    pub fn full_path(&self, path: &ProjectPath) -> PathBuf {
        path.as_str()
            .split('/')
            .fold(self.root.clone(), |full, part| full.join(part))
    }

    fn io(&self, path: &Path, e: std::io::Error) -> StoreError {
        StoreError::Io {
            path: path.display().to_string(),
            message: e.to_string(),
        }
    }
}

impl Store for FsStore {
    fn read(&self, path: &ProjectPath) -> Result<Option<Vec<u8>>, StoreError> {
        let full = self.full_path(path);
        match fs::read(&full) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(self.io(&full, e)),
        }
    }

    fn write(&mut self, path: &ProjectPath, bytes: &[u8]) -> Result<(), StoreError> {
        let full = self.full_path(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).map_err(|e| self.io(parent, e))?;
        }
        fs::write(&full, bytes).map_err(|e| self.io(&full, e))
    }

    fn list(&self, dir: &str) -> Result<Vec<ProjectPath>, StoreError> {
        let dir = ProjectPath::new(dir.trim_end_matches('/'))?;
        let full = self.full_path(&dir);
        let entries = match fs::read_dir(&full) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(self.io(&full, e)),
        };
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| self.io(&full, e))?;
            let is_file = entry.file_type().map_err(|e| self.io(&full, e))?.is_file();
            // Names that aren't valid UTF-8 or project paths can't be
            // referred to from the project's files, so they're left out.
            if let (true, Some(name)) = (is_file, entry.file_name().to_str())
                && let Ok(path) = ProjectPath::new(format!("{dir}/{name}"))
            {
                out.push(path);
            }
        }
        out.sort();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::FsStore;
    use crate::{MemoryStore, ProjectPath, Store};

    #[test]
    fn behaves_as_the_memory_store_does() {
        let dir = std::env::temp_dir().join(format!("gnss-store-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut disk = FsStore::new(&dir);
        let mut memory = MemoryStore::new();
        for store in [&mut disk as &mut dyn Store, &mut memory as &mut dyn Store] {
            store
                .write(&ProjectPath::new("occupations/a.toml").unwrap(), b"a")
                .unwrap();
            store
                .write(&ProjectPath::new("marks.toml").unwrap(), b"m")
                .unwrap();
            store
                .add_raw(&ProjectPath::new("raw/x.rnx").unwrap(), b"x")
                .unwrap();
        }
        for store in [&disk as &dyn Store, &memory as &dyn Store] {
            assert_eq!(
                store.list("occupations").unwrap(),
                [ProjectPath::new("occupations/a.toml").unwrap()]
            );
            assert_eq!(
                store
                    .read_raw(&ProjectPath::new("raw/x.rnx").unwrap())
                    .unwrap(),
                b"x"
            );
            assert_eq!(
                store
                    .read(&ProjectPath::new("missing.toml").unwrap())
                    .unwrap(),
                None
            );
            assert!(store.list("nowhere").unwrap().is_empty());
        }
        assert_eq!(
            disk.read_existing(&ProjectPath::new("raw.lock").unwrap())
                .unwrap(),
            memory
                .read_existing(&ProjectPath::new("raw.lock").unwrap())
                .unwrap()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
