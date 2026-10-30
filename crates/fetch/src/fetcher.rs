//! Turning needs into files: finding each in its source, and downloading it
//! into the cache unless it's there already.
//!
//! Network access goes through [`Http`], so the same code runs natively
//! (`crate::native`), in the browser, and against a fake in tests.

use std::collections::{BTreeMap, BTreeSet};

use gnss_store::Store;

use crate::FetchError;
use crate::cache::{Cache, Entry, without_query};
use crate::ga::{self, FileType, Period};
use crate::plan::{Need, Options, Want};
use crate::products::{self, Latency};
use crate::relay::Route;
use crate::sites::{Site, parse_sites};
use crate::sources::{Source, SourceKind, find};
use crate::time::{Day, Window, format_time};

/// Days a cached site list is used before it's fetched again.
pub const SITES_MAX_AGE_DAYS: i64 = 30;

/// Fetching over HTTP.
pub trait Http {
    /// The body of a GET, following redirects, or `None` if the server says
    /// there's no such file (404). Any other failure is an error.
    fn get(&mut self, url: &str) -> Result<Option<Vec<u8>>, FetchError>;
}

/// A need met.
#[derive(Debug, Clone, PartialEq)]
pub struct Fetched {
    /// The need.
    pub need: Need,
    /// The files that meet it, as cached.
    pub files: Vec<Entry>,
    /// Whether any of them was downloaded now, rather than already cached.
    pub downloaded: bool,
    /// Anything to know about it, such as a lesser product used because the
    /// best isn't out yet.
    pub note: Option<String>,
}

/// A need not met, or only partly.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// The need.
    pub need: Need,
    /// What went wrong.
    pub message: String,
}

/// What a fetch did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    /// The needs met.
    pub fetched: Vec<Fetched>,
    /// The needs not met, or only partly.
    pub problems: Vec<Problem>,
    /// Attributions and citations for the data fetched, for reports.
    pub attributions: BTreeSet<String>,
}

/// Fetches needs from a pack's sources into a cache.
pub struct Fetcher<'a, H: Http, S: Store> {
    /// The pack's sources.
    pub sources: &'a [Source],
    /// The network.
    pub http: H,
    /// Where files are kept.
    pub cache: Cache<S>,
    /// Today, for which product series can be out.
    pub today: Day,
    /// Now, as `YYYY-MM-DDTHH:MM:SSZ`, recorded with each download.
    pub now: String,
    /// What was asked for.
    pub options: Options,
    /// How sources are reached: directly, or some through a relay.
    pub route: Route,
}

