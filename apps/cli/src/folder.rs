//! The project folder on disk: `project.toml`, `raw/` with `raw.lock`, and
//! `results/` (ARCHITECTURE.md section 6).

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use gnss_api::project::{EditRecord, Import, Project};
use gnss_store::{FsStore, ProjectPath, Store, StoreError};
use serde::Serialize;

/// A project folder.
pub struct Folder {
    root: PathBuf,
    store: FsStore,
}

impl Folder {
    /// Creates a project folder, which must not exist or be empty.
    pub fn create(root: &Path, project: &Project) -> Result<Folder, String> {
        if root.exists()
            && fs::read_dir(root)
                .map_err(|e| format!("{}: {e}", root.display()))?
                .next()
                .is_some()
        {
            return Err(format!(
                "{} already exists and isn't empty; choose a new folder",
                root.display()
            ));
        }
        let folder = Folder::at(root);
        for dir in [root.to_path_buf(), folder.raw(), folder.results()] {
            fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let text = format!(
            "# A GNSS adjustment project (ARCHITECTURE.md section 6).\n\
             # Imports and edits are added by `gnss import` and `gnss exclude`; edit the\n\
             # settings here.\n\n{}",
            project.to_toml().map_err(|e| e.to_string())?
        );
        folder.write(&folder.project_file(), text.as_bytes())?;
        folder.write(&folder.lock_file(), b"")?;
        Ok(folder)
    }

    /// Opens an existing project folder.
    pub fn open(root: &Path) -> Result<Folder, String> {
        let folder = Folder::at(root);
        if !folder.project_file().is_file() {
            return Err(format!(
                "{} isn't a project: there's no project.toml. Create one with gnss init, or \
                 give --project",
                root.display()
            ));
        }
        Ok(folder)
    }

    fn at(root: &Path) -> Folder {
        Folder {
            root: root.to_path_buf(),
            store: FsStore::new(root),
        }
    }

    /// The folder as a store, for the API's functions that work on any.
    pub fn store(&self) -> FsStore {
        self.store.clone()
    }

    /// The folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn project_file(&self) -> PathBuf {
        self.root.join("project.toml")
    }

    fn lock_file(&self) -> PathBuf {
        self.root.join("raw.lock")
    }

    fn raw(&self) -> PathBuf {
        self.root.join("raw")
    }

    /// The results folder.
    pub fn results(&self) -> PathBuf {
        self.root.join("results")
    }

    /// The project's settings, imports and edits.
    pub fn project(&self) -> Result<Project, String> {
        let text =
            fs::read_to_string(self.project_file()).map_err(|e| format!("project.toml: {e}"))?;
        Project::from_toml(&text).map_err(|e| e.to_string())
    }

    /// A raw file's bytes, checked against `raw.lock`: raw files never change.
    /// The message doesn't repeat the file's name, which callers add.
    pub fn read_raw(&self, file: &str) -> Result<Vec<u8>, String> {
        ProjectPath::new(file)
            .and_then(|path| self.store.read_raw(&path))
            .map_err(|e| {
                let message = e.to_string();
                message
                    .strip_prefix(&format!("{file}: "))
                    .map_or(message.clone(), str::to_owned)
            })
    }

    /// Copies an import into `raw/`, records its hash and adds it to the project.
    pub fn add_import(&self, import: &Import, bytes: &[u8]) -> Result<(), String> {
        let path = ProjectPath::new(import.file.as_str()).map_err(|e| e.to_string())?;
        if self.store.read(&path).map_err(|e| e.to_string())?.is_some() {
            return Err(StoreError::RawExists(import.file.clone()).to_string());
        }
        let project = self.project()?;
        if project.imports.iter().any(|i| i.file == import.file) {
            return Err(format!("{} is already imported", import.file));
        }
        let mut store = self.store.clone();
        store.add_raw(&path, bytes).map_err(|e| e.to_string())?;
        #[derive(Serialize)]
        struct Entry<'a> {
            import: [&'a Import; 1],
        }
        let text = toml::to_string(&Entry { import: [import] }).map_err(|e| e.to_string())?;
        self.append(&self.project_file(), &format!("\n{text}"))
    }

    /// Adds an edit to the project.
    pub fn add_edit(&self, edit: &EditRecord) -> Result<(), String> {
        #[derive(Serialize)]
        struct Entry<'a> {
            edit: [&'a EditRecord; 1],
        }
        let text = toml::to_string(&Entry { edit: [edit] }).map_err(|e| e.to_string())?;
        self.append(&self.project_file(), &format!("\n{text}"))?;
        // Check the project still reads.
        self.project().map(|_| ())
    }

    /// Writes a file, creating its folder.
    pub fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn append(&self, path: &Path, text: &str) -> Result<(), String> {
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        file.write_all(text.as_bytes())
            .map_err(|e| format!("{}: {e}", path.display()))
    }
}
