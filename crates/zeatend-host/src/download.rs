//! Downloads checked against a SHA-256 digest.

use std::fs::{self, File};
use std::io::{self, BufRead as _, Write as _};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use http::header::{LOCATION, USER_AGENT};
use http::{HeaderMap, Request, Response, Uri};
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::Incoming;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use sha2::{Digest as _, Sha256};

use crate::HostError;

/// The largest file [`fetch`] downloads.
const MAX_FILE: u64 = 4 << 30;

/// How often a failed request is tried again.
const RETRIES: u32 = 3;

/// The wait before the first retry; each further one doubles it.
const BACKOFF: Duration = if cfg!(test) {
    Duration::from_millis(1)
} else {
    Duration::from_secs(1)
};

/// The longest `Retry-After` wait honoured; a longer one ends the retrying.
const MAX_WAIT: Duration = Duration::from_secs(30);

/// How long a request may wait for the answer's head, connection included.
const HEAD_TIME: Duration = Duration::from_mins(1);

/// How long reading an answer's body may take.
const BODY_TIME: Duration = Duration::from_hours(1);

/// How many redirects one request follows.
const REDIRECTS: u32 = 10;

const AGENT: &str = concat!("zeatend/", env!("CARGO_PKG_VERSION"));

type Https = Client<HttpsConnector<HttpConnector>, Full<Bytes>>;

/// An HTTP and HTTPS client for zeatend's downloads. It trusts the platform's CA store and
/// uses TLS from `aws-lc-rs`.
pub struct Agent {
    client: Result<Https, String>,
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent").finish_non_exhaustive()
    }
}

impl Agent {
    /// The client; a client that could not be made fails the download of `url`.
    fn client(&self, url: &str) -> Result<&Https, HostError> {
        self.client.as_ref().map_err(|reason| HostError::Download {
            url: url.to_owned(),
            reason: reason.clone(),
        })
    }
}

/// A client that trusts the platform's CA store. It must be made inside a Tokio runtime.
///
/// A TLS setup that fails is reported by the first download.
#[must_use]
pub fn agent() -> Agent {
    Agent {
        client: tls().map(|tls| {
            let connector = hyper_rustls::HttpsConnectorBuilder::new()
                .with_tls_config(tls)
                .https_or_http()
                .enable_http1()
                .build();
            Client::builder(TokioExecutor::new()).build(connector)
        }),
    }
}

/// The TLS configuration: the `aws-lc-rs` provider and the platform's trust roots.
fn tls() -> Result<rustls::ClientConfig, String> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier =
        rustls_platform_verifier::Verifier::new_with_extra_roots(Vec::new(), Arc::clone(&provider))
            .map_err(|e| e.to_string())?;
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth())
}

/// The innermost cause, which names what failed (DNS, TLS, a refused connection).
fn describe(error: &(dyn std::error::Error + 'static)) -> String {
    let mut source = error;
    while let Some(next) = source.source() {
        source = next;
    }
    source.to_string()
}

/// Downloads `url` to `dest` unless a file with the expected digest is already there. Returns the
/// digest. With no expected digest the file is always downloaded; check it with [`sha256_file`]
/// against a list.
///
/// # Errors
///
/// When the download fails, is larger than 4 GiB or the digest differs; a wrong file is removed.
pub async fn fetch(
    agent: &Agent,
    url: &str,
    dest: &Path,
    sha256: Option<&str>,
) -> Result<String, HostError> {
    if let Some(expected) = sha256
        && dest.exists()
        && sha256_file(dest)? == expected
    {
        return Ok(expected.to_owned());
    }
    let fail = |reason: String| HostError::Download {
        url: url.to_owned(),
        reason,
    };
    let dir = dest.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir).map_err(HostError::io(dir))?;
    let partial = dest.with_extension("partial");
    let https = agent.client(url)?;
    let actual = {
        let response = get_ok(https, url).await.map_err(fail)?;
        let mut file = File::create(&partial).map_err(HostError::io(&partial))?;
        let mut hasher = Sha256::new();
        let mut body = response.into_body();
        let mut size: u64 = 0;
        let reading = async {
            while let Some(frame) = body.frame().await {
                let frame = frame.map_err(|e| fail(describe(&e)))?;
                let Ok(chunk) = frame.into_data() else {
                    continue;
                };
                size = size.saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
                if size > MAX_FILE {
                    return Err(fail("the file is larger than 4 GiB".to_owned()));
                }
                hasher.update(&chunk);
                file.write_all(&chunk).map_err(HostError::io(&partial))?;
            }
            Ok(())
        };
        tokio::time::timeout(BODY_TIME, reading)
            .await
            .map_err(|_elapsed| fail("the download did not finish in time".to_owned()))??;
        file.sync_all().map_err(HostError::io(&partial))?;
        hex::encode(hasher.finalize())
    };
    if let Some(expected) = sha256
        && actual != expected
    {
        drop(fs::remove_file(&partial));
        return Err(HostError::Digest {
            file: file_name(dest),
            expected: expected.to_owned(),
            actual,
        });
    }
    fs::rename(&partial, dest).map_err(HostError::io(dest))?;
    Ok(actual)
}

