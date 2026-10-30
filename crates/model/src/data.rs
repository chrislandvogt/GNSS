use serde::{Deserialize, Serialize};

use crate::error::from_toml;
use crate::{ModelError, SCHEMA};

/// `data.toml`: the files the project uses from outside it, such as CORS
/// observations and precise orbits, as fetched into the shared download
/// cache. It records exactly which files were used, by hash, and the
/// attribution their licences ask for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DataFiles {
    /// The schema version: 1 (`SCHEMA`), raised only when a migration is needed.
    pub schema: u32,
    /// What reports must say about the data, word for word.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributions: Vec<String>,
    /// The files, sorted by what they're for, then by name.
    #[serde(default, rename = "file", skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<DataFile>,
}

/// A file from outside the project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DataFile {
    /// What it's for, such as `SYDN observations, 2025-06-15 03:05:00 to
    /// 2025-06-15 05:15:00 GPS`.
    pub purpose: String,
    /// The file's name, as published.
    pub name: String,
    /// The source's id in the jurisdiction pack.
    pub source: String,
    /// The file's SHA-256, in hex: its key in the download cache.
    pub sha256: String,
    /// Its size in bytes.
    pub bytes: u64,
    /// Where it came from.
    pub url: String,
    /// When it was downloaded, in UTC.
    pub fetched: String,
    /// The data's licence.
    pub licence: String,
    /// Anything to know, such as rapid orbits used because finals weren't
    /// out yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl DataFiles {
    /// An empty file at the current schema.
    pub fn new() -> Self {
        Self {
            schema: SCHEMA,
            attributions: Vec::new(),
            files: Vec::new(),
        }
    }

    /// Reads `data.toml`, checking the hashes are hashes.
    pub fn from_toml(text: &str) -> Result<Self, ModelError> {
        let file: DataFiles = from_toml(text)?;
        for f in &file.files {
            if f.sha256.len() != 64 || !f.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(ModelError::Invalid(format!(
                    "file {}: {:?} isn't a SHA-256",
                    f.name, f.sha256
                )));
            }
        }
        Ok(file)
    }

    /// The file as TOML.
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// Adds files and attributions, replacing any entry for the same
    /// purpose and name, and keeps both lists sorted.
    pub fn merge(&mut self, files: Vec<DataFile>, attributions: impl IntoIterator<Item = String>) {
        for file in files {
            self.files
                .retain(|f| !(f.purpose == file.purpose && f.name == file.name));
            self.files.push(file);
        }
        self.files
            .sort_by(|a, b| (&a.purpose, &a.name).cmp(&(&b.purpose, &b.name)));
        self.attributions.extend(attributions);
        self.attributions.sort();
        self.attributions.dedup();
    }
}

impl Default for DataFiles {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;

    fn file(purpose: &str, name: &str, hash: char) -> DataFile {
        DataFile {
            purpose: purpose.into(),
            name: name.into(),
            source: "ga-archive".into(),
            sha256: hash.to_string().repeat(64),
            bytes: 10,
            url: "https://x.example/f".into(),
            fetched: "2026-10-29T00:00:00Z".into(),
            licence: "CC BY 4.0".into(),
            note: None,
        }
    }

    #[test]
    fn files_merge_sorted_and_round_trip() {
        let mut data = DataFiles::new();
        data.merge(
            vec![file("b", "x", 'a'), file("a", "y", 'b')],
            ["GA".to_owned()],
        );
        data.merge(
            vec![file("b", "x", 'c')],
            ["GA".to_owned(), "CODE".to_owned()],
        );
        assert_eq!(data.files.len(), 2);
        assert_eq!(data.files[0].purpose, "a");
        assert_eq!(data.files[1].sha256, "c".repeat(64));
        assert_eq!(data.attributions, ["CODE", "GA"]);
        let again = DataFiles::from_toml(&data.to_toml()).unwrap();
        assert_eq!(again, data);
        let bad = data.to_toml().replace(&"c".repeat(64), "nothex");
        assert!(DataFiles::from_toml(&bad).is_err());
    }
}
