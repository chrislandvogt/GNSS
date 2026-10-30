//! The relay: forwards a browser's GET requests to the data sources that
//! don't send cross-origin headers (ARCHITECTURE.md section 13).
//!
//! - **Only declared hosts.** A request names the file it wants
//!   (`/?url=<percent-encoded URL>`, `gnss_fetch::relay`). It's forwarded
//!   only if the URL is HTTPS on a host the jurisdiction packs declare, and
//!   redirects are followed only to such hosts.
//! - **GET only**, with `Range` passed on so downloads resume, and
//!   cross-origin headers added to the answer.
//! - **Stateless, with no logs.** Nothing about a request is written
//!   anywhere or kept after it's answered. It sees which files are asked
//!   for, which hints at where and when a survey happened, so the web app
//!   says so before it's first used, and it's off by default.
//!
//! The HTTP it speaks is a deliberately small HTTP/1.1: one request per
//! connection, a request head of at most 8 KiB, and time limits on reading
//! it, so a slow or malformed client can't hold the relay up.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use gnss_fetch::Source;
use gnss_fetch::relay::{allowed, requested};

/// How the relay runs.
#[derive(Debug, Clone)]
pub struct Config {
    /// The packs' sources: their hosts are the only ones forwarded to.
    pub sources: Vec<Source>,
    /// The `Access-Control-Allow-Origin` sent back: `*`, or the web app's
    /// origin to serve only it.
    pub allow_origin: String,
    /// Requests served at once; more are refused with 503.
    pub max_connections: usize,
}

/// What an upstream server answered.
pub struct UpstreamResponse {
    /// The HTTP status.
    pub status: u16,
    /// The headers worth passing on, lower-case names.
    pub headers: Vec<(String, String)>,
    /// The body, read as it's sent on.
    pub body: Box<dyn Read + Send>,
}

/// Fetching from the sources: the network, or a fake in tests.
pub trait Upstream: Send + Sync {
    /// One GET, not following redirects, with `Range` if given.
    fn get(&self, url: &str, range: Option<&str>) -> Result<UpstreamResponse, String>;
}

/// Headers passed on from upstream.
const PASSED: [&str; 7] = [
    "content-type",
    "content-length",
    "content-range",
    "accept-ranges",
    "etag",
    "last-modified",
    "location",
];

/// Redirects followed before giving up.
const MAX_REDIRECTS: usize = 5;

/// Serves forever on a listener, a thread per connection.
pub fn serve(listener: TcpListener, config: Arc<Config>, upstream: Arc<dyn Upstream>) {
    let busy = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let (config, upstream, busy) = (config.clone(), upstream.clone(), busy.clone());
        std::thread::spawn(move || {
            let n = busy.fetch_add(1, Ordering::SeqCst);
            let _ = if n >= config.max_connections {
                reply(
                    &stream,
                    &config,
                    503,
                    "the relay is busy; try again shortly",
                )
            } else {
                handle(&stream, &config, upstream.as_ref())
            };
            busy.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// A request's method, target and `Range` header.
struct Request {
    method: String,
    target: String,
    range: Option<String>,
}

fn read_request(stream: &TcpStream) -> io::Result<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(20)))?;
    let mut reader = BufReader::new(stream.take(8 * 1024));
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no request line",
        ));
    };
    let (method, target) = (method.to_owned(), target.to_owned());
    let mut range = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the request head is cut short or too long",
            ));
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("range")
        {
            let value = value.trim();
            // Only a plain byte range, as a resumed download sends.
            if value.starts_with("bytes=") && value.len() < 64 {
                range = Some(value.to_owned());
            }
        }
    }
    Ok(Request {
        method,
        target,
        range,
    })
}

fn cors(config: &Config) -> String {
    format!(
        "Access-Control-Allow-Origin: {}\r\nAccess-Control-Expose-Headers: Content-Length, Content-Range, ETag\r\nVary: Origin\r\n",
        config.allow_origin
    )
}

/// A short plain-text answer.
fn reply(mut stream: &TcpStream, config: &Config, status: u16, text: &str) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {}\r\n{}Content-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{text}",
        reason(status),
        cors(config),
        text.len()
    )?;
    stream.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        206 => "Partial Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        416 => "Range Not Satisfiable",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "",
    }
}

