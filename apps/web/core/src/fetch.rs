//! Fetching a project's data from the browser (M7): the same planner and
//! fetcher as the CLI's `gnss fetch`, with HTTP and the cache supplied by
//! the page.
//!
//! The core worker can't wait on browser storage, which only answers
//! asynchronously, so the cache comes in as files: its index whole, and its
//! objects by name only (empty), except the CORS site list, which is read.
//! What the fetch adds to the cache goes back as files for the worker to
//! save. HTTP is a synchronous request from the worker, which a dedicated
//! worker may make.

use gnss_api::fetch::gnss_fetch::products::Constellations;
use gnss_api::fetch::gnss_fetch::time::Day;
use gnss_api::fetch::gnss_fetch::{self as f, Cache, Fetcher, Http, Route};
use gnss_api::fetch::{SiteLogCheck, Stations, check_site_logs, purpose, record};
use gnss_api::folder;
use gnss_store::{MemoryStore, ProjectPath, Store, StoreError};
use serde::{Deserialize, Serialize};

use crate::{ProjectFile, store};

/// What to fetch, from the page.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FetchOptions {
    /// CORS stations to use with every occupation; empty for the nearest.
    #[serde(default)]
    pub cors: Vec<String>,
    /// How many of the nearest CORS to use, without `cors`.
    pub nearest: usize,
    /// How far away a nearest CORS can be, in km.
    pub within_km: f64,
    /// Precise orbits, clocks and ERPs too (through the relay).
    pub precise: bool,
    /// The ANTEX file too (through the relay).
    pub antex: bool,
    /// A relay for the sources a browser can't reach, if the user turned
    /// one on.
    pub relay: Option<String>,
    /// Now, as the page's clock gives it: `2026-10-30T09:53:13.123Z`.
    pub now: String,
}

/// A file fetched or found in the cache.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FetchedFile {
    /// What it's for.
    pub purpose: String,
    /// Its name.
    pub name: String,
    /// Its size.
    pub bytes: u64,
    /// Whether it was downloaded now.
    pub downloaded: bool,
    /// Anything to know about it.
    pub note: Option<String>,
}

/// What a fetch did.
#[derive(Debug, Clone, Serialize)]
pub struct FetchOutcome {
    /// The files that meet the project's needs.
    pub fetched: Vec<FetchedFile>,
    /// What couldn't be fetched, and occupations that couldn't be planned for.
    pub problems: Vec<String>,
    /// Stations passed over, and why.
    pub notes: Vec<String>,
    /// What reports must say about the data.
    pub attributions: Vec<String>,
    /// CORS files' headers against their stations' site logs.
    pub site_logs: Vec<SiteLogCheck>,
    /// The project's `data.toml`, updated, for the page to save.
    pub data_toml: String,
    /// Files the fetch added to the cache, for the page to save.
    #[serde(skip)]
    pub cache_writes: Vec<ProjectFile>,
}

/// A store that remembers what was written to it.
struct Recording {
    inner: MemoryStore,
    written: Vec<ProjectPath>,
}

impl Store for Recording {
    fn read(&self, path: &ProjectPath) -> Result<Option<Vec<u8>>, StoreError> {
        self.inner.read(path)
    }

    fn write(&mut self, path: &ProjectPath, bytes: &[u8]) -> Result<(), StoreError> {
        if !self.written.contains(path) {
            self.written.push(path.clone());
        }
        self.inner.write(path, bytes)
    }

    fn list(&self, dir: &str) -> Result<Vec<ProjectPath>, StoreError> {
        self.inner.list(dir)
    }
}

