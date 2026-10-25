use std::fmt;

use crate::StoreError;

/// A file's path from the project folder, such as `raw/PM12345A.rnx`:
/// relative, written with `/`, with no empty, `.` or `..` parts. It can't
/// leave the project, whatever the store underneath.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectPath(String);

impl ProjectPath {
    /// A path, checked.
    pub fn new(path: impl Into<String>) -> Result<Self, StoreError> {
        let path = path.into();
        let ok = !path.is_empty()
            && !path.contains('\\')
            && !path.contains(':')
            && path
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..");
        if ok {
            Ok(Self(path))
        } else {
            Err(StoreError::BadPath(path))
        }
    }

    /// The path's text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The file's name, after the last `/`.
    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    /// Whether the path is in `raw/`.
    pub fn is_raw(&self) -> bool {
        self.0.starts_with("raw/")
    }
}

impl fmt::Display for ProjectPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::ProjectPath;

    #[test]
    fn paths_stay_inside_the_project() {
        let p = ProjectPath::new("occupations/2026-10-14-PM1-A.toml").unwrap();
        assert_eq!(p.file_name(), "2026-10-14-PM1-A.toml");
        assert!(!p.is_raw());
        assert!(ProjectPath::new("raw/a.rnx").unwrap().is_raw());
        for bad in [
            "",
            "/etc/passwd",
            "raw/../../x",
            "./a",
            "a//b",
            "a/",
            "raw\\a",
            "C:a",
            "raw/./a",
        ] {
            assert!(ProjectPath::new(bad).is_err(), "{bad}");
        }
    }
}