impl<H: Http, S: Store> Fetcher<'_, H, S> {
    /// Meets every need it can, and says what it couldn't.
    pub fn fetch(&mut self, needs: &[Need]) -> Report {
        let mut report = Report::default();
        // Stations whose own navigation files are tried first.
        let stations: Vec<String> = needs
            .iter()
            .filter_map(|n| match n {
                Need::CorsObs { station, .. } => Some(station.clone()),
                _ => None,
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for need in needs {
            let result = match need {
                Need::CorsObs { station, window } => self.cors_obs(station, *window),
                Need::Nav { day } => self.nav(*day, &stations),
                Need::Product { product, day } => self.product(product.product(), *day),
                Need::File { source, name } => self.file(source, name),
            };
            match result {
                Ok(Met {
                    files,
                    downloaded,
                    note,
                    problem,
                    attribution,
                }) => {
                    report.attributions.extend(attribution);
                    if let Some(message) = problem {
                        report.problems.push(Problem {
                            need: need.clone(),
                            message,
                        });
                    }
                    if !files.is_empty() {
                        report.fetched.push(Fetched {
                            need: need.clone(),
                            files,
                            downloaded,
                            note,
                        });
                    }
                }
                Err(e) => report.problems.push(Problem {
                    need: need.clone(),
                    message: e.to_string(),
                }),
            }
        }
        report
    }

    /// The CORS sites the pack's site list names, from the cache if it was
    /// fetched in the last [`SITES_MAX_AGE_DAYS`] days.
    pub fn sites(&mut self) -> Result<Vec<Site>, FetchError> {
        let source = self.source(SourceKind::GaSites)?.clone();
        let name = "corsSites.json";
        if let Some(entry) = self.cache.get(&source.id, name)? {
            let age = Day::parse(entry.fetched.get(..10).unwrap_or_default())
                .map(|d| d.days_until(self.today));
            if age.is_some_and(|a| a <= SITES_MAX_AGE_DAYS)
                && let Some(bytes) = self.cache.read(&entry)?
            {
                return parse_sites(&String::from_utf8_lossy(&bytes));
            }
        }
        let url = format!("{}?size=10000", source.url);
        let data = self.get(&source, &url)?.ok_or_else(|| FetchError::Http {
            url: url.clone(),
            message: "not found".into(),
        })?;
        // Only a list that reads is kept.
        let sites = parse_sites(&String::from_utf8_lossy(&data))?;
        let entry = Entry {
            source: source.id.clone(),
            name: name.into(),
            sha256: String::new(),
            bytes: 0,
            url,
            fetched: self.now.clone(),
            licence: source.licence.clone(),
        };
        self.cache.put(entry, &data)?;
        Ok(sites)
    }

    /// For each want with a count, keeps the first of its candidate
    /// stations, in order, whose files cover its CORS window by the
    /// archive's listings, and says which were passed over and why. Wants
    /// without a count are left alone. Nothing is downloaded.
    pub fn choose_stations(&mut self, wants: &mut [Want]) -> Vec<String> {
        let mut notes = Vec::new();
        let mut seen: BTreeMap<(String, Window), Result<bool, String>> = BTreeMap::new();
        for want in wants.iter_mut() {
            let Some(count) = want.count else { continue };
            let window = want.cors_window(self.options.pad);
            let mut chosen = Vec::new();
            for station in &want.stations {
                if chosen.len() == count {
                    break;
                }
                let key = (station.clone(), window);
                let found = match seen.get(&key) {
                    Some(found) => found.clone(),
                    None => {
                        let found = self.covers(station, window).map_err(|e| e.to_string());
                        seen.insert(key, found.clone());
                        found
                    }
                };
                match found {
                    Ok(true) => chosen.push(station.clone()),
                    Ok(false) => notes.push(format!(
                        "{station} has no files covering {} to {} GPS, so the next nearest was tried",
                        format_time(window.start),
                        format_time(window.end)
                    )),
                    Err(e) => notes.push(format!("{station} couldn't be checked ({e}), so the next nearest was tried")),
                }
            }
            if chosen.len() < count {
                notes.push(format!(
                    "only {} of the {count} nearest CORS wanted have data from {} to {} GPS",
                    chosen.len(),
                    format_time(window.start),
                    format_time(window.end)
                ));
            }
            want.stations = chosen;
            want.count = None;
        }
        notes.sort();
        notes.dedup();
        notes
    }

    /// Whether the archive's files from a station cover a window.
    fn covers(&mut self, station: &str, window: Window) -> Result<bool, FetchError> {
        let source = self.source(SourceKind::GaArchive)?.clone();
        let daily = self.listing(&source, station, window, FileType::Obs, Period::Daily)?;
        let hourly = self.listing(&source, station, window, FileType::Obs, Period::Hourly)?;
        let choice = ga::choose(&daily, &hourly, window);
        Ok(!choice.files.is_empty() && choice.missing.is_empty())
    }

    /// A GET of a source's URL, by the route.
    fn get(&mut self, source: &Source, url: &str) -> Result<Option<Vec<u8>>, FetchError> {
        let routed = self.route.url(source, url).ok_or_else(|| FetchError::Http {
            url: crate::cache::without_query(url).to_owned(),
            message: format!(
                "{} can't be reached from a browser without the relay; turn the relay on, or bring the file in a data pack",
                source.name
            ),
        })?;
        self.http.get(&routed)
    }

    fn source(&self, kind: SourceKind) -> Result<&Source, FetchError> {
        find(self.sources, kind).ok_or(FetchError::NoSource(kind))
    }

    /// A file from the cache, or downloaded into it. `None` if the source
    /// hasn't got it.
    fn obtain(
        &mut self,
        source: &Source,
        name: &str,
        url: &str,
        bytes: Option<u64>,
    ) -> Result<Option<(Entry, bool)>, FetchError> {
        if let Some(entry) = self.cache.get(&source.id, name)? {
            return Ok(Some((entry, false)));
        }
        if !source.allows(url) {
            return Err(FetchError::Host(url.to_owned()));
        }
        let Some(data) = self.get(source, url)? else {
            return Ok(None);
        };
        if let Some(expected) = bytes
            && expected != data.len() as u64
        {
            return Err(FetchError::Truncated {
                name: name.to_owned(),
                got: data.len() as u64,
                expected,
            });
        }
        let entry = Entry {
            source: source.id.clone(),
            name: name.to_owned(),
            sha256: String::new(),
            bytes: 0,
            url: without_query(url).to_owned(),
            fetched: self.now.clone(),
            licence: source.licence.clone(),
        };
        Ok(Some((self.cache.put(entry, &data)?, true)))
    }

    fn listing(
        &mut self,
        source: &Source,
        station: &str,
        window: Window,
        file: FileType,
        period: Period,
    ) -> Result<Vec<ga::RemoteFile>, FetchError> {
        let url = ga::listing_url(&source.url, station, window, file, period);
        match self.get(source, &url)? {
            None => Ok(Vec::new()),
            Some(body) => ga::parse_listing(&String::from_utf8_lossy(&body)),
        }
    }

    fn cors_obs(&mut self, station: &str, window: Window) -> Result<Met, FetchError> {
        let source = self.source(SourceKind::GaArchive)?.clone();
        let daily = self.listing(&source, station, window, FileType::Obs, Period::Daily)?;
        let hourly = self.listing(&source, station, window, FileType::Obs, Period::Hourly)?;
        let choice = ga::choose(&daily, &hourly, window);
        let mut met = Met::from(&source);
        for file in &choice.files {
            match self.obtain(&source, &file.name, &file.url, file.bytes)? {
                Some((entry, downloaded)) => {
                    met.downloaded |= downloaded;
                    met.files.push(entry);
                }
                None => {
                    met.problem = Some(format!(
                        "{} is listed but couldn't be downloaded",
                        file.name
                    ))
                }
            }
        }
        if met.files.is_empty() {
            met.problem = Some(format!(
                "the archive has no RINEX 3 observations from {station} then"
            ));
        } else if !choice.missing.is_empty() {
            let gaps: Vec<String> = choice
                .missing
                .iter()
                .map(|w| {
                    format!(
                        "{} to {}",
                        crate::time::format_time(w.start),
                        crate::time::format_time(w.end)
                    )
                })
                .collect();
            met.problem = Some(format!("no files from {station} for {}", gaps.join(", ")));
        }
        Ok(met)
    }

    fn nav(&mut self, day: Day, stations: &[String]) -> Result<Met, FetchError> {
        let source = self.source(SourceKind::GaArchive)?.clone();
        let candidates: Vec<String> = stations
            .iter()
            .chain(&source.nav_stations)
            .cloned()
            .collect::<Vec<_>>();
        let mut tried = BTreeSet::new();
        for station in candidates {
            if !tried.insert(station.clone()) {
                continue;
            }
            let files = self.listing(
                &source,
                &station,
                day.window(),
                FileType::Nav,
                Period::Daily,
            )?;
            let Some(file) = ga::mixed_nav(&files).filter(|f| f.window.start == day.start()) else {
                continue;
            };
            if let Some((entry, downloaded)) =
                self.obtain(&source, &file.name, &file.url, file.bytes)?
            {
                let mut met = Met::from(&source);
                met.downloaded = downloaded;
                met.files.push(entry);
                if !stations.contains(&station) {
                    met.note = Some(format!(
                        "from {station}: no station in use has a mixed navigation file"
                    ));
                }
                return Ok(met);
            }
        }
        let mut met = Met::from(&source);
        met.problem = Some(format!(
            "no mixed-constellation navigation file for {day} from {}",
            tried.into_iter().collect::<Vec<_>>().join(", ")
        ));
        Ok(met)
    }

    fn product(&mut self, product: products::Product, day: Day) -> Result<Met, FetchError> {
        let source = self.source(SourceKind::Code)?.clone();
        let candidates =
            products::candidates(product, self.options.constellations, day, self.today);
        let best = candidates.first().map(|c| (c.latency, c.constellations));
        for candidate in &candidates {
            let url = format!("{}/{}", source.url.trim_end_matches('/'), candidate.path);
            if let Some((entry, downloaded)) = self.obtain(&source, candidate.name(), &url, None)? {
                let mut met = Met::from(&source);
                met.attribution.push(candidate.citation().to_owned());
                met.downloaded = downloaded;
                met.files.push(entry);
                if best != Some((candidate.latency, candidate.constellations)) {
                    met.note = Some(format!(
                        "{} {}: better products aren't out yet",
                        candidate.latency.name(),
                        match candidate.constellations {
                            products::Constellations::Multi => "multi-GNSS",
                            products::Constellations::Operational => "GPS, GLONASS and Galileo",
                        }
                    ));
                } else if candidate.latency != Latency::Final {
                    met.note = Some(format!(
                        "{}: finals aren't out yet",
                        candidate.latency.name()
                    ));
                }
                return Ok(met);
            }
        }
        let mut met = Met::from(&source);
        met.problem = Some(format!("CODE has no {product:?} product for {day} yet"));
        Ok(met)
    }

    fn file(&mut self, source_id: &str, name: &str) -> Result<Met, FetchError> {
        let source = self
            .sources
            .iter()
            .find(|s| s.id == source_id)
            .cloned()
            .ok_or(FetchError::NoSource(SourceKind::Files))?;
        let url = format!("{}{name}", source.url);
        let mut met = Met::from(&source);
        match self.obtain(&source, name, &url, None)? {
            Some((entry, downloaded)) => {
                met.downloaded = downloaded;
                met.files.push(entry);
            }
            None => met.problem = Some(format!("{} has no {name}", source.name)),
        }
        Ok(met)
    }
}

/// What meeting one need came to.
struct Met {
    files: Vec<Entry>,
    downloaded: bool,
    note: Option<String>,
    problem: Option<String>,
    attribution: Vec<String>,
}

impl From<&Source> for Met {
    fn from(source: &Source) -> Self {
        Self {
            files: Vec::new(),
            downloaded: false,
            note: None,
            problem: None,
            attribution: vec![source.attribution.clone()],
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use std::collections::BTreeMap;

    use gnss_store::MemoryStore;

    use super::*;
    use crate::plan::{ProductKind, Want, plan};
    use crate::sources::read_sources;
    use crate::time::parse_time;

    /// A fake web: URLs to bodies; anything else is a 404.
    #[derive(Default)]
    struct Fake {
        pages: BTreeMap<String, Vec<u8>>,
        gets: Vec<String>,
    }

    impl Http for Fake {
        fn get(&mut self, url: &str) -> Result<Option<Vec<u8>>, FetchError> {
            self.gets.push(url.to_owned());
            Ok(self.pages.get(url).cloned())
        }
    }

    const API: &str = "https://data.gnss.ga.gov.au/api/v1/rinexFiles";
    const S3: &str =
        "https://geodesy-archive-permanent-storage-prod.s3.ap-southeast-2.amazonaws.com/default";

    fn listing(name: &str, period: &str, start: &str, size: usize) -> Vec<u8> {
        format!(
            r#"[{{"siteId":"x","fileType":"obs","filePeriod":"{period}","startDate":"{start}","rinexVersion":"3","fileLocation":"{S3}/{name}?X-Amz-Signature=abc","fileSize":{size}}}]"#
        )
        .into_bytes()
    }

    fn web() -> Fake {
        let mut fake = Fake::default();
        let obs = "SYDN00AUS_R_20251660000_01D_30S_MO.crx.gz";
        let nav = "STR100AUS_R_20251660000_01D_MN.rnx.gz";
        fake.pages.insert(
            format!("{API}?stationId=SYDN&startDate=2025-06-15T00:55:00Z&endDate=2025-06-15T05:05:00Z&fileType=obs&rinexVersion=3&filePeriod=01D"),
            listing(obs, "01D", "2025-06-15T00:00:00Z", 6),
        );
        fake.pages.insert(
            format!("{API}?stationId=STR1&startDate=2025-06-15T00:00:00Z&endDate=2025-06-16T00:00:00Z&fileType=nav&rinexVersion=3&filePeriod=01D"),
            listing(nav, "01D", "2025-06-15T00:00:00Z", 3),
        );
        fake.pages.insert(
            format!("{S3}/{obs}?X-Amz-Signature=abc"),
            b"crxgz!".to_vec(),
        );
        fake.pages
            .insert(format!("{S3}/{nav}?X-Amz-Signature=abc"), b"nav".to_vec());
        fake.pages.insert(
            "https://download.aiub.unibe.ch/CODE/2025/COD0OPSFIN_20251660000_01D_05M_ORB.SP3.gz"
                .into(),
            b"sp3".to_vec(),
        );
        fake.pages.insert(
            "https://files.igs.org/pub/station/general/igs20.atx".into(),
            b"atx".to_vec(),
        );
        fake
    }

    #[test]
    fn needs_are_met_from_the_sources_and_then_from_the_cache() {
        let sources = read_sources(include_str!("../../../packs/au/pack.toml")).unwrap();
        let wants = [Want {
            window: Window::new(
                parse_time("2025-06-15T01:00:00Z").unwrap(),
                parse_time("2025-06-15T05:00:00Z").unwrap(),
            ),
            stations: vec!["SYDN".into()],
            count: None,
        }];
        let mut needs = plan(&wants, &Options::default());
        needs.push(Need::Product {
            product: ProductKind::Orbit,
            day: Day::parse("2025-06-15").unwrap(),
        });
        let mut fetcher = Fetcher {
            sources: &sources,
            http: web(),
            cache: Cache::new(MemoryStore::default()),
            today: Day::parse("2026-10-29").unwrap(),
            now: "2026-10-29T00:00:00Z".into(),
            options: Options::default(),
            route: Route::Direct,
        };
        let report = fetcher.fetch(&needs);
        assert_eq!(report.problems, []);
        let names: Vec<&str> = report
            .fetched
            .iter()
            .flat_map(|f| f.files.iter().map(|e| e.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                "SYDN00AUS_R_20251660000_01D_30S_MO.crx.gz",
                // SYDN has no mixed navigation file, so a fallback's is used.
                "STR100AUS_R_20251660000_01D_MN.rnx.gz",
                "igs20.atx",
                "COD0OPSFIN_20251660000_01D_05M_ORB.SP3.gz",
            ]
        );
        let nav = &report.fetched[1];
        assert!(nav.note.as_deref().unwrap().contains("STR1"));
        assert!(report.fetched.iter().all(|f| f.downloaded));
        // Signatures aren't kept.
        assert!(report.fetched[0].files[0].url.ends_with(".crx.gz"));
        assert!(
            report
                .attributions
                .iter()
                .any(|a| a.contains("10.48620/97773"))
        );
        assert!(
            report
                .attributions
                .iter()
                .any(|a| a.contains("Geoscience Australia"))
        );

        // Again: listings are asked for, but nothing is downloaded.
        fetcher.http.gets.clear();
        let again = fetcher.fetch(&needs);
        assert!(again.fetched.iter().all(|f| !f.downloaded));
        assert!(
            fetcher
                .http
                .gets
                .iter()
                .all(|u| !u.contains("X-Amz") && !u.contains("aiub") && !u.contains("igs20"))
        );
    }

    #[test]
    fn what_cant_be_found_is_reported() {
        let sources = read_sources(include_str!("../../../packs/au/pack.toml")).unwrap();
        let mut fetcher = Fetcher {
            sources: &sources,
            http: Fake::default(),
            cache: Cache::new(MemoryStore::default()),
            today: Day::parse("2026-10-29").unwrap(),
            now: "2026-10-29T00:00:00Z".into(),
            options: Options::default(),
            route: Route::Direct,
        };
        let day = Day::parse("2025-06-15").unwrap();
        let report = fetcher.fetch(&[
            Need::CorsObs {
                station: "ZZZZ".into(),
                window: day.window(),
            },
            Need::Nav { day },
            Need::Product {
                product: ProductKind::Clock,
                day,
            },
        ]);
        assert_eq!(report.fetched, []);
        assert_eq!(report.problems.len(), 3);
        assert!(report.problems[0].message.contains("ZZZZ"));
        assert!(report.problems[1].message.contains("TID1"));
        assert!(report.problems[2].message.contains("Clock"));
    }

    #[test]
    fn the_site_list_is_cached_for_a_month() {
        let sources = read_sources(include_str!("../../../packs/au/pack.toml")).unwrap();
        let mut web = Fake::default();
        web.pages.insert(
            "https://gws.geodesy.ga.gov.au/corsSites?size=10000".into(),
            br#"{"_embedded":{"corsSites":[{"fourCharacterId":"SYDN","name":"Sydney","approximatePosition":{"coordinates":[-33.78,151.15,86.2]},"siteStatus":"PUBLIC"}]}}"#.to_vec(),
        );
        let mut fetcher = Fetcher {
            sources: &sources,
            http: web,
            cache: Cache::new(MemoryStore::default()),
            today: Day::parse("2026-10-29").unwrap(),
            now: "2026-10-29T00:00:00Z".into(),
            options: Options::default(),
            route: Route::Direct,
        };
        assert_eq!(fetcher.sites().unwrap()[0].id, "SYDN");
        assert_eq!(fetcher.sites().unwrap().len(), 1);
        assert_eq!(fetcher.http.gets.len(), 1);
        fetcher.today = Day::parse("2026-12-29").unwrap();
        fetcher.sites().unwrap();
        assert_eq!(fetcher.http.gets.len(), 2);
    }

    #[test]
    fn the_nearest_stations_with_data_are_chosen() {
        let sources = read_sources(include_str!("../../../packs/au/pack.toml")).unwrap();
        let mut fetcher = Fetcher {
            sources: &sources,
            http: web(),
            cache: Cache::new(MemoryStore::default()),
            today: Day::parse("2026-10-29").unwrap(),
            now: "2026-10-29T00:00:00Z".into(),
            options: Options::default(),
            route: Route::Direct,
        };
        // ZZZZ is nearest but has nothing; SYDN does; STR1 isn't needed.
        let window = Window::new(
            parse_time("2025-06-15T01:00:00Z").unwrap(),
            parse_time("2025-06-15T05:00:00Z").unwrap(),
        );
        let mut wants = [Want {
            window,
            stations: vec!["ZZZZ".into(), "SYDN".into(), "STR1".into()],
            count: Some(1),
        }];
        let notes = fetcher.choose_stations(&mut wants);
        assert_eq!(wants[0].stations, ["SYDN"]);
        assert_eq!(wants[0].count, None);
        assert!(
            notes[0].starts_with("ZZZZ has no files covering 2025-06-15T00:55:00Z"),
            "{notes:?}"
        );
        // Nothing was downloaded.
        assert!(fetcher.http.gets.iter().all(|u| u.starts_with(API)));
        let mut short = [Want {
            window,
            stations: vec!["ZZZZ".into()],
            count: Some(2),
        }];
        let notes = fetcher.choose_stations(&mut short);
        assert!(
            notes.iter().any(|n| n.starts_with("only 0 of the 2")),
            "{notes:?}"
        );
    }

    #[test]
    fn a_browser_without_the_relay_gets_ga_data_but_not_codes() {
        let sources = read_sources(include_str!("../../../packs/au/pack.toml")).unwrap();
        let mut fetcher = Fetcher {
            sources: &sources,
            http: web(),
            cache: Cache::new(MemoryStore::default()),
            today: Day::parse("2026-10-29").unwrap(),
            now: "2026-10-29T00:00:00Z".into(),
            options: Options::default(),
            route: Route::BrowserOnly,
        };
        let day = Day::parse("2025-06-15").unwrap();
        let report = fetcher.fetch(&[
            Need::Nav { day },
            Need::Product {
                product: ProductKind::Orbit,
                day,
            },
        ]);
        assert_eq!(report.fetched.len(), 1);
        assert!(
            report.problems[0].message.contains("without the relay"),
            "{:?}",
            report.problems
        );
        // Nothing was asked of CODE.
        assert!(fetcher.http.gets.iter().all(|u| !u.contains("aiub")));

        // Through a relay, CODE's file is asked of the relay.
        fetcher.route = Route::Via("http://relay.example".into());
        let relayed = "http://relay.example/?url=https%3A%2F%2Fdownload.aiub.unibe.ch%2FCODE%2F2025%2FCOD0OPSFIN_20251660000_01D_05M_ORB.SP3.gz";
        fetcher.http.pages.insert(relayed.into(), b"sp3".to_vec());
        let report = fetcher.fetch(&[Need::Product {
            product: ProductKind::Orbit,
            day,
        }]);
        assert_eq!(report.problems, []);
        assert_eq!(
            report.fetched[0].files[0].url,
            "https://download.aiub.unibe.ch/CODE/2025/COD0OPSFIN_20251660000_01D_05M_ORB.SP3.gz"
        );
    }

    #[test]
    fn a_short_download_is_an_error() {
        let sources = read_sources(include_str!("../../../packs/au/pack.toml")).unwrap();
        let mut web = web();
        let obs = "SYDN00AUS_R_20251660000_01D_30S_MO.crx.gz";
        web.pages
            .insert(format!("{S3}/{obs}?X-Amz-Signature=abc"), b"crx".to_vec());
        let mut fetcher = Fetcher {
            sources: &sources,
            http: web,
            cache: Cache::new(MemoryStore::default()),
            today: Day::parse("2026-10-29").unwrap(),
            now: "2026-10-29T00:00:00Z".into(),
            options: Options::default(),
            route: Route::Direct,
        };
        let report = fetcher.fetch(&[Need::CorsObs {
            station: "SYDN".into(),
            window: Window::new(
                parse_time("2025-06-15T00:55:00Z").unwrap(),
                parse_time("2025-06-15T05:05:00Z").unwrap(),
            ),
        }]);
        assert!(
            report.problems[0].message.contains("3 of 6 bytes"),
            "{:?}",
            report.problems
        );
    }
}
