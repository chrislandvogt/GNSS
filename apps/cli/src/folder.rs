//! The project folder on disk: `project.toml`, `raw/` with `raw.lock`, and
//! `results/` (ARCHITECTURE.md section 6).

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use gnss_api::project::{EditRecord, Import, Project};
use serde::Serialize;

/// A project folder.
pub struct Folder {
    root: PathBuf,
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
        let folder = Folder {
            root: root.to_path_buf(),
        };
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
        let folder = Folder {
            root: root.to_path_buf(),
        };
        if !folder.project_file().is_file() {
            return Err(format!(
                "{} isn't a project: there's no project.toml. Create one with gnss init, or \
                 give --project",
                root.display()
            ));
        }
        Ok(folder)
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

    /// `raw.lock`'s entries: (SHA-256, path).
    fn locks(&self) -> Result<Vec<(String, String)>, String> {
        let text = fs::read_to_string(self.lock_file()).map_err(|e| format!("raw.lock: {e}"))?;
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                l.split_once("  ")
                    .map(|(h, p)| (h.to_owned(), p.to_owned()))
                    .ok_or_else(|| format!("raw.lock: {l:?} isn't a hash and a path"))
            })
            .collect()
    }

    /// A raw file's bytes, checked against `raw.lock`: raw files never change.
    pub fn read_raw(&self, file: &str) -> Result<Vec<u8>, String> {
        let bytes = fs::read(self.root.join(file)).map_err(|e| e.to_string())?;
        let hash = gnss_store::sha256_hex(&bytes);
        match self.locks()?.iter().find(|(_, p)| p == file) {
            Some((expected, _)) if *expected == hash => Ok(bytes),
            Some(_) => Err(
                "it has changed since it was imported (its SHA-256 no longer matches raw.lock). \
                 Raw files are never edited: restore it, or import the new version under another \
                 name"
                    .into(),
            ),
            None => Err("raw.lock has no hash for it".into()),
        }
    }

    /// Copies an import into `raw/`, records its hash and adds it to the project.
    pub fn add_import(&self, import: &Import, bytes: &[u8]) -> Result<(), String> {
        let target = self.root.join(&import.file);
        if target.exists() {
            return Err(format!(
                "{} is already in the project; give the new file another name",
                import.file
            ));
        }
        let project = self.project()?;
        if project.imports.iter().any(|i| i.file == import.file) {
            return Err(format!("{} is already imported", import.file));
        }
        self.write(&target, bytes)?;
        self.append(
            &self.lock_file(),
            &format!("{}  {}\n", gnss_store::sha256_hex(bytes), import.file),
        )?;
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
