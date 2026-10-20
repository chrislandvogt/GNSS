//! Errors from building and adjusting a network.

use gnss_datum::DatumError;

/// Everything that can stop an adjustment. Each variant names the marks or
/// observations involved and, where it helps, what to do about it.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum AdjustError {
    /// A network with nothing in it.
    #[error("the network has no marks to adjust")]
    Empty,

    /// An observation that can't be used as given.
    #[error("observation {id}: {reason}")]
    InvalidObservation {
        /// The observation's identifier.
        id: String,
        /// What's wrong with it.
        reason: String,
    },

    /// A mark that no observation refers to, so the adjustment can't place it.
    #[error("mark {name} isn't in any observation, so it can't be adjusted; remove it")]
    Unobserved {
        /// The mark's name.
        name: String,
    },

    /// A mark given twice.
    #[error("mark {name} is in the network twice")]
    DuplicateMark {
        /// The mark's name.
        name: String,
    },

    /// Part of the network with nothing to fix its position.
    #[error(
        "nothing fixes the position of {}: hold one of its marks fixed or add control to it",
        list(.marks)
    )]
    UndefinedDatum {
        /// The marks in that part of the network.
        marks: Vec<String>,
    },

    /// A minimal datum for a network in separate parts.
    #[error(
        "the network is in {} separate parts, so one fixed mark or inner constraints can't \
         define its datum: {}. Connect them, or adjust each part on its own",
        .parts.len(),
        .parts.iter().map(|p| format!("({})", list(p))).collect::<Vec<_>>().join(", ")
    )]
    Disconnected {
        /// The marks in each part.
        parts: Vec<Vec<String>>,
    },

    /// A datum that doesn't fit the network.
    #[error("{reason}")]
    InvalidDatum {
        /// What's wrong.
        reason: String,
    },

    /// A mark whose position the adjustment needs before it starts.
    #[error("mark {name} needs an approximate position: {reason}")]
    NoApproximatePosition {
        /// The mark's name.
        name: String,
        /// Why it's needed.
        reason: &'static str,
    },

    /// A mark whose position the observations don't determine, or only
    /// barely: near-singular geometry or covariances.
    #[error(
        "the observations don't determine the position of mark {name}: check the geometry and \
         covariances of the observations to it"
    )]
    Singular {
        /// The mark's name.
        name: String,
    },

    /// A mark's approximate position isn't near the Earth's surface, so it has
    /// no local east, north and up axes.
    #[error("mark {name}: {source}")]
    MarkPosition {
        /// The mark's name.
        name: String,
        /// What the datum crate found.
        source: DatumError,
    },

    /// An option out of range.
    #[error("{name} must be {expected}, got {value}")]
    InvalidOption {
        /// The option.
        name: &'static str,
        /// What it must be.
        expected: &'static str,
        /// The value given.
        value: f64,
    },

    /// A frame, epoch or position the datum crate rejected.
    #[error(transparent)]
    Datum(#[from] DatumError),
}

/// Names as a comma-separated list, shortened past ten.
fn list(names: &[String]) -> String {
    const SHOWN: usize = 10;
    if names.len() <= SHOWN {
        names.join(", ")
    } else {
        format!(
            "{} and {} more",
            names[..SHOWN].join(", "),
            names.len() - SHOWN
        )
    }
}