fn handle(stream: &TcpStream, config: &Config, upstream: &dyn Upstream) -> io::Result<()> {
    let request = match read_request(stream) {
        Ok(r) => r,
        Err(_) => return reply(stream, config, 400, "that isn't a request the relay reads"),
    };
    match request.method.as_str() {
        "OPTIONS" => {
            let mut out = stream;
            write!(
                out,
                "HTTP/1.1 204 No Content\r\n{}Access-Control-Allow-Methods: GET\r\nAccess-Control-Allow-Headers: Range\r\nAccess-Control-Max-Age: 86400\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                cors(config)
            )?;
            return out.flush();
        }
        "GET" => {}
        _ => return reply(stream, config, 405, "the relay only answers GET"),
    }
    let Some(mut url) = requested(&request.target) else {
        return reply(
            stream,
            config,
            if request.target == "/" { 200 } else { 400 },
            "gnss relay: ask for /?url=<a percent-encoded https URL on a host the packs declare>",
        );
    };
    if !allowed(&config.sources, &url) {
        return reply(
            stream,
            config,
            403,
            "that isn't an https URL on a host the packs declare",
        );
    }
    for _ in 0..=MAX_REDIRECTS {
        let response = match upstream.get(&url, request.range.as_deref()) {
            Ok(r) => r,
            Err(_) => return reply(stream, config, 502, "the source didn't answer"),
        };
        if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            let location = response
                .headers
                .iter()
                .find(|(n, _)| n == "location")
                .map(|(_, v)| resolve(&url, v));
            match location {
                Some(next) if allowed(&config.sources, &next) => {
                    url = next;
                    continue;
                }
                _ => {
                    return reply(
                        stream,
                        config,
                        502,
                        "the source redirected outside the hosts the packs declare",
                    );
                }
            }
        }
        return forward(stream, config, response);
    }
    reply(stream, config, 502, "too many redirects")
}

/// A redirect's target, absolute.
fn resolve(base: &str, location: &str) -> String {
    if location.starts_with('/') {
        let origin_end = base
            .strip_prefix("https://")
            .and_then(|rest| rest.find('/'))
            .map_or(base.len(), |i| i + "https://".len());
        format!("{}{location}", base.get(..origin_end).unwrap_or(base))
    } else {
        location.to_owned()
    }
}

