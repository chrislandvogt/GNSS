//! Downloading natively, with ureq: resumable, with retries and backoff,
//! through the proxy the environment names (`HTTPS_PROXY`, `NO_PROXY` and
//! the like, or the Windows system setting), trusting the platform's
//! certificates.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gnss_store::sha256_hex;
use ureq::tls::{RootCerts, TlsConfig};

use crate::FetchError;
use crate::cache::without_query;
use crate::fetcher::Http;

/// Attempts at a download before giving up.
const ATTEMPTS: u32 = 4;

/// HTTP over the network.
pub struct NativeHttp {
    agent: ureq::Agent,
    /// Where partial downloads wait to be resumed.
    partial: PathBuf,
    /// Seconds to wait before the first retry; doubled each time.
    backoff: f64,
}

impl NativeHttp {
    /// A client keeping partial downloads in `partial` (made if missing),
    /// using the proxy the environment names, if any.
    pub fn new(partial: impl Into<PathBuf>) -> Self {
        Self::with_proxy(partial, ureq::Proxy::try_from_env())
    }

    /// The same, with a given proxy or none.
    pub fn with_proxy(partial: impl Into<PathBuf>, proxy: Option<ureq::Proxy>) -> Self {
        let config = ureq::Agent::config_builder()
            .proxy(proxy)
            .http_status_as_error(false)
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .timeout_connect(Some(Duration::from_secs(30)))
            // Per read, not for the whole file: a slow day file is fine.
            .timeout_recv_body(Some(Duration::from_secs(120)))
            .user_agent(concat!("gnss/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: config.into(),
            partial: partial.into(),
            backoff: 2.0,
        }
    }

    /// The same, retrying straight away: for tests.
    pub fn without_backoff(mut self) -> Self {
        self.backoff = 0.0;
        self
    }

    fn partial_path(&self, url: &str) -> PathBuf {
        // Signed links change between attempts; the file they name doesn't.
        let key = sha256_hex(without_query(url).as_bytes());
        self.partial.join(format!("{}.part", &key[..32]))
    }

    /// One attempt: `Ok(None)` for 404, `Err((error, retry))` otherwise.
    fn attempt(&self, url: &str, part: &Path) -> Result<Option<Vec<u8>>, (FetchError, bool)> {
        let fail = |message: String, retry: bool| {
            (
                FetchError::Http {
                    url: without_query(url).to_owned(),
                    message,
                },
                retry,
            )
        };
        let have = fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        let mut request = self.agent.get(url);
        if have > 0 {
            request = request.header("Range", format!("bytes={have}-"));
        }
        let mut response = request.call().map_err(|e| fail(e.to_string(), true))?;
        let status = response.status().as_u16();
        let resumed = match status {
            200 => false,
            206 => true,
            404 => return Ok(None),
            // The partial file is already whole.
            416 if have > 0 => {
                return read_whole(part)
                    .map(Some)
                    .map_err(|e| fail(e.to_string(), false));
            }
            408 | 429 | 500..=599 => return Err(fail(format!("HTTP {status}"), true)),
            _ => return Err(fail(format!("HTTP {status}"), false)),
        };
        let mut out = OpenOptions::new()
            .create(true)
            .write(true)
            .append(resumed)
            .truncate(!resumed)
            .open(part)
            .map_err(|e| fail(e.to_string(), false))?;
        let mut body = response.body_mut().as_reader();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = match body.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                // What's been written stays, to be resumed.
                Err(e) => return Err(fail(e.to_string(), true)),
            };
            out.write_all(buf.get(..n).unwrap_or_default())
                .map_err(|e| fail(e.to_string(), false))?;
        }
        drop(out);
        read_whole(part)
            .map(Some)
            .map_err(|e| fail(e.to_string(), false))
    }
}

