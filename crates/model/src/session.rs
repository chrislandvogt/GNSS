//! Sessions and candidate baselines, derived from occupations
//! (ARCHITECTURE.md section 5). Neither is ever entered by hand.

use crate::{MarkId, Occupation, OccupationId, Timestamp};

/// Two occupations of different marks that overlap long enough to process
/// a baseline between them.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// The occupation whose id sorts first.
    pub from: OccupationId,
    /// The other occupation.
    pub to: OccupationId,
    /// The marks occupied, in the same order.
    pub marks: (MarkId, MarkId),
    /// When both were observing.
    pub start: Timestamp,
    /// When the first of them stopped.
    pub end: Timestamp,
}

impl Candidate {
    /// How long both were observing, in seconds.
    pub fn overlap_s(&self) -> f64 {
        self.start.seconds_until(self.end)
    }
}

/// Every candidate baseline: each pair of occupations of different marks
/// that overlap by at least `min_overlap_s` seconds, sorted by their ids.
///
/// Two occupations of the same mark at the same time aren't a baseline, so
/// they're left out; the blunder checks (M9) report them.
pub fn candidates(occupations: &[Occupation], min_overlap_s: f64) -> Vec<Candidate> {
    let mut sorted: Vec<&Occupation> = occupations.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let mut out = Vec::new();
    for (i, a) in sorted.iter().enumerate() {
        for b in &sorted[i + 1..] {
            if a.mark == b.mark {
                continue;
            }
            let start = if a.start > b.start { a.start } else { b.start };
            let end = if a.end < b.end { a.end } else { b.end };
            if start.seconds_until(end) >= min_overlap_s {
                out.push(Candidate {
                    from: a.id.clone(),
                    to: b.id.clone(),
                    marks: (a.mark.clone(), b.mark.clone()),
                    start,
                    end,
                });
            }
        }
    }
    out
}

/// Occupations joined by candidate baselines, directly or through others:
/// what a field crew calls a session.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    /// The occupations, sorted by id.
    pub occupations: Vec<OccupationId>,
    /// When the first of them started.
    pub start: Timestamp,
    /// When the last of them ended.
    pub end: Timestamp,
}

/// Groups occupations into sessions: the connected groups of the candidate
/// baselines with at least `min_overlap_s` of overlap. An occupation that
/// overlaps nothing is a session on its own. Sessions are sorted by start,
/// then by their first occupation's id.
pub fn sessions(occupations: &[Occupation], min_overlap_s: f64) -> Vec<Session> {
    let mut sorted: Vec<&Occupation> = occupations.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let index = |id: &OccupationId| sorted.binary_search_by(|o| o.id.cmp(id)).ok();
    // Union-find over the sorted occupations.
    let mut parent: Vec<usize> = (0..sorted.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for candidate in candidates(occupations, min_overlap_s) {
        if let (Some(a), Some(b)) = (index(&candidate.from), index(&candidate.to)) {
            let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
            parent[ra.max(rb)] = ra.min(rb);
        }
    }
    let mut groups: Vec<Vec<&Occupation>> = Vec::new();
    let mut group_of: Vec<Option<usize>> = vec![None; sorted.len()];
    for (i, occupation) in sorted.iter().enumerate() {
        let r = root(&mut parent, i);
        let g = *group_of[r].get_or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[g].push(occupation);
    }
    let mut out: Vec<Session> = groups
        .into_iter()
        .filter_map(|members| {
            let start = members
                .iter()
                .map(|o| o.start)
                .reduce(|a, b| if b < a { b } else { a })?;
            let end = members
                .iter()
                .map(|o| o.end)
                .reduce(|a, b| if b > a { b } else { a })?;
            Some(Session {
                occupations: members.iter().map(|o| o.id.clone()).collect(),
                start,
                end,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.occupations.cmp(&b.occupations))
    });
    out
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{candidates, sessions};
    use crate::{Occupation, Timestamp};

    fn occupation(id: &str, mark: &str, start: &str, end: &str) -> Occupation {
        let text = format!(
            "schema = 1\nid = \"{id}\"\nmark = \"{mark}\"\nstart = \"2026-10-14T{start}:00Z\"\nend = \"2026-10-14T{end}:00Z\"\n\
             [receiver]\nmake = \"Leica\"\nmodel = \"GS10\"\n\
             [antenna]\nigs = \"LEIAS10\"\n\
             [height]\nmethod = \"vertical\"\nto = \"ARP\"\nvalue = 1.5\n"
        );
        Occupation::from_toml(&text).unwrap()
    }

    /// Morning: A, B and C overlap. Midday: D overlaps C by 10 minutes only.
    /// Afternoon: E and F. G is on A's mark at the same time as A.
    fn day() -> Vec<Occupation> {
        vec![
            occupation("F", "PM6", "13:00", "15:00"),
            occupation("A", "PM1", "08:00", "10:00"),
            occupation("B", "PM2", "08:30", "10:30"),
            occupation("C", "PM3", "09:00", "11:00"),
            occupation("D", "PM4", "10:50", "12:00"),
            occupation("E", "PM5", "13:30", "16:00"),
            occupation("G", "PM1", "08:15", "09:00"),
        ]
    }

    #[test]
    fn candidates_are_overlapping_occupations_of_different_marks() {
        let found = candidates(&day(), 15.0 * 60.0);
        let pairs: Vec<(&str, &str, f64)> = found
            .iter()
            .map(|c| (c.from.as_str(), c.to.as_str(), c.overlap_s() / 60.0))
            .collect();
        // A and G are the same mark; C and G only touch; D overlaps C by 10 minutes.
        assert_eq!(
            pairs,
            [
                ("A", "B", 90.0),
                ("A", "C", 60.0),
                ("B", "C", 90.0),
                ("B", "G", 30.0),
                ("E", "F", 90.0),
            ]
        );
        assert_eq!(
            found[0].start,
            Timestamp::parse("2026-10-14T08:30:00Z").unwrap()
        );
        // D overlaps C by 10 minutes: a baseline only with a lower threshold.
        assert!(
            candidates(&day(), 5.0 * 60.0)
                .iter()
                .any(|c| c.from.as_str() == "C" && c.to.as_str() == "D")
        );
    }

    #[test]
    fn sessions_are_groups_joined_by_baselines() {
        let found = sessions(&day(), 15.0 * 60.0);
        let ids: Vec<Vec<&str>> = found
            .iter()
            .map(|s| s.occupations.iter().map(|o| o.as_str()).collect())
            .collect();
        assert_eq!(ids, [vec!["A", "B", "C", "G"], vec!["D"], vec!["E", "F"]]);
        assert_eq!(
            found[0].end,
            Timestamp::parse("2026-10-14T11:00:00Z").unwrap()
        );
        // Lowering the threshold lets D join the morning.
        assert_eq!(sessions(&day(), 5.0 * 60.0).len(), 2);
        // The order of the input doesn't matter.
        let mut reversed = day();
        reversed.reverse();
        assert_eq!(sessions(&reversed, 15.0 * 60.0), found);
    }
}