/// Downloads a text document, such as a manifest, of at most 64 MiB, and checks it against
/// `sha256` (`sha256:<hex>`) when one is given.
///
/// # Errors
///
/// When the download fails or the digest differs.
pub async fn fetch_text(
    agent: &Agent,
    url: &str,
    sha256: Option<&str>,
) -> Result<String, HostError> {
    let fail = |reason: String| HostError::Download {
        url: url.to_owned(),
        reason,
    };
    let https = agent.client(url)?;
    let response = get_ok(https, url).await.map_err(fail)?;
    let bytes = read_all(response, 64 << 20).await.map_err(fail)?;
    let text = String::from_utf8(bytes).map_err(|e| fail(e.to_string()))?;
    if let Some(expected) = sha256 {
        check(url, expected, text.as_bytes())?;
    }
    Ok(text)
}

/// The body of `response`, at most `limit` bytes.
async fn read_all(response: Response<Incoming>, limit: usize) -> Result<Vec<u8>, String> {
    let reading = Limited::new(response.into_body(), limit).collect();
    let body = tokio::time::timeout(BODY_TIME, reading)
        .await
        .map_err(|_elapsed| "the download did not finish in time".to_owned())?
        .map_err(|e| describe(&*e))?;
    Ok(body.to_bytes().to_vec())
}

/// A GET that succeeds, or the reason it did not.
async fn get_ok(https: &Https, url: &str) -> Result<Response<Incoming>, String> {
    let response = get_retrying(https, url).await?;
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(format!("HTTP {}", response.status()))
    }
}

/// A GET that is tried again, up to three times with a doubling wait, after a failed exchange or
/// a status that RFC 9110 and RFC 6585 describe as temporary (see [`temporary`]). A
/// `Retry-After` answer sets the wait; one longer than [`MAX_WAIT`] is not waited for and the
/// response is returned. Any other status is returned as it is for the caller to judge.
async fn get_retrying(https: &Https, url: &str) -> Result<Response<Incoming>, String> {
    let uri: Uri = url.parse().map_err(|e| format!("{e}"))?;
    let mut wait = BACKOFF;
    let mut attempt = 0;
    loop {
        let asked = match get(https, uri.clone()).await {
            Ok(response) => {
                if !temporary(response.status().as_u16()) || attempt == RETRIES {
                    return Ok(response);
                }
                let asked = retry_after(response.headers(), SystemTime::now());
                if asked.is_some_and(|a| a > MAX_WAIT) {
                    return Ok(response);
                }
                asked
            }
            Err(e) => {
                if attempt == RETRIES {
                    return Err(e);
                }
                None
            }
        };
        attempt += 1;
        tokio::time::sleep(asked.unwrap_or(wait)).await;
        wait *= 2;
    }
}

