/// Why a RINEX file couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RinexError {
    /// The file is damaged or isn't what it claims to be. `line` counts from
    /// 1; in a Hatanaka-compressed file it's the line of the compressed file.
    #[error("line {line}: {message}")]
    Format {
        /// The line.
        line: u64,
        /// What's wrong, and what was expected.
        message: String,
    },
    /// The file couldn't be read.
    #[error("{0}")]
    Io(String),
}

impl RinexError {
    pub(crate) fn at(line: u64, message: impl Into<String>) -> Self {
        Self::Format {
            line,
            message: message.into(),
        }
    }
}
