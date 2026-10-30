//! The web app served on this machine (`gnss serve`, ARCHITECTURE.md
//! sections 14.3 and 15): a release bundle's files, from its zip or a
//! folder, with the response headers its `_headers` file gives (the same
//! Content Security Policy as the hosted app), and the relay beside it at
//! `/relay/`.
//!
//! It answers only requests addressed to this machine by the port it
//! listens on (`Host`), so a website can't reach it through DNS rebinding,
//! and only requests from its own pages (`Origin`, `Sec-Fetch-Site`), so
//! another site can't use its relay.

use std::collections::BTreeMap;
use std::path::Path;

use gnss_store::{MemoryStore, zip};

/// Where the relay is, under the site.
pub const RELAY: &str = "/relay";

/// A web app's files, as served.
#[derive(Debug, Clone, Default)]
pub struct Site {
    files: BTreeMap<String, Vec<u8>>,
    rules: Vec<(String, Vec<(String, String)>)>,
    port: u16,
}

impl Site {
    /// A release bundle's zip: its files, from the one top folder the
    /// release zips them in (`gnss-web-<version>/`).
    pub fn from_zip(bytes: &[u8], port: u16) -> Result<Self, String> {
        let store = zip::import(bytes).map_err(|e| e.to_string())?;
        Self::from_store(&store, port)
    }

    /// A folder holding the web app, such as a built `dist/`.
    pub fn from_dir(dir: &Path, port: u16) -> Result<Self, String> {
        let mut files = BTreeMap::new();
        read_dir(dir, "", &mut files)?;
        Self::new(files, port)
    }

    fn from_store(store: &MemoryStore, port: u16) -> Result<Self, String> {
        let files: BTreeMap<String, Vec<u8>> = store
            .files()
            .map(|(path, bytes)| (path.as_str().to_owned(), bytes.to_vec()))
            .collect();
        // The release zips the site in one folder.
        let top = files
            .keys()
            .find_map(|name| name.strip_suffix("/index.html"))
            .filter(|top| !top.contains('/'))
            .map(|top| format!("{top}/"));
        let files = match top {
            Some(top) if files.keys().all(|n| n.starts_with(&top)) => files
                .into_iter()
                .filter_map(|(n, b)| Some((n.strip_prefix(&top)?.to_owned(), b)))
                .collect(),
            _ => files,
        };
        Self::new(files, port)
    }

    fn new(mut files: BTreeMap<String, Vec<u8>>, port: u16) -> Result<Self, String> {
        if !files.contains_key("index.html") {
            return Err("it has no index.html, so it isn't the web app".into());
        }
        // Host configuration, not part of the site.
        let rules = files
            .remove("_headers")
            .map(|h| parse_headers(&String::from_utf8_lossy(&h)))
            .unwrap_or_default();
        Ok(Self { files, rules, port })
    }

    /// The port it's served on, which requests must address.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// How many files it has.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether it has no files (never, once made).
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Whether a `Host` header names this machine at the site's port.
    pub fn is_our_host(&self, host: &str) -> bool {
        let port = self.port;
        [
            format!("127.0.0.1:{port}"),
            format!("localhost:{port}"),
            format!("[::1]:{port}"),
        ]
        .iter()
        .any(|ours| host.eq_ignore_ascii_case(ours))
    }

    /// A file for a request's path (without its query), with its type:
    /// `/` is `index.html`.
    pub fn file(&self, path: &str) -> Option<(&[u8], &'static str)> {
        let name = match path.strip_prefix('/')? {
            "" => "index.html",
            name => name,
        };
        let bytes = self.files.get(name)?;
        Some((bytes, content_type(name)))
    }

    /// The headers `_headers` gives a path, in order.
    pub fn headers(&self, path: &str) -> Vec<(&str, &str)> {
        self.rules
            .iter()
            .filter(|(pattern, _)| matches(pattern, path))
            .flat_map(|(_, headers)| headers.iter().map(|(n, v)| (n.as_str(), v.as_str())))
            .collect()
    }
}

fn read_dir(dir: &Path, prefix: &str, files: &mut BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let rel = format!("{prefix}{name}");
        if path.is_dir() {
            read_dir(&path, &format!("{rel}/"), files)?;
        } else {
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            files.insert(rel, bytes);
        }
    }
    Ok(())
}

/// The rules in a `_headers` file (Cloudflare Pages and Netlify): a path
/// pattern on its own line, then its headers indented, `Name: value`.
fn parse_headers(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut rules: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with([' ', '\t']) {
            rules.push((line.trim().to_owned(), Vec::new()));
        } else if let (Some((_, headers)), Some((name, value))) =
            (rules.last_mut(), line.trim().split_once(':'))
        {
            headers.push((name.trim().to_owned(), value.trim().to_owned()));
        }
    }
    rules
}

/// Whether a path matches a pattern: exactly, or by its prefix where the
/// pattern ends in `*`.
fn matches(pattern: &str, path: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => path.starts_with(prefix),
        None => pattern == path,
    }
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("wasm") => "application/wasm",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADERS: &str = "# comment
/*
  Content-Security-Policy: default-src 'self'
  X-Content-Type-Options: nosniff

/sw.js
  Cache-Control: no-cache

/data/*
  Content-Type: application/octet-stream
";

    fn site() -> Site {
        let mut files = BTreeMap::new();
        files.insert("index.html".into(), b"<!doctype html>".to_vec());
        files.insert("sw.js".into(), b"//".to_vec());
        files.insert("data/a.atx.gz".into(), b"gz".to_vec());
        files.insert("_headers".into(), HEADERS.as_bytes().to_vec());
        Site::new(files, 8765).unwrap()
    }

    #[test]
    fn files_are_found_with_their_types_and_headers() {
        let site = site();
        assert_eq!(site.file("/").unwrap().1, "text/html; charset=utf-8");
        assert_eq!(site.file("/sw.js").unwrap().0, b"//");
        assert!(site.file("/_headers").is_none());
        assert!(site.file("/missing").is_none());
        assert_eq!(
            site.headers("/sw.js"),
            [
                ("Content-Security-Policy", "default-src 'self'"),
                ("X-Content-Type-Options", "nosniff"),
                ("Cache-Control", "no-cache")
            ]
        );
        assert_eq!(site.headers("/data/a.atx.gz").len(), 3);
        assert!(site.is_our_host("127.0.0.1:8765"));
        assert!(site.is_our_host("LOCALHOST:8765"));
        assert!(!site.is_our_host("127.0.0.1:80"));
        assert!(!site.is_our_host("evil.example:8765"));
    }

    #[test]
    fn a_release_zip_is_served_from_its_top_folder() {
        let mut store = MemoryStore::new();
        for (name, bytes) in [
            ("gnss-web-1.0.0/index.html", &b"<!doctype html>"[..]),
            ("gnss-web-1.0.0/assets/app.js", b"//"),
            ("gnss-web-1.0.0/LICENSE", b"Apache"),
        ] {
            let _ = gnss_store::Store::write(
                &mut store,
                &gnss_store::ProjectPath::new(name).unwrap(),
                bytes,
            );
        }
        let site = Site::from_zip(&zip::export(&store).unwrap(), 1).unwrap();
        assert_eq!(site.len(), 3);
        assert!(site.file("/assets/app.js").is_some());
        assert!(Site::from_zip(&zip::export(&MemoryStore::new()).unwrap(), 1).is_err());
    }
}