fn forward(
    mut stream: &TcpStream,
    config: &Config,
    mut response: UpstreamResponse,
) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\n{}Cache-Control: no-store\r\nConnection: close\r\n",
        response.status,
        reason(response.status),
        cors(config)
    );
    for (name, value) in &response.headers {
        // Values from upstream are passed on only if they're one line.
        if PASSED.contains(&name.as_str()) && name != "location" && !value.contains(['\r', '\n']) {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    io::copy(&mut response.body, &mut stream)?;
    stream.flush()
}

/// The network, through ureq: not following redirects (the relay checks
/// each), trusting the platform's certificates, through the environment's
/// proxy.
pub struct Network {
    agent: ureq::Agent,
}

impl Network {
    /// A client with the environment's proxy, if any.
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(Duration::from_secs(120)))
            .user_agent(concat!("gnss-relay/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Default for Network {
    fn default() -> Self {
        Self::new()
    }
}

impl Upstream for Network {
    fn get(&self, url: &str, range: Option<&str>) -> Result<UpstreamResponse, String> {
        let mut request = self.agent.get(url);
        if let Some(range) = range {
            request = request.header("Range", range);
        }
        let response = request.call().map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(n, v)| {
                Some((n.as_str().to_ascii_lowercase(), v.to_str().ok()?.to_owned()))
            })
            .collect();
        let body = Box::new(response.into_body().into_reader());
        Ok(UpstreamResponse {
            status,
            headers,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::net::Shutdown;
    use std::sync::Mutex;

    use super::*;

    /// A fake answer: status, headers and body.
    type Page = (u16, Vec<(String, String)>, Vec<u8>);

    /// Fake sources: URL to answer; records each request.
    #[derive(Default)]
    struct Fake {
        pages: BTreeMap<String, Page>,
        asked: Mutex<Vec<String>>,
    }

    impl Upstream for Fake {
        fn get(&self, url: &str, range: Option<&str>) -> Result<UpstreamResponse, String> {
            self.asked.lock().unwrap().push(format!("{url} {range:?}"));
            let (status, headers, body) =
                self.pages
                    .get(url)
                    .cloned()
                    .unwrap_or((404, Vec::new(), Vec::new()));
            let (status, body) = match range.and_then(|r| {
                r.strip_prefix("bytes=")?
                    .trim_end_matches('-')
                    .parse::<usize>()
                    .ok()
            }) {
                Some(from) if status == 200 => (206, body.get(from..).unwrap_or_default().to_vec()),
                _ => (status, body),
            };
            let mut headers = headers;
            headers.push(("content-length".into(), body.len().to_string()));
            Ok(UpstreamResponse {
                status,
                headers,
                body: Box::new(io::Cursor::new(body)),
            })
        }
    }

    fn start(fake: Fake) -> (String, Arc<Fake>) {
        let sources =
            gnss_fetch::read_sources(include_str!("../../../packs/au/pack.toml")).unwrap();
        let config = Arc::new(Config {
            sources,
            allow_origin: "*".into(),
            max_connections: 8,
        });
        let fake = Arc::new(fake);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let upstream: Arc<dyn Upstream> = fake.clone();
        std::thread::spawn(move || serve(listener, config, upstream));
        (addr, fake)
    }

    /// Sends a raw request and returns the whole answer.
    fn ask(addr: &str, request: &str) -> String {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut out = String::new();
        stream.read_to_string(&mut out).unwrap();
        out
    }

    fn get(addr: &str, url: &str, extra: &str) -> String {
        let target = gnss_fetch::relay::relay_url("", url);
        ask(
            addr,
            &format!("GET {target} HTTP/1.1\r\nHost: relay\r\n{extra}\r\n"),
        )
    }

    const CODE: &str = "https://download.aiub.unibe.ch/CODE/2025/x.SP3.gz";
    const S3: &str = "https://zhw-b.s3.cloud.switch.ch/aiub/CODE/2025/x.SP3.gz";

    fn code_with_redirect() -> Fake {
        let mut fake = Fake::default();
        fake.pages.insert(
            CODE.into(),
            (301, vec![("location".into(), S3.into())], Vec::new()),
        );
        fake.pages.insert(
            S3.into(),
            (
                200,
                vec![("content-type".into(), "application/gzip".into())],
                b"0123456789".to_vec(),
            ),
        );
        fake
    }

    #[test]
    fn declared_files_are_relayed_with_cross_origin_headers() {
        let (addr, fake) = start(code_with_redirect());
        let answer = get(&addr, CODE, "");
        assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
        assert!(answer.contains("Access-Control-Allow-Origin: *\r\n"));
        assert!(answer.contains("content-type: application/gzip\r\n"));
        assert!(answer.ends_with("\r\n\r\n0123456789"));
        // The redirect was followed by the relay, not passed on.
        assert!(!answer.contains("location:"));
        assert_eq!(fake.asked.lock().unwrap().len(), 2);
        // A resumed download asks for the rest.
        let answer = get(&addr, CODE, "Range: bytes=4-\r\n");
        assert!(
            answer.starts_with("HTTP/1.1 206 Partial Content\r\n"),
            "{answer}"
        );
        assert!(answer.ends_with("456789"));
    }

    #[test]
    fn nothing_undeclared_is_relayed() {
        let mut fake = code_with_redirect();
        fake.pages.insert(
            CODE.into(),
            (
                302,
                vec![("location".into(), "https://example.org/x".into())],
                Vec::new(),
            ),
        );
        let (addr, fake) = start(fake);
        let answer = get(&addr, "https://example.org/secret", "");
        assert!(answer.starts_with("HTTP/1.1 403"), "{answer}");
        let answer = get(&addr, "http://download.aiub.unibe.ch/x", "");
        assert!(answer.starts_with("HTTP/1.1 403"), "{answer}");
        // A redirect off the declared hosts isn't followed.
        let answer = get(&addr, CODE, "");
        assert!(answer.starts_with("HTTP/1.1 502"), "{answer}");
        // Only the one declared request reached upstream.
        assert_eq!(*fake.asked.lock().unwrap(), [format!("{CODE} None")]);
        let answer = ask(&addr, "POST /?url=x HTTP/1.1\r\n\r\n");
        assert!(answer.starts_with("HTTP/1.1 405"), "{answer}");
        let answer = ask(&addr, "OPTIONS /?url=x HTTP/1.1\r\n\r\n");
        assert!(
            answer.starts_with("HTTP/1.1 204")
                && answer.contains("Access-Control-Allow-Headers: Range"),
            "{answer}"
        );
        let answer = ask(&addr, "nonsense");
        assert!(answer.starts_with("HTTP/1.1 400"), "{answer}");
        // A head over 8 KiB: answered 400 and closed, which the client may
        // see as a reset while it's still sending. Either way, refused.
        let mut stream = TcpStream::connect(&addr).unwrap();
        let big = format!("GET /?url=x HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(9000));
        let _ = stream.write_all(big.as_bytes());
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        assert!(out.is_empty() || out.starts_with("HTTP/1.1 400"), "{out}");
        assert_eq!(fake.asked.lock().unwrap().len(), 1);
    }

    #[test]
    fn redirects_resolve_against_the_source() {
        assert_eq!(
            resolve("https://a.example/x/y?z", "/b/c"),
            "https://a.example/b/c"
        );
        assert_eq!(
            resolve("https://a.example/x", "https://b.example/c"),
            "https://b.example/c"
        );
    }
}
