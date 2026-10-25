//! Domain types: marks, occupations, antennas, heights, sessions and baselines.
//!
//! See `docs/ARCHITECTURE.md` (sections 5 and 6) for this crate's place in the
//! design. The model is built around occupations: one receiver and antenna
//! set up over one mark for one span of time. Sessions and candidate
//! baselines are derived from them, never entered.
//!
//! The types here are what the project folder's files hold: `marks.toml` is
//! a [`MarkFile`] and each file in `occupations/` is an [`Occupation`].
//! Reading one checks it on its own ([`Occupation::from_toml`]); a whole
//! project is checked with [`validate`], which reports every problem at once.

mod error;
mod ids;
mod mark;
mod occupation;
mod session;
mod time;

pub use error::{Issue, ModelError};
pub use ids::{MarkId, OccupationId};
pub use mark::{Constraint, KnownCoordinate, Mark, MarkFile, MarkKind};
pub use occupation::{Antenna, HeightMeasurement, HeightMethod, Occupation, Receiver};
pub use session::{Candidate, Session, candidates, sessions};
pub use time::Timestamp;

/// The schema version of the files this crate reads and writes.
pub const SCHEMA: u32 = 1;

/// Checks a project's marks and occupations together, returning every
/// problem found, in a stable order. Each file was already checked on its own
/// when it was read; this adds what only the whole project can show:
/// duplicate ids, and occupations of marks that `marks.toml` doesn't have.
pub fn validate(marks: &MarkFile, occupations: &[Occupation]) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut mark_ids: Vec<&MarkId> = marks.marks.iter().map(|m| &m.id).collect();
    mark_ids.sort();
    for pair in mark_ids.windows(2) {
        if pair[0] == pair[1] {
            issues.push(Issue::new(
                "marks.toml",
                format!("mark {} is defined twice", pair[0]),
            ));
        }
    }
    let mut occupation_ids: Vec<&OccupationId> = occupations.iter().map(|o| &o.id).collect();
    occupation_ids.sort();
    for pair in occupation_ids.windows(2) {
        if pair[0] == pair[1] {
            issues.push(Issue::new(
                format!("occupations/{}.toml", pair[0]),
                format!("occupation {} is defined twice", pair[0]),
            ));
        }
    }
    for occupation in occupations {
        if mark_ids.binary_search(&&occupation.mark).is_err() {
            issues.push(Issue::new(
                format!("occupations/{}.toml", occupation.id),
                format!(
                    "mark {} isn't in marks.toml; add it there first",
                    occupation.mark
                ),
            ));
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    use super::{Mark, MarkFile, MarkId, Occupation, validate};

    #[test]
    fn a_project_is_checked_as_a_whole() {
        let occupation = Occupation::from_toml(
            "schema = 1\nid = \"A\"\nmark = \"PM1\"\nstart = \"2026-10-14T08:00:00Z\"\nend = \"2026-10-14T10:00:00Z\"\n\
             [receiver]\nmake = \"Leica\"\nmodel = \"GS10\"\n[antenna]\nigs = \"LEIAS10\"\n\
             [height]\nmethod = \"vertical\"\nto = \"ARP\"\nvalue = 1.5\n",
        )
        .unwrap();
        let mut marks = MarkFile::new();
        marks.marks.push(Mark::new(MarkId::new("PM1").unwrap()));
        assert!(validate(&marks, std::slice::from_ref(&occupation)).is_empty());

        marks.marks.push(Mark::new(MarkId::new("PM1").unwrap()));
        let mut other = occupation.clone();
        other.mark = MarkId::new("PM9").unwrap();
        let issues: Vec<String> = validate(&marks, &[occupation, other])
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            issues,
            [
                "marks.toml: mark PM1 is defined twice",
                "occupations/A.toml: occupation A is defined twice",
                "occupations/A.toml: mark PM9 isn't in marks.toml; add it there first",
            ]
        );
    }
}
