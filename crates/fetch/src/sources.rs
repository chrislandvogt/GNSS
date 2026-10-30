//! Data sources, as a jurisdiction pack declares them (`[[sources]]`).

use serde::Deserialize;

use crate::FetchError;

/// What a source is, which says how to find files in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    /// Geoscience Australia's GNSS Data Repository: its Web API lists
    /// RINEX files by station, type and time (`crate::ga`).
    GaArchive,
    /// Geoscience Australia's list of CORS sites, with positions.
    GaSites,
    /// CODE's products at AIUB (`crate::products`).
    Code,
    /// Files by name under a base URL.
    Files,
}

/// Whether the web app can download from a source itself, or needs the
/// relay or a data pack (ARCHITECTURE.md section 13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Browser {
    /// The host sends cross-origin headers.
    Direct,
    /// It doesn't, or it couldn't be checked.
    Relay,
}

/// A source of data.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// Its id, unique in the pack.
    pub id: String,
    /// Its name, for people.
    pub name: String,
    /// What it is.
    pub kind: SourceKind,
    /// The base URL.
    pub url: String,
    /// Every host files come from, redirects included: what a relay may
    /// forward to.
    pub hosts: Vec<String>,
    /// Whether the web app can reach it directly.
    pub browser: Browser,
    /// The data's licence.
    pub licence: String,
    /// What a report says about the data, word for word.
    pub attribution: String,
    /// For `files`: the files it has.
    #[serde(default)]
    pub files: Vec<String>,
    /// For `ga-archive`: stations to take broadcast navigation files from
    /// when the stations in use have no mixed-constellation file.
    #[serde(default)]
    pub nav_stations: Vec<String>,
}

impl Source {
    /// Whether a URL is on one of the source's hosts.
    pub fn allows(&self, url: &str) -> bool {
        host(url).is_some_and(|h| self.hosts.iter().any(|allowed| allowed == h))
    }
}

/// The host of an `https://` URL.
pub fn host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest.get(..end)?;
    (!host.is_empty() && !host.contains('@')).then_some(host)
}

/// The sources a pack declares, checked: unique ids, HTTPS only, and the
/// base URL's host among `hosts`.
pub fn read_sources(pack: &str) -> Result<Vec<Source>, FetchError> {
    #[derive(Deserialize)]
    struct Pack {
        #[serde(default)]
        sources: Vec<Source>,
    }
    let pack: Pack = toml::from_str(pack).map_err(|e| FetchError::Pack(e.message().to_owned()))?;
    let mut ids = std::collections::BTreeSet::new();
    for source in &pack.sources {
        if !ids.insert(source.id.as_str()) {
            return Err(FetchError::Pack(format!(
                "two sources are called {}",
                source.id
            )));
        }
        let url = &source.url;
        if host(url).is_none() {
            return Err(FetchError::Pack(format!(
                "source {}: {url} isn't an https:// URL",
                source.id
            )));
        }
        if !source.allows(url) {
            return Err(FetchError::Pack(format!(
                "source {}: {url}'s host isn't in its hosts",
                source.id
            )));
        }
    }
    Ok(pack.sources)
}

/// The first source of a kind.
pub fn find(sources: &[Source], kind: SourceKind) -> Option<&Source> {
    sources.iter().find(|s| s.kind == kind)
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;

    const AU: &str = include_str!("../../../packs/au/pack.toml");

    #[test]
    fn the_australian_pack_declares_its_sources() {
        let sources = read_sources(AU).unwrap();
        let ga = find(&sources, SourceKind::GaArchive).unwrap();
        assert_eq!(ga.browser, Browser::Direct);
        assert!(ga.attribution.contains("Geoscience Australia"));
        assert!(!ga.nav_stations.is_empty());
        let code = find(&sources, SourceKind::Code).unwrap();
        assert_eq!(code.browser, Browser::Relay);
        assert!(code.allows("https://zhw-b.s3.cloud.switch.ch/aiub/CODE/x"));
        assert!(!code.allows("https://example.org/aiub/CODE/x"));
        assert!(find(&sources, SourceKind::GaSites).is_some());
        let igs = find(&sources, SourceKind::Files).unwrap();
        assert!(igs.files.iter().any(|f| f == "igs20.atx"));
    }

    #[test]
    fn the_web_app_may_connect_to_every_host_the_browser_reaches_directly() {
        let headers = include_str!("../../../apps/web/ui/public/_headers");
        let connect = headers
            .split(';')
            .find_map(|d| d.trim().strip_prefix("connect-src"))
            .unwrap();
        let allowed: Vec<&str> = connect.split_whitespace().collect();
        for source in read_sources(AU).unwrap() {
            if source.browser == Browser::Direct {
                for host in &source.hosts {
                    let origin = format!("https://{host}");
                    assert!(
                        allowed.contains(&origin.as_str()),
                        "{origin} isn't in connect-src"
                    );
                }
            }
        }
        // And a relay on this machine.
        assert!(allowed.contains(&"http://127.0.0.1:*"));
    }

    #[test]
    fn hosts_are_https_only() {
        assert_eq!(host("https://a.example/x?y"), Some("a.example"));
        assert_eq!(host("https://a.example"), Some("a.example"));
        assert_eq!(host("http://a.example/x"), None);
        assert_eq!(host("https://user@a.example/"), None);
        let bad = r#"[[sources]]
id = "x"
name = "X"
kind = "files"
url = "https://a.example/"
hosts = ["b.example"]
browser = "relay"
licence = "?"
attribution = "?"
"#;
        assert!(read_sources(bad).is_err());
    }
}