/// One GET that follows up to [`REDIRECTS`] redirects. A redirect from `https` to `http` is
/// refused.
async fn get(https: &Https, mut uri: Uri) -> Result<Response<Incoming>, String> {
    for _ in 0..=REDIRECTS {
        let request = Request::get(uri.clone())
            .header(USER_AGENT, AGENT)
            .body(Full::<Bytes>::default())
            .map_err(|e| e.to_string())?;
        let response = tokio::time::timeout(HEAD_TIME, https.request(request))
            .await
            .map_err(|_elapsed| "the server did not answer in time".to_owned())?
            .map_err(|e| describe(&e))?;
        if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        let Some(location) = response
            .headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok())
        else {
            return Ok(response);
        };
        let next = resolve(&uri, location)?;
        if uri.scheme_str() == Some("https") && next.scheme_str() != Some("https") {
            return Err(format!("refused the redirect from https to {next}"));
        }
        uri = next;
    }
    Err(format!("more than {REDIRECTS} redirects"))
}

/// The target of a redirect from `base` to `location`: the reference resolution of RFC 3986
/// section 5.2, as RFC 9110 section 10.2.2 asks.
fn resolve(base: &Uri, location: &str) -> Result<Uri, String> {
    let scheme = base.scheme_str().unwrap_or("https");
    let authority = base.authority().map_or("", |a| a.as_str());
    let location = location.split('#').next().unwrap_or_default();
    let lower = location.to_ascii_lowercase();
    let target = if lower.starts_with("http://") || lower.starts_with("https://") {
        location.to_owned()
    } else if let Some(rest) = location.strip_prefix("//") {
        format!("{scheme}://{rest}")
    } else if location.is_empty() {
        base.to_string()
    } else if location.starts_with('?') {
        format!("{scheme}://{authority}{}{location}", base.path())
    } else {
        let (path, query) = location
            .split_once('?')
            .map_or((location, ""), |(p, q)| (p, q));
        let joined = if path.starts_with('/') {
            path.to_owned()
        } else {
            let own = base.path();
            let dir = own.rfind('/').and_then(|i| own.get(..=i)).unwrap_or("/");
            format!("{dir}{path}")
        };
        let query = if location.contains('?') {
            format!("?{query}")
        } else {
            String::new()
        };
        format!("{scheme}://{authority}{}{query}", without_dots(&joined))
    };
    target
        .parse()
        .map_err(|e| format!("the redirect to {location}: {e}"))
}

/// `path` without its `.` and `..` segments (RFC 3986 section 5.2.4).
fn without_dots(path: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut segments = path.split('/').skip(1).peekable();
    while let Some(segment) = segments.next() {
        let last = segments.peek().is_none();
        match segment {
            "." => {}
            ".." => {
                kept.pop();
            }
            _ => {
                kept.push(segment);
                continue;
            }
        }
        if last {
            kept.push("");
        }
    }
    format!("/{}", kept.join("/"))
}

/// Whether the status says the same request may work later: 408 (RFC 9110 section 15.5.9),
/// 429 (RFC 6585 section 4), and the gateway and availability errors 502, 503 and 504
/// (RFC 9110 sections 15.6.3 to 15.6.5). 500, 501 and 505 describe a fault that waiting does
/// not fix.
const fn temporary(status: u16) -> bool {
    matches!(status, 408 | 429 | 502 | 503 | 504)
}

/// The wait a `Retry-After` header asks for, as `delay-seconds` or an `HTTP-date`
/// (RFC 9110 section 10.2.3). A date in the past means no wait.
fn retry_after(headers: &HeaderMap, now: SystemTime) -> Option<Duration> {
    let value = headers.get("retry-after")?.to_str().ok()?.trim();
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        return value.parse().ok().map(Duration::from_secs);
    }
    let date = httpdate::parse_http_date(value).ok()?;
    Some(date.duration_since(now).unwrap_or_default())
}

/// Checks `bytes` against `expected`, `sha256:<hex>` or bare hex.
///
/// # Errors
///
/// When the digest differs.
pub fn check(what: &str, expected: &str, bytes: &[u8]) -> Result<(), HostError> {
    let expected = expected.strip_prefix("sha256:").unwrap_or(expected);
    let actual = hex::encode(Sha256::digest(bytes));
    if actual == expected {
        return Ok(());
    }
    Err(HostError::Digest {
        file: what.to_owned(),
        expected: expected.to_owned(),
        actual,
    })
}

