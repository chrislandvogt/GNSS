//! Errors in files being read or written.

use std::fmt;

/// A problem with a file, and where in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError {
    /// The file's name, as the caller gave it.
    pub file: String,
    /// The line, counting from 1, if the problem is on one.
    pub line: Option<usize>,
    /// What's wrong, in a sentence.
    pub message: String,
}

impl FormatError {
    /// A problem on a line.
    pub fn at(file: &str, line: usize, message: impl Into<String>) -> Self {
        Self {
            file: file.to_owned(),
            line: Some(line),
            message: message.into(),
        }
    }

    /// A problem with the whole file.
    pub fn in_file(file: &str, message: impl Into<String>) -> Self {
        Self {
            file: file.to_owned(),
            line: None,
            message: message.into(),
        }
    }
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}, line {line}: {}", self.file, self.message),
            None => write!(f, "{}: {}", self.file, self.message),
        }
    }
}

impl std::error::Error for FormatError {}
