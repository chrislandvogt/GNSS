//! Fetching what a project's occupations need from outside it (M7): CORS
//! observations, navigation data, precise products and ANTEX, recorded in
//! `data.toml` (ARCHITECTURE.md section 13).

use gnss_fetch::plan::{Need, Want};
use gnss_fetch::sites::{Site, nearest};
use gnss_fetch::time::{Day, Window, format_time};
use gnss_fetch::{Fetched, Report};
use gnss_model::{DataFile, DataFiles, ModelError, Timestamp};
use gnss_store::{ProjectPath, Store, StoreError};
use hifitime::TimeScale;

use crate::folder::Loaded;

pub use gnss_fetch;

/// The Australian jurisdiction pack, the only one so far.
pub const AU_PACK: &str = include_str!("../../../packs/au/pack.toml");

/// Candidates beyond the number of nearest CORS wanted, tried in order of
/// distance where nearer ones have no data.
pub const SPARE_CORS: usize = 3;

/// Which CORS stations go with the occupations.
#[derive(Debug, Clone, PartialEq)]
pub enum Stations {
    /// These, for every occupation.
    Named(Vec<String>),
    /// The nearest public sites to each occupation's mark.
    Nearest {
        /// How many.
        count: usize,
        /// No further than this, in metres.
        within: f64,
    },
}

/// The CORS data each occupation needs, and the occupations that can't be
/// planned for (a mark with no approximate position, when stations are to
/// be chosen by distance).
pub fn wants(loaded: &Loaded, stations: &Stations, sites: &[Site]) -> (Vec<Want>, Vec<String>) {
    let mut wants = Vec::new();
    let mut problems = Vec::new();
    for occupation in &loaded.occupations {
        let (Some(start), Some(end)) = (gps_seconds(occupation.start), gps_seconds(occupation.end))
        else {
            problems.push(format!(
                "occupation {}: its times don't read",
                occupation.id
            ));
            continue;
        };
        let (chosen, count) = match stations {
            Stations::Named(names) => (names.iter().map(|s| s.to_uppercase()).collect(), None),
            Stations::Nearest { count, within } => {
                // Latitude and longitude in degrees, height in metres.
                let approx = loaded.marks.get(&occupation.mark).and_then(|m| m.approx);
                let Some(position) =
                    approx.map(|[lat, lon, h]| gnss_fetch::sites::ecef(lat, lon, h))
                else {
                    problems.push(format!(
                        "occupation {}: mark {} has no approximate position, so the nearest CORS can't be found; name stations instead",
                        occupation.id, occupation.mark
                    ));
                    continue;
                };
                // Spares, to fall back on where the nearest have no data.
                let near: Vec<String> = nearest(sites, position, *count + SPARE_CORS, *within)
                    .into_iter()
                    .map(|n| n.site.id.clone())
                    .collect();
                if near.is_empty() {
                    problems.push(format!(
                        "occupation {}: no public CORS within {:.0} km of mark {}",
                        occupation.id,
                        within / 1000.0,
                        occupation.mark
                    ));
                    continue;
                }
                (near, Some(*count))
            }
        };
        wants.push(Want {
            window: Window::new(start, end),
            stations: chosen,
            count,
        });
    }
    (wants, problems)
}

/// A time as GPS seconds since 1970-01-01 (whole seconds): archives name
/// and split their files by GPS days and hours, 18 s ahead of UTC since
/// 2017, so a file "for 2025-06-15" starts at 23:59:42 UTC the day before.
fn gps_seconds(t: Timestamp) -> Option<i64> {
    let (y, mo, d, h, mi, s, _) = t.epoch().to_gregorian(TimeScale::GPST);
    let day = Day::from_ymd(y, u32::from(mo), u32::from(d))?;
    Some(day.start() + i64::from(h) * 3600 + i64::from(mi) * 60 + i64::from(s))
}

/// What a need is for, in words: `data.toml`'s `purpose`. Times are GPS
/// time.
pub fn purpose(need: &Need) -> String {
    match need {
        Need::CorsObs { station, window } => {
            let gps = |t: i64| format_time(t).replace('T', " ").replace('Z', "");
            format!(
                "{station} observations, {} to {} GPS",
                gps(window.start),
                gps(window.end)
            )
        }
        Need::Nav { day } => format!("navigation data, {day}"),
        Need::Product { product, day } => format!("{product:?} product, {day}").to_lowercase(),
        Need::File { name, .. } => name.clone(),
    }
}