/// The SHA-256 of a file.
///
/// # Errors
///
/// When it cannot be read.
pub fn sha256_file(path: &Path) -> Result<String, HostError> {
    let mut file = io::BufReader::new(File::open(path).map_err(HostError::io(path))?);
    let mut hasher = Sha256::new();
    loop {
        let chunk = file.fill_buf().map_err(HostError::io(path))?;
        if chunk.is_empty() {
            break;
        }
        hasher.update(chunk);
        let n = chunk.len();
        file.consume(n);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// The digest of `file` in a `sha256sum` list.
#[must_use]
pub fn digest_in_list<'a>(list: &'a str, file: &str) -> Option<&'a str> {
    list.lines().find_map(|line| {
        let (digest, name) = line.split_once(char::is_whitespace)?;
        (name.trim_start().trim_start_matches('*') == file).then_some(digest)
    })
}

/// Checks a downloaded file against a `sha256sum` list, removing it when it differs.
///
/// # Errors
///
/// When the list does not name it or the digest differs.
pub fn check_against_list(list: &str, path: &Path, actual: &str) -> Result<(), HostError> {
    let file = file_name(path);
    let expected = digest_in_list(list, &file).ok_or_else(|| HostError::Parse {
        what: "the release's checksum list".to_owned(),
        reason: format!("no {file}"),
    })?;
    if expected == actual {
        return Ok(());
    }
    drop(fs::remove_file(path));
    Err(HostError::Digest {
        file,
        expected: expected.to_owned(),
        actual: actual.to_owned(),
    })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;

    use super::*;

    /// Serves one status per request, the last one for all later requests. Returns the URL and
    /// the number of requests seen.
    fn serve(statuses: &'static [u16], body: &'static str) -> (String, Arc<AtomicUsize>) {
        serve_with(statuses, "", body)
    }

    /// Like [`serve`], with `extra` header lines (each ending in CRLF) on every non-200 reply.
    fn serve_with(
        statuses: &'static [u16],
        extra: &'static str,
        body: &'static str,
    ) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let seen = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&seen);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut request = [0_u8; 4096];
                drop(stream.read(&mut request));
                let n = count.fetch_add(1, Ordering::SeqCst);
                let status = statuses
                    .get(n)
                    .or_else(|| statuses.last())
                    .copied()
                    .unwrap_or(500);
                let body = if status == 200 { body } else { "" };
                let extra = if status == 200 { "" } else { extra };
                let reply = format!(
                    "HTTP/1.1 {status} X\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                drop(stream.write_all(reply.as_bytes()));
            }
        });
        (url, seen)
    }

    const ABC: &str = "abc";
    const ABC_SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[tokio::test]
    async fn retries_server_errors_until_success() {
        let (url, seen) = serve(&[503, 503, 200], ABC);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f");
        let digest = fetch(&agent(), &url, &dest, Some(ABC_SHA)).await.unwrap();
        assert_eq!(digest, ABC_SHA);
        assert_eq!(fs::read_to_string(&dest).unwrap(), ABC);
        assert_eq!(seen.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn text_is_retried_too() {
        let (url, _) = serve(&[429, 504, 200], ABC);
        assert_eq!(fetch_text(&agent(), &url, None).await.unwrap(), ABC);
    }

    #[tokio::test]
    async fn client_errors_fail_at_once() {
        let (url, seen) = serve(&[404], "");
        let dir = tempfile::tempdir().unwrap();
        let err = fetch(&agent(), &url, &dir.path().join("f"), None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("404"), "{err}");
        assert_eq!(seen.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn gives_up_after_the_limit() {
        let (url, seen) = serve(&[503], "");
        let err = fetch_text(&agent(), &url, None).await.unwrap_err();
        assert!(err.to_string().contains("503"), "{err}");
        assert_eq!(seen.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn only_temporary_statuses_are_retried() {
        let (url, seen) = serve(&[408, 502, 200], ABC);
        assert_eq!(fetch_text(&agent(), &url, None).await.unwrap(), ABC);
        assert_eq!(seen.load(Ordering::SeqCst), 3);
        let fixed: [&'static [u16]; 3] = [&[500, 200], &[501, 200], &[505, 200]];
        for statuses in fixed {
            let (url, seen) = serve(statuses, ABC);
            assert!(
                fetch_text(&agent(), &url, None).await.is_err(),
                "{statuses:?}"
            );
            assert_eq!(seen.load(Ordering::SeqCst), 1, "{statuses:?}");
        }
    }

    #[tokio::test]
    async fn retry_after_is_honoured_as_seconds_or_a_date() {
        let (url, seen) = serve_with(&[503, 200], "Retry-After: 0\r\n", ABC);
        assert_eq!(fetch_text(&agent(), &url, None).await.unwrap(), ABC);
        assert_eq!(seen.load(Ordering::SeqCst), 2);
        let (url, seen) = serve_with(
            &[429, 200],
            "Retry-After: Fri, 31 Dec 1999 23:59:59 GMT\r\n",
            ABC,
        );
        assert_eq!(fetch_text(&agent(), &url, None).await.unwrap(), ABC);
        assert_eq!(seen.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_retry_after_beyond_the_cap_is_not_waited_for() {
        let (url, seen) = serve_with(&[503, 200], "Retry-After: 3600\r\n", ABC);
        let err = fetch_text(&agent(), &url, None).await.unwrap_err();
        assert!(err.to_string().contains("503"), "{err}");
        assert_eq!(seen.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn digest_mismatch_is_not_retried() {
        let (url, seen) = serve(&[200], ABC);
        let dir = tempfile::tempdir().unwrap();
        let wrong = "0".repeat(64);
        let err = fetch(&agent(), &url, &dir.path().join("f"), Some(&wrong))
            .await
            .unwrap_err();
        assert!(matches!(err, HostError::Digest { .. }), "{err}");
        assert_eq!(seen.load(Ordering::SeqCst), 1);
    }

    /// Answers `/file` with a redirect to `/moved/file`, and that with `abc`; returns the URL and
    /// the request heads seen.
    fn serve_redirect(location: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/dir/file", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let heads = Arc::clone(&seen);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut request = [0_u8; 4096];
                let n = stream.read(&mut request).unwrap_or(0);
                let head = String::from_utf8_lossy(&request[..n]).to_ascii_lowercase();
                let reply = if head.starts_with("get /dir/file ") {
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                } else {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{ABC}",
                        ABC.len()
                    )
                };
                heads.lock().unwrap().push(head);
                drop(stream.write_all(reply.as_bytes()));
            }
        });
        (url, seen)
    }

    #[tokio::test]
    async fn redirects_are_followed() {
        for location in ["/moved/file", "moved/file"] {
            let (url, seen) = serve_redirect(location);
            assert_eq!(fetch_text(&agent(), &url, None).await.unwrap(), ABC);
            let heads = seen.lock().unwrap().clone();
            let target = if location.starts_with('/') {
                "get /moved/file "
            } else {
                "get /dir/moved/file "
            };
            assert!(heads[1].starts_with(target), "{heads:?}");
            assert!(heads[1].contains("user-agent: zeatend/"), "{heads:?}");
        }
    }

    #[test]
    fn redirect_targets_resolve_against_the_request() {
        let base: Uri = "https://example.com/a/b".parse().unwrap();
        for (location, target) in [
            ("c", "https://example.com/a/c"),
            ("/c", "https://example.com/c"),
            ("//cdn.example.com/x", "https://cdn.example.com/x"),
            ("HTTPS://cdn.example.com/y", "https://cdn.example.com/y"),
            ("?token=abc", "https://example.com/a/b?token=abc"),
            ("c?x=1#frag", "https://example.com/a/c?x=1"),
            ("../c", "https://example.com/c"),
            ("../../../c", "https://example.com/c"),
            ("./c/./d/..", "https://example.com/a/c/"),
            ("/p/../q?r=1", "https://example.com/q?r=1"),
        ] {
            assert_eq!(resolve(&base, location).unwrap().to_string(), target);
        }
    }

    #[test]
    fn checksum_lists() {
        let list = "aa  rke2.linux-arm64.tar.gz\nbb *rke2-images-core.linux-arm64.tar.zst\n";
        assert_eq!(digest_in_list(list, "rke2.linux-arm64.tar.gz"), Some("aa"));
        assert_eq!(
            digest_in_list(list, "rke2-images-core.linux-arm64.tar.zst"),
            Some("bb")
        );
        assert_eq!(digest_in_list(list, "rke2.linux-amd64.tar.gz"), None);
    }

    #[test]
    fn file_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