fn read_whole(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

impl Http for NativeHttp {
    fn get(&mut self, url: &str) -> Result<Option<Vec<u8>>, FetchError> {
        fs::create_dir_all(&self.partial).map_err(|e| FetchError::Http {
            url: self.partial.display().to_string(),
            message: e.to_string(),
        })?;
        let part = self.partial_path(url);
        let mut wait = self.backoff;
        let mut attempt = 1;
        loop {
            match self.attempt(url, &part) {
                Ok(found) => {
                    let _ = fs::remove_file(&part);
                    return Ok(found);
                }
                Err((error, retry)) if !retry || attempt == ATTEMPTS => return Err(error),
                Err(_) => {
                    std::thread::sleep(Duration::from_secs_f64(wait));
                    wait *= 2.0;
                    attempt += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use super::*;

    /// A tiny HTTP server. For each request it calls `answer` with the
    /// request's path and `Range` header, and writes what it returns.
    fn serve(
        answer: impl Fn(&str, Option<u64>) -> Vec<u8> + Send + 'static,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
                let mut range = None;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = header.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        range = v.trim().trim_end_matches('-').parse().ok();
                    }
                }
                seen.lock().unwrap().push(format!("{path} {range:?}"));
                let _ = stream.write_all(&answer(&path, range));
            }
        });
        (base, log)
    }

    fn reply(status: &str, body: &[u8], extra: &str) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn client(dir: &Path) -> NativeHttp {
        // Not through any proxy the environment names: the server is local.
        NativeHttp::with_proxy(dir.join("partial"), None).without_backoff()
    }

    #[test]
    fn files_download_and_missing_ones_are_none() {
        let dir = tempfile::tempdir().unwrap();
        let (base, _) = serve(|path, _| match path {
            "/a" => reply("200 OK", b"hello", ""),
            _ => reply("404 Not Found", b"", ""),
        });
        let mut http = client(dir.path());
        assert_eq!(http.get(&format!("{base}/a")).unwrap().unwrap(), b"hello");
        assert_eq!(http.get(&format!("{base}/b")).unwrap(), None);
        // Nothing partial is left behind.
        assert_eq!(fs::read_dir(dir.path().join("partial")).unwrap().count(), 0);
    }

    #[test]
    fn a_cut_download_resumes_where_it_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let (base, log) = serve(|_, range| match range {
            // Claims 10 bytes, sends 4, and hangs up.
            None => {
                let mut r =
                    b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n".to_vec();
                r.extend_from_slice(b"0123");
                r
            }
            Some(from) => {
                let body = &b"0123456789"[from as usize..];
                reply(
                    "206 Partial Content",
                    body,
                    &format!("Content-Range: bytes {from}-9/10\r\n"),
                )
            }
        });
        let mut http = client(dir.path());
        assert_eq!(
            http.get(&format!("{base}/f?sig=1")).unwrap().unwrap(),
            b"0123456789"
        );
        assert_eq!(*log.lock().unwrap(), ["/f?sig=1 None", "/f?sig=1 Some(4)"]);
    }

    #[test]
    fn server_errors_are_retried_and_client_errors_arent() {
        let dir = tempfile::tempdir().unwrap();
        let count = Arc::new(Mutex::new(0));
        let n = count.clone();
        let (base, log) = serve(move |path, _| {
            if path.starts_with("/forbidden") {
                return reply("403 Forbidden", b"", "");
            }
            let mut n = n.lock().unwrap();
            *n += 1;
            if *n < 3 {
                reply("503 Service Unavailable", b"", "")
            } else {
                reply("200 OK", b"ok", "")
            }
        });
        let mut http = client(dir.path());
        assert_eq!(http.get(&format!("{base}/flaky")).unwrap().unwrap(), b"ok");
        assert_eq!(*count.lock().unwrap(), 3);
        let err = http
            .get(&format!("{base}/forbidden?sig=secret"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("403") && !err.contains("secret"), "{err}");
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .filter(|l| l.starts_with("/forbidden"))
                .count(),
            1
        );
    }
}
