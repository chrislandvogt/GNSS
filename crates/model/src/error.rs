use std::fmt;

/// Why a file couldn't be read as a project file.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ModelError {
    /// The text isn't valid TOML, or doesn't have the fields the file needs.
    #[error("{0}")]
    Toml(String),
    /// The file is from a newer version of the app.
    #[error(
        "schema {found} is newer than this version of the app reads ({supported}); open it in a newer version"
    )]
    NewerSchema {
        /// The file's schema version.
        found: u32,
        /// The newest version this build reads.
        supported: u32,
    },
    /// A value is out of range or inconsistent with another.
    #[error("{0}")]
    Invalid(String),
}

/// A problem found by [`crate::validate`], with the file it's in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// The file, relative to the project folder.
    pub file: String,
    /// What's wrong, and what to do about it.
    pub message: String,
}

impl Issue {
    /// An issue in a file.
    pub fn new(file: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.file, self.message)
    }
}

/// Reads TOML, checking the schema version before anything else, so a file
/// from a newer app says so rather than failing on a field it doesn't know.
pub(crate) fn from_toml<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, ModelError> {
    #[derive(serde::Deserialize)]
    struct Version {
        schema: Option<u32>,
    }
    if let Ok(Version {
        schema: Some(found),
    }) = toml::from_str::<Version>(text)
        && found > crate::SCHEMA
    {
        return Err(ModelError::NewerSchema {
            found,
            supported: crate::SCHEMA,
        });
    }
    toml::from_str(text).map_err(|e| ModelError::Toml(e.to_string().trim_end().to_owned()))
}