/// Fetches what the project's occupations need, as `gnss fetch` does.
pub fn fetch_data<H: Http>(
    project: Vec<ProjectFile>,
    cache: Vec<ProjectFile>,
    options: &FetchOptions,
    http: H,
) -> Result<FetchOutcome, String> {
    let mut project = store(project)?;
    let loaded = folder::load(&project).map_err(|e| e.to_string())?;
    let sources = f::read_sources(gnss_api::fetch::AU_PACK).map_err(|e| e.to_string())?;
    let today = options
        .now
        .get(..10)
        .and_then(Day::parse)
        .ok_or("the page's clock doesn't read")?;
    let fetch_options = f::Options {
        precise: options.precise,
        antex: options.antex,
        constellations: Constellations::Operational,
        ..f::Options::default()
    };
    let mut fetcher = Fetcher {
        sources: &sources,
        http,
        cache: Cache::new(Recording {
            inner: store(cache)?,
            written: Vec::new(),
        }),
        today,
        now: format!("{}Z", options.now.get(..19).unwrap_or(&options.now)),
        options: fetch_options,
        route: match &options.relay {
            Some(relay) if !relay.trim().is_empty() => Route::Via(relay.trim().to_owned()),
            _ => Route::BrowserOnly,
        },
    };
    let stations = if options.cors.is_empty() {
        Stations::Nearest {
            count: options.nearest,
            within: options.within_km * 1000.0,
        }
    } else {
        Stations::Named(options.cors.clone())
    };
    let sites = match stations {
        Stations::Nearest { .. } => fetcher.sites().map_err(|e| e.to_string())?,
        Stations::Named(_) => Vec::new(),
    };
    let (mut wants, mut problems) = gnss_api::fetch::wants(&loaded, &stations, &sites);
    let notes = fetcher.choose_stations(&mut wants);
    let mut report = fetcher.fetch(&f::plan(&wants, &fetch_options));
    let site_logs = check_site_logs(&mut fetcher, &mut report);
    let data = record(&mut project, &report, &site_logs).map_err(|e| e.to_string())?;
    problems.extend(
        report
            .problems
            .iter()
            .map(|p| format!("{}: {}", purpose(&p.need), p.message)),
    );
    let recording = fetcher.cache.store();
    let mut cache_writes = Vec::new();
    for path in &recording.written {
        if let Some(data) = recording.read(path).map_err(|e| e.to_string())? {
            cache_writes.push(ProjectFile {
                path: path.to_string(),
                data,
            });
        }
    }
    Ok(FetchOutcome {
        fetched: report
            .fetched
            .iter()
            .flat_map(|x| {
                x.files.iter().map(|e| FetchedFile {
                    purpose: purpose(&x.need),
                    name: e.name.clone(),
                    bytes: e.bytes,
                    downloaded: x.downloaded,
                    note: x.note.clone(),
                })
            })
            .collect(),
        problems,
        notes,
        attributions: report.attributions.into_iter().collect(),
        site_logs,
        data_toml: data.to_toml(),
        cache_writes,
    })
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use std::collections::BTreeMap;

    use f::FetchError;

    use super::*;

    /// A fake web: URLs to bodies; anything else is a 404.
    #[derive(Default)]
    struct Fake {
        pages: BTreeMap<String, Vec<u8>>,
        gets: Vec<String>,
    }

    impl Http for &mut Fake {
        fn get(&mut self, url: &str) -> Result<Option<Vec<u8>>, FetchError> {
            self.gets.push(url.to_owned());
            Ok(self.pages.get(url).cloned())
        }
    }

    const API: &str = "https://data.gnss.ga.gov.au/api/v1/rinexFiles";
    const S3: &str =
        "https://geodesy-archive-permanent-storage-prod.s3.ap-southeast-2.amazonaws.com/default";

    fn file(path: &str, data: &str) -> ProjectFile {
        ProjectFile {
            path: path.into(),
            data: data.as_bytes().to_vec(),
        }
    }

    fn project() -> Vec<ProjectFile> {
        vec![
            file(
                "project.toml",
                "schema = 1\nname = \"Fetch\"\n\n[adjustment]\nframe = \"GDA2020\"\n",
            ),
            file(
                "marks.toml",
                "schema = 1\n\n[[mark]]\nid = \"PM1\"\napprox = [-33.815, 151.003, 20.0]\n",
            ),
            file(
                "occupations/2025-06-15-PM1-A.toml",
                "schema = 1\nid = \"2025-06-15-PM1-A\"\nmark = \"PM1\"\nstart = \"2025-06-15T03:10:00Z\"\nend = \"2025-06-15T05:10:00Z\"\n\n[receiver]\nmake = \"Trimble\"\nmodel = \"R12\"\n\n[antenna]\nigs = \"TRM159800.00\"\nradome = \"NONE\"\n\n[height]\nmethod = \"vertical\"\nto = \"ARP\"\nvalue = 1.5\n",
            ),
        ]
    }

    fn listing(name: &str, kind: &str, start: &str) -> Vec<u8> {
        format!(
            r#"[{{"siteId":"x","fileType":"{kind}","filePeriod":"01D","startDate":"{start}","rinexVersion":"3","fileLocation":"{S3}/{name}?sig=1","fileSize":4}}]"#
        )
        .into_bytes()
    }

    fn options() -> FetchOptions {
        FetchOptions {
            cors: vec!["SYDN".into()],
            nearest: 2,
            within_km: 100.0,
            precise: true,
            antex: false,
            relay: None,
            now: "2026-10-30T09:00:00.000Z".into(),
        }
    }

    #[test]
    fn a_browser_fetches_gas_data_and_says_what_needs_the_relay() {
        let mut web = Fake::default();
        let obs = "SYDN00AUS_R_20251660000_01D_30S_MO.crx.gz";
        let nav = "SYDN00AUS_R_20251660000_01D_MN.rnx.gz";
        // GPS time: the occupation is 03:10 to 05:10 UTC, so 03:05:18 to
        // 05:15:18 GPS with five minutes' margin.
        web.pages.insert(
            format!("{API}?stationId=SYDN&startDate=2025-06-15T03:05:18Z&endDate=2025-06-15T05:15:18Z&fileType=obs&rinexVersion=3&filePeriod=01D"),
            listing(obs, "obs", "2025-06-15T00:00:00Z"),
        );
        web.pages.insert(
            format!("{API}?stationId=SYDN&startDate=2025-06-15T00:00:00Z&endDate=2025-06-16T00:00:00Z&fileType=nav&rinexVersion=3&filePeriod=01D"),
            listing(nav, "nav", "2025-06-15T00:00:00Z"),
        );
        web.pages
            .insert(format!("{S3}/{obs}?sig=1"), b"obs!".to_vec());
        web.pages
            .insert(format!("{S3}/{nav}?sig=1"), b"nav!".to_vec());

        let outcome = fetch_data(project(), Vec::new(), &options(), &mut web).unwrap();
        let names: Vec<&str> = outcome.fetched.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, [obs, nav]);
        // CODE's products need the relay: said, not tried.
        assert!(
            outcome
                .problems
                .iter()
                .all(|p| p.contains("without the relay")),
            "{:?}",
            outcome.problems
        );
        assert_eq!(outcome.problems.len(), 3);
        assert!(web.gets.iter().all(|u| !u.contains("aiub")));
        assert!(
            outcome.data_toml.contains(obs) && outcome.data_toml.contains("Geoscience Australia")
        );
        // The cache gets two objects and their index entries.
        let paths: Vec<&str> = outcome
            .cache_writes
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert_eq!(
            paths.iter().filter(|p| p.starts_with("objects/")).count(),
            2,
            "{paths:?}"
        );
        assert!(paths.contains(&format!("index/ga-archive/{obs}.json").as_str()));

        // Again, with the cache as the page would pass it: index entries
        // whole, objects by name only. Nothing is downloaded.
        let cache: Vec<ProjectFile> = outcome
            .cache_writes
            .into_iter()
            .map(|f| {
                if f.path.starts_with("objects/") {
                    ProjectFile {
                        path: f.path,
                        data: Vec::new(),
                    }
                } else {
                    f
                }
            })
            .collect();
        web.gets.clear();
        let again = fetch_data(project(), cache, &options(), &mut web).unwrap();
        assert!(again.fetched.iter().all(|f| !f.downloaded));
        assert!(again.cache_writes.is_empty());
        assert!(web.gets.iter().all(|u| u.starts_with(API)));
    }
}
