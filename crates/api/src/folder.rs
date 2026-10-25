//! Loading an occupation-first project folder (ARCHITECTURE.md section 6)
//! from any [`Store`]: `project.toml`, `marks.toml`, one file per occupation
//! in `occupations/`, and the raw files and evidence they refer to.
//!
//! A project that can't be read at all, because `project.toml` is missing or
//! broken, is an error. Everything else is collected as [`Issue`]s, so a
//! person sees every problem at once: a file with problems is left out and
//! the rest is loaded.

use gnss_model::{Issue, MarkFile, Occupation};
use gnss_store::{ProjectPath, Store, StoreError};

use crate::project::{Project, ProjectError};

/// The project's files, as loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// `project.toml`: settings, imports and edits.
    pub project: Project,
    /// `marks.toml`, or no marks if there's no such file.
    pub marks: MarkFile,
    /// The occupations that read without problems, sorted by id.
    pub occupations: Vec<Occupation>,
    /// Every problem found, by file.
    pub issues: Vec<Issue>,
}

/// Why a project couldn't be loaded at all.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LoadError {
    /// There's no `project.toml`.
    #[error("there's no project.toml, so this isn't a project folder")]
    NotAProject,
    /// `project.toml` doesn't read.
    #[error("project.toml: {0}")]
    Project(#[from] ProjectError),
    /// The storage underneath failed.
    #[error("{0}")]
    Store(#[from] StoreError),
}

/// Loads a project from a store, checking every file and the raw files they
/// refer to against `raw.lock`.
pub fn load<S: Store + ?Sized>(store: &S) -> Result<Loaded, LoadError> {
    let text = |path: &str| -> Result<Option<String>, StoreError> {
        let bytes = store.read(&ProjectPath::new(path)?)?;
        Ok(bytes.map(|b| String::from_utf8_lossy(&b).into_owned()))
    };
    let project = Project::from_toml(&text("project.toml")?.ok_or(LoadError::NotAProject)?)?;
    let mut issues = Vec::new();

    let marks = match text("marks.toml")? {
        None => MarkFile::new(),
        Some(text) => MarkFile::from_toml(&text).unwrap_or_else(|e| {
            issues.push(Issue::new("marks.toml", e.to_string()));
            MarkFile::new()
        }),
    };

    let mut occupations = Vec::new();
    for path in store.list("occupations")? {
        let file = path.to_string();
        let Some(stem) = path.file_name().strip_suffix(".toml") else {
            issues.push(Issue::new(
                file,
                "only .toml files belong in occupations/; move it elsewhere",
            ));
            continue;
        };
        let occupation =
            match Occupation::from_toml(&String::from_utf8_lossy(&store.read_existing(&path)?)) {
                Ok(o) => o,
                Err(e) => {
                    issues.push(Issue::new(file, e.to_string()));
                    continue;
                }
            };
        if occupation.id.as_str() != stem {
            issues.push(Issue::new(
                file,
                format!(
                    "its id is {} but its file is {stem}.toml; an occupation's file is named after its id",
                    occupation.id
                ),
            ));
            continue;
        }
        let mut problems = Vec::new();
        for raw in &occupation.rinex {
            if let Err(e) = ProjectPath::new(raw.as_str()).and_then(|p| store.read_raw(&p)) {
                problems.push(e.to_string());
            }
        }
        let evidence = occupation.field_log.iter().chain(&occupation.height.photos);
        for path in evidence {
            let found = ProjectPath::new(path.as_str()).and_then(|p| store.read(&p));
            if !matches!(found, Ok(Some(_))) {
                problems.push(format!("{path}: no such file"));
            }
        }
        if problems.is_empty() {
            occupations.push(occupation);
        } else {
            issues.extend(problems.into_iter().map(|p| Issue::new(file.clone(), p)));
        }
    }
    occupations.sort_by(|a, b| a.id.cmp(&b.id));
    issues.extend(gnss_model::validate(&marks, &occupations));
    Ok(Loaded {
        project,
        marks,
        occupations,
        issues,
    })
}
