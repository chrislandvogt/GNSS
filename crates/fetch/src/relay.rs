//! The relay's address scheme, shared by the fetcher and the relay itself
//! (`apps/relay`, ARCHITECTURE.md section 13).
//!
//! A browser can only download from hosts that send cross-origin headers.
//! For the sources that don't, the web app asks a relay instead:
//! `GET <relay>/?url=<the file's URL, percent-encoded>`. The relay forwards
//! the request only to hosts the jurisdiction packs declare, follows
//! redirects only to those hosts, and keeps no logs or copies.

use crate::sources::{Browser, Source, host};

/// How a fetch reaches its sources.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Route {
    /// Every source directly: natively, where cross-origin rules don't apply.
    #[default]
    Direct,
    /// Sources a browser can't reach go through this relay, such as
    /// `http://127.0.0.1:8787`; the rest directly.
    Via(String),
    /// In a browser with no relay: sources it can't reach are refused,
    /// rather than tried and failed.
    BrowserOnly,
}

impl Route {
    /// The URL to request for a file of a source, or `None` if this route
    /// can't reach it.
    pub fn url(&self, source: &Source, url: &str) -> Option<String> {
        match (self, source.browser) {
            (Route::Direct, _) | (_, Browser::Direct) => Some(url.to_owned()),
            (Route::Via(relay), Browser::Relay) => Some(relay_url(relay, url)),
            (Route::BrowserOnly, Browser::Relay) => None,
        }
    }
}

/// The relay's address for a URL.
pub fn relay_url(relay: &str, url: &str) -> String {
    format!("{}/?url={}", relay.trim_end_matches('/'), encode(url))
}

/// The URL a relay request asks for: the `url` parameter of a request
/// target such as `/?url=https%3A%2F%2F…`, decoded.
pub fn requested(target: &str) -> Option<String> {
    let query = target.split_once('?')?.1;
    let value = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("url="))?;
    decode(value)
}

/// Whether a URL is one a relay may fetch: HTTPS, on a host some source
/// declares.
pub fn allowed(sources: &[Source], url: &str) -> bool {
    host(url).is_some() && sources.iter().any(|s| s.allows(url))
}

/// Percent-encodes everything but unreserved characters (RFC 3986).
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decodes percent-encoding (and `+` as a space), or `None` if it's
/// malformed or not UTF-8.
pub fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;
    use crate::sources::{SourceKind, find, read_sources};

    #[test]
    fn urls_survive_the_round_trip() {
        let url = "https://x.example/a b/f.gz?X-Amz-Signature=a%2Fb&c=d";
        let target = relay_url("http://127.0.0.1:8787/", url);
        assert!(target.starts_with("http://127.0.0.1:8787/?url=https%3A%2F%2Fx.example"));
        let path = target.trim_start_matches("http://127.0.0.1:8787");
        assert_eq!(requested(path).as_deref(), Some(url));
        assert_eq!(requested("/?other=1"), None);
        assert_eq!(decode("%zz"), None);
        assert_eq!(decode("%4"), None);
    }

    #[test]
    fn routes_send_only_what_needs_it_through_the_relay() {
        let sources = read_sources(include_str!("../../../packs/au/pack.toml")).unwrap();
        let ga = find(&sources, SourceKind::GaArchive).unwrap();
        let code = find(&sources, SourceKind::Code).unwrap();
        let a = "https://data.gnss.ga.gov.au/api/v1/rinexFiles?x=1";
        let c = "https://download.aiub.unibe.ch/CODE/x.gz";
        assert_eq!(Route::Direct.url(code, c).as_deref(), Some(c));
        let via = Route::Via("http://r.example".into());
        assert_eq!(via.url(ga, a).as_deref(), Some(a));
        assert!(
            via.url(code, c)
                .unwrap()
                .starts_with("http://r.example/?url=https%3A")
        );
        assert_eq!(Route::BrowserOnly.url(code, c), None);
        assert_eq!(Route::BrowserOnly.url(ga, a).as_deref(), Some(a));
        assert!(allowed(&sources, c));
        assert!(allowed(
            &sources,
            "https://zhw-b.s3.cloud.switch.ch/aiub/CODE/x"
        ));
        assert!(!allowed(&sources, "https://example.org/x"));
        assert!(!allowed(&sources, "http://download.aiub.unibe.ch/x"));
    }
}