/// `data.toml` with what a fetch got added: read, merged and written.
pub fn record<S: Store + ?Sized>(store: &mut S, report: &Report) -> Result<DataFiles, RecordError> {
    let path = data_path();
    let mut data = match store.read(&path)? {
        Some(bytes) => DataFiles::from_toml(&String::from_utf8_lossy(&bytes))?,
        None => DataFiles::new(),
    };
    let files = report
        .fetched
        .iter()
        .flat_map(|f: &Fetched| {
            f.files.iter().map(|e| DataFile {
                purpose: purpose(&f.need),
                name: e.name.clone(),
                source: e.source.clone(),
                sha256: e.sha256.clone(),
                bytes: e.bytes,
                url: e.url.clone(),
                fetched: e.fetched.clone(),
                licence: e.licence.clone(),
                note: f.note.clone(),
            })
        })
        .collect();
    data.merge(files, report.attributions.iter().cloned());
    store.write(&path, data.to_toml().as_bytes())?;
    Ok(data)
}

fn data_path() -> ProjectPath {
    // A constant, valid path.
    ProjectPath::new("data.toml").unwrap_or_else(|_| unreachable!())
}

/// Why `data.toml` couldn't be updated.
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    /// It doesn't read.
    #[error("data.toml: {0}")]
    Model(#[from] ModelError),
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use gnss_fetch::sites::parse_sites;
    use gnss_fetch::{Entry, Fetched};
    use gnss_model::{MarkFile, MarkId, Occupation};
    use gnss_store::MemoryStore;

    use super::*;
    use crate::project::Project;

    fn loaded(approx: Option<[f64; 3]>) -> Loaded {
        let occupation = Occupation::from_toml(
            r#"schema = 1
id = "2025-06-15-PM1-A"
mark = "PM1"
start = "2025-06-15T03:10:00Z"
end = "2025-06-15T05:10:00Z"
[receiver]
make = "Trimble"
model = "R12"
[antenna]
igs = "TRM159800.00"
radome = "NONE"
[height]
method = "vertical"
to = "ARP"
value = 1.5
"#,
        )
        .unwrap();
        let mut marks = MarkFile::new();
        let mut mark = gnss_model::Mark::new(MarkId::new("PM1").unwrap());
        mark.approx = approx;
        marks.marks.push(mark);
        Loaded {
            project: Project::new(
                "Test",
                crate::project::AdjustmentSettings::new(gnss_datum::FrameId::Gda2020, None),
            ),
            marks,
            occupations: vec![occupation],
            overlays: Vec::new(),
            issues: Vec::new(),
        }
    }

    const SITES: &str = r#"{"_embedded":{"corsSites":[
      {"fourCharacterId":"SYDN","name":"Sydney","approximatePosition":{"coordinates":[-33.7808,151.1504,86.2]},"siteStatus":"PUBLIC"},
      {"fourCharacterId":"PERT","name":"Perth","approximatePosition":{"coordinates":[-31.8020,115.8853,12.8]},"siteStatus":"PUBLIC"}
    ]}}"#;

    #[test]
    fn occupations_want_cors_data_over_their_spans() {
        let sites = parse_sites(SITES).unwrap();
        let near_sydney = [-33.815, 151.003, 20.0];
        let (found, problems) = wants(
            &loaded(Some(near_sydney)),
            &Stations::Nearest {
                count: 2,
                within: 100_000.0,
            },
            &sites,
        );
        assert_eq!(problems, Vec::<String>::new());
        assert_eq!(found[0].stations, ["SYDN"]);
        assert_eq!(found[0].count, Some(2));
        assert_eq!(found[0].window.end - found[0].window.start, 7200);
        let (_, problems) = wants(
            &loaded(None),
            &Stations::Nearest {
                count: 2,
                within: 1e5,
            },
            &sites,
        );
        assert!(problems[0].contains("no approximate position"));
        let (named, _) = wants(&loaded(None), &Stations::Named(vec!["str1".into()]), &[]);
        assert_eq!(named[0].stations, ["STR1"]);
    }

    #[test]
    fn fetched_files_are_recorded_in_data_toml() {
        let mut store = MemoryStore::new();
        let day = Day::parse("2025-06-15").unwrap();
        let report = Report {
            fetched: vec![Fetched {
                need: Need::Nav { day },
                files: vec![Entry {
                    source: "ga-archive".into(),
                    name: "STR100AUS_R_20251660000_01D_MN.rnx.gz".into(),
                    sha256: "a".repeat(64),
                    bytes: 3,
                    url: "https://x.example/f".into(),
                    fetched: "2026-10-29T00:00:00Z".into(),
                    licence: "CC BY 4.0".into(),
                }],
                downloaded: true,
                note: None,
            }],
            problems: Vec::new(),
            attributions: ["GA data".to_owned()].into_iter().collect(),
        };
        let data = record(&mut store, &report).unwrap();
        assert_eq!(data.files[0].purpose, "navigation data, 2025-06-15");
        let text = String::from_utf8(store.read(&data_path()).unwrap().unwrap()).unwrap();
        assert!(
            text.contains("STR100AUS") && text.contains("GA data"),
            "{text}"
        );
        // Again: one entry still.
        assert_eq!(record(&mut store, &report).unwrap().files.len(), 1);
    }
}
