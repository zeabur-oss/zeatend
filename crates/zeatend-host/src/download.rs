//! Downloads checked against a SHA-256 digest.

use std::fs::{self, File};
use std::io::{self, BufRead as _, Seek as _, Write as _};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use reqwest::header::{
    CACHE_CONTROL, CONTENT_RANGE, ETAG, HeaderMap, HeaderValue, IF_RANGE, LAST_MODIFIED, PRAGMA,
    RANGE,
};
use reqwest::redirect::{self, Attempt};
use reqwest::{Client, StatusCode};
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

/// How long a request may wait for the answer's head, connection and redirects included.
const HEAD_TIME: Duration = Duration::from_mins(1);

/// How long the body of an answer may pause before the attempt fails.
const IDLE: Duration = if cfg!(test) {
    Duration::from_millis(500)
} else {
    Duration::from_mins(1)
};

/// How often [`Progress::update`] is called while a body arrives.
const REPORT_EVERY: Duration = Duration::from_secs(1);

/// How many redirects one request follows.
const REDIRECTS: usize = 10;

const AGENT: &str = concat!("zeatend/", env!("CARGO_PKG_VERSION"));

/// An HTTP and HTTPS client for zeatend's downloads. It trusts the platform's CA store, uses TLS
/// from `aws-lc-rs`, and goes through the proxy that `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY`
/// name.
pub struct Agent {
    client: Result<Client, String>,
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent").finish_non_exhaustive()
    }
}

impl Agent {
    /// The client; a client that could not be made fails the download of `url`.
    fn client(&self, url: &str) -> Result<&Client, HostError> {
        self.client.as_ref().map_err(|reason| HostError::Download {
            url: url.to_owned(),
            reason: reason.clone(),
        })
    }
}

/// A client that trusts the platform's CA store.
///
/// A TLS setup that fails is reported by the first download.
#[must_use]
pub fn agent() -> Agent {
    Agent {
        client: Client::builder()
            .user_agent(AGENT)
            .redirect(redirect::Policy::custom(follow))
            .build()
            .map_err(|e| describe(&e)),
    }
}

/// Follows up to [`REDIRECTS`] redirects. A redirect from `https` to `http` is refused.
fn follow(attempt: Attempt<'_>) -> redirect::Action {
    let from_https = attempt
        .previous()
        .last()
        .is_some_and(|url| url.scheme() == "https");
    if from_https && attempt.url().scheme() != "https" {
        let reason = format!("refused the redirect from https to {}", attempt.url());
        attempt.error(reason)
    } else if attempt.previous().len() > REDIRECTS {
        attempt.error(format!("more than {REDIRECTS} redirects"))
    } else {
        attempt.follow()
    }
}

/// The innermost cause, which names what failed (DNS, TLS, a refused connection).
fn describe(error: &(dyn std::error::Error + 'static)) -> String {
    let mut source = error;
    while let Some(next) = source.source() {
        source = next;
    }
    source.to_string()
}

/// Receives the progress of a download.
pub trait Progress: Send {
    /// Called about once a second while the body arrives, and once when it is complete. `done`
    /// counts the bytes of the file so far, `total` is its size when the server said it, and
    /// `rate` is the average speed of this download in bytes per second.
    fn update(&mut self, done: u64, total: Option<u64>, rate: u64);

    /// Called once when the download ends, whether it worked or not.
    fn finish(&mut self) {}
}

/// Shows no progress.
impl Progress for () {
    fn update(&mut self, _done: u64, _total: Option<u64>, _rate: u64) {}
}

/// Downloads `url` to `dest` unless a file with the expected digest is already there. Returns the
/// digest. With no expected digest the file is always downloaded; check the returned digest
/// against a list.
///
/// A failed attempt is tried again up to three times; an attempt that adds to the file does not
/// count. A retry asks for the rest of the file
/// (`Range`, RFC 9110 section 14.2); a server that sends the whole file again starts it over.
/// The body may pause for at most a minute between chunks.
///
/// # Errors
///
/// When the download fails, is larger than 4 GiB or the digest differs; a wrong file is removed.
pub async fn fetch(
    agent: &Agent,
    url: &str,
    dest: &Path,
    sha256: Option<&str>,
    progress: &mut dyn Progress,
) -> Result<String, HostError> {
    if let Some(expected) = sha256
        && is_cached(dest, expected)
    {
        return Ok(expected.to_owned());
    }
    let dir = dest.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir).map_err(HostError::io(dir))?;
    let partial = dest.with_extension("partial");
    let https = agent.client(url)?;
    let mut part = Part::create(&partial)?;
    let mut meter = Meter::new(progress);
    let result = download(https, url, &mut part, &mut meter).await;
    meter.progress.finish();
    let actual = match result.and_then(|()| part.finish()) {
        Ok(actual) => actual,
        Err(e) => {
            drop(fs::remove_file(&partial));
            return Err(e);
        }
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

/// The part of a file downloaded so far, and its digest.
struct Part<'a> {
    path: &'a Path,
    file: File,
    hasher: Sha256,
    size: u64,
    /// The `ETag` or `Last-Modified` of the answer that began the file, sent as `If-Range` so
    /// that a server whose file changed sends all of it (RFC 9110 section 13.1.5).
    validator: Option<HeaderValue>,
    /// The complete length the answer that began the file gave. A later part must belong to a
    /// file of that length.
    length: Option<u64>,
}

impl<'a> Part<'a> {
    fn create(path: &'a Path) -> Result<Self, HostError> {
        Ok(Self {
            path,
            file: File::create(path).map_err(HostError::io(path))?,
            hasher: Sha256::new(),
            size: 0,
            validator: None,
            length: None,
        })
    }

    /// Empties the file to download it from the start.
    fn restart(&mut self) -> Result<(), HostError> {
        self.file.set_len(0).map_err(HostError::io(self.path))?;
        self.file.rewind().map_err(HostError::io(self.path))?;
        self.hasher = Sha256::new();
        self.size = 0;
        self.validator = None;
        self.length = None;
        Ok(())
    }

    fn append(&mut self, chunk: &[u8]) -> Result<(), Failure> {
        self.size = self
            .size
            .saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
        if self.size > MAX_FILE {
            return Err(Failure::Fatal("the file is larger than 4 GiB".to_owned()));
        }
        self.hasher.update(chunk);
        self.file
            .write_all(chunk)
            .map_err(HostError::io(self.path))?;
        Ok(())
    }

    /// Writes the file to disk and returns its digest.
    fn finish(self) -> Result<String, HostError> {
        self.file.sync_all().map_err(HostError::io(self.path))?;
        Ok(hex::encode(self.hasher.finalize()))
    }
}

/// Reports progress at most once per [`REPORT_EVERY`].
struct Meter<'a> {
    progress: &'a mut dyn Progress,
    started: Instant,
    reported: Instant,
    received: u64,
}

impl<'a> Meter<'a> {
    fn new(progress: &'a mut dyn Progress) -> Self {
        let now = Instant::now();
        Self {
            progress,
            started: now,
            reported: now,
            received: 0,
        }
    }

    fn add(&mut self, bytes: usize, done: u64, total: Option<u64>) {
        self.received = self
            .received
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
        if self.reported.elapsed() >= REPORT_EVERY {
            self.reported = Instant::now();
            self.report(done, total);
        }
    }

    fn report(&mut self, done: u64, total: Option<u64>) {
        let millis = self.started.elapsed().as_millis().max(1);
        let rate = u128::from(self.received).saturating_mul(1000) / millis;
        self.progress
            .update(done, total, u64::try_from(rate).unwrap_or(u64::MAX));
    }
}

/// Why an attempt failed.
enum Failure {
    /// Another attempt may work, after the wait the server asked for, if any.
    Retry(String, Option<Duration>),
    /// Another attempt will not help.
    Fatal(String),
    /// A local error.
    Host(HostError),
}

impl From<HostError> for Failure {
    fn from(error: HostError) -> Self {
        Self::Host(error)
    }
}

fn retry(reason: impl Into<String>) -> Failure {
    Failure::Retry(reason.into(), None)
}

/// Tries [`attempt`] up to four times in a row without progress, with a doubling wait or the wait
/// a `Retry-After` header asks for. A `Retry-After` longer than [`MAX_WAIT`] ends the retrying.
async fn download(
    https: &Client,
    url: &str,
    part: &mut Part<'_>,
    meter: &mut Meter<'_>,
) -> Result<(), HostError> {
    let fail = |reason: String| HostError::Download {
        url: url.to_owned(),
        reason,
    };
    let mut wait = BACKOFF;
    let mut tries = 0;
    // The most bytes the file has held. An attempt that passes it made progress and does not
    // count as a try; the file's size bounds how often that can happen.
    let mut most = 0;
    loop {
        let (reason, asked) = match attempt(https, url, part, meter, tries > 0).await {
            Ok(()) => return Ok(()),
            Err(Failure::Host(e)) => return Err(e),
            Err(Failure::Fatal(reason)) => return Err(fail(reason)),
            Err(Failure::Retry(reason, asked)) => (reason, asked),
        };
        if part.size > most {
            most = part.size;
            tries = 0;
            wait = BACKOFF;
        }
        if tries == RETRIES || asked.is_some_and(|a| a > MAX_WAIT) {
            return Err(fail(reason));
        }
        tries += 1;
        tokio::time::sleep(asked.unwrap_or(wait)).await;
        wait *= 2;
    }
}

/// One GET, with its redirects, that adds its body to `part`. With bytes in `part` it asks for
/// the rest. A retry asks caches on the way, such as a proxy, not to answer from a stored copy,
/// which may be the incomplete answer of the attempt before (RFC 9111 section 5.2.1.4).
async fn attempt(
    https: &Client,
    url: &str,
    part: &mut Part<'_>,
    meter: &mut Meter<'_>,
    retrying: bool,
) -> Result<(), Failure> {
    let mut request = https.get(url);
    if retrying {
        request = request
            .header(CACHE_CONTROL, "no-cache")
            .header(PRAGMA, "no-cache");
    }
    if part.size > 0 {
        request = request.header(RANGE, format!("bytes={}-", part.size));
        if let Some(validator) = &part.validator {
            request = request.header(IF_RANGE, validator.clone());
        }
    }
    let mut response = tokio::time::timeout(HEAD_TIME, request.send())
        .await
        .map_err(|_elapsed| retry("the server did not answer in time"))?
        .map_err(|e| retry(describe(&e)))?;
    let status = response.status();
    let total = if status == StatusCode::PARTIAL_CONTENT && part.size > 0 {
        match content_range(response.headers()) {
            Some((start, total)) if start == part.size => {
                if let (Some(before), Some(now)) = (part.length, total)
                    && before != now
                {
                    part.restart()?;
                    return Err(retry(format!(
                        "the file was {before} bytes and is now {now} bytes"
                    )));
                }
                total.or(part.length)
            }
            _ => {
                part.restart()?;
                return Err(retry("the server sent another part of the file"));
            }
        }
    } else if status.is_success() && status != StatusCode::PARTIAL_CONTENT {
        part.restart()?;
        part.validator = validator(response.headers());
        part.length = response.content_length();
        part.length
    } else if status == StatusCode::RANGE_NOT_SATISFIABLE && part.size > 0 {
        part.restart()?;
        return Err(retry(format!("HTTP {status}")));
    } else if temporary(status.as_u16()) {
        let asked = retry_after(response.headers(), SystemTime::now());
        return Err(Failure::Retry(format!("HTTP {status}"), asked));
    } else {
        return Err(Failure::Fatal(format!("HTTP {status}")));
    };
    if total.is_some_and(|t| t > MAX_FILE) {
        return Err(Failure::Fatal("the file is larger than 4 GiB".to_owned()));
    }
    loop {
        let chunk = match tokio::time::timeout(IDLE, response.chunk()).await {
            Err(_elapsed) => return Err(retry(format!("no data for {}s", IDLE.as_secs_f32()))),
            Ok(Err(e)) => return Err(retry(describe(&e))),
            Ok(Ok(None)) => break,
            Ok(Ok(Some(chunk))) => chunk,
        };
        part.append(&chunk)?;
        meter.add(chunk.len(), part.size, total);
    }
    // A 206 may cover less than the rest of the file; ask again for what is missing.
    if let Some(total) = total
        && part.size != total
    {
        if part.size > total {
            part.restart()?;
        }
        return Err(retry(format!(
            "the server sent {} of {total} bytes",
            part.size
        )));
    }
    meter.report(part.size, total);
    Ok(())
}

/// The first byte and the complete length in a `Content-Range` header of the form
/// `bytes <first>-<last>/<length>` or `bytes <first>-<last>/*` (RFC 9110 section 14.4).
fn content_range(headers: &HeaderMap) -> Option<(u64, Option<u64>)> {
    let value = headers.get(CONTENT_RANGE)?.to_str().ok()?.trim();
    let (range, length) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (first, _last) = range.split_once('-')?;
    let length = if length == "*" {
        None
    } else {
        Some(length.parse().ok()?)
    };
    Some((first.parse().ok()?, length))
}

/// A validator for `If-Range`: a strong `ETag`, else `Last-Modified`. A weak `ETag` may not be
/// used there (RFC 9110 section 13.1.5).
fn validator(headers: &HeaderMap) -> Option<HeaderValue> {
    headers
        .get(ETAG)
        .filter(|etag| !etag.as_bytes().starts_with(b"W/"))
        .or_else(|| headers.get(LAST_MODIFIED))
        .cloned()
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

/// Whether `path` holds a file with the SHA-256 digest `expected`.
#[must_use]
pub fn is_cached(path: &Path, expected: &str) -> bool {
    sha256_file(path).is_ok_and(|actual| actual == expected)
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
fn digest_in_list<'a>(list: &'a str, file: &str) -> Option<&'a str> {
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

    /// Downloads `url` to a temporary file and returns its text.
    async fn fetch_text(url: &str) -> Result<String, HostError> {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f");
        fetch(&agent(), url, &dest, None, &mut ()).await?;
        Ok(fs::read_to_string(&dest).unwrap())
    }

    const ABC: &str = "abc";
    const ABC_SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[tokio::test]
    async fn retries_server_errors_until_success() {
        let (url, seen) = serve(&[503, 503, 200], ABC);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f");
        let digest = fetch(&agent(), &url, &dest, Some(ABC_SHA), &mut ())
            .await
            .unwrap();
        assert_eq!(digest, ABC_SHA);
        assert_eq!(fs::read_to_string(&dest).unwrap(), ABC);
        assert_eq!(seen.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn rate_limits_and_gateway_timeouts_are_retried() {
        let (url, _) = serve(&[429, 504, 200], ABC);
        assert_eq!(fetch_text(&url).await.unwrap(), ABC);
    }

    #[tokio::test]
    async fn client_errors_fail_at_once() {
        let (url, seen) = serve(&[404], "");
        let dir = tempfile::tempdir().unwrap();
        let err = fetch(&agent(), &url, &dir.path().join("f"), None, &mut ())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("404"), "{err}");
        assert_eq!(seen.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn gives_up_after_the_limit() {
        let (url, seen) = serve(&[503], "");
        let err = fetch_text(&url).await.unwrap_err();
        assert!(err.to_string().contains("503"), "{err}");
        assert_eq!(seen.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn only_temporary_statuses_are_retried() {
        let (url, seen) = serve(&[408, 502, 200], ABC);
        assert_eq!(fetch_text(&url).await.unwrap(), ABC);
        assert_eq!(seen.load(Ordering::SeqCst), 3);
        let fixed: [&'static [u16]; 3] = [&[500, 200], &[501, 200], &[505, 200]];
        for statuses in fixed {
            let (url, seen) = serve(statuses, ABC);
            assert!(fetch_text(&url).await.is_err(), "{statuses:?}");
            assert_eq!(seen.load(Ordering::SeqCst), 1, "{statuses:?}");
        }
    }

    #[tokio::test]
    async fn retry_after_is_honoured_as_seconds_or_a_date() {
        let (url, seen) = serve_with(&[503, 200], "Retry-After: 0\r\n", ABC);
        assert_eq!(fetch_text(&url).await.unwrap(), ABC);
        assert_eq!(seen.load(Ordering::SeqCst), 2);
        let (url, seen) = serve_with(
            &[429, 200],
            "Retry-After: Fri, 31 Dec 1999 23:59:59 GMT\r\n",
            ABC,
        );
        assert_eq!(fetch_text(&url).await.unwrap(), ABC);
        assert_eq!(seen.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_retry_after_beyond_the_cap_is_not_waited_for() {
        let (url, seen) = serve_with(&[503, 200], "Retry-After: 3600\r\n", ABC);
        let err = fetch_text(&url).await.unwrap_err();
        assert!(err.to_string().contains("503"), "{err}");
        assert_eq!(seen.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn digest_mismatch_is_not_retried() {
        let (url, seen) = serve(&[200], ABC);
        let dir = tempfile::tempdir().unwrap();
        let wrong = "0".repeat(64);
        let err = fetch(&agent(), &url, &dir.path().join("f"), Some(&wrong), &mut ())
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
            assert_eq!(fetch_text(&url).await.unwrap(), ABC);
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

    /// One reply of [`serve_script`]: the bytes to send, and whether to keep the connection open
    /// without sending more.
    struct Reply {
        bytes: String,
        stall: bool,
    }

    /// Answers the n-th request with `answer(n)`; returns the URL and the request heads seen.
    fn serve_script(
        answer: impl Fn(usize) -> Reply + Send + 'static,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let heads = Arc::clone(&seen);
        thread::spawn(move || {
            #[expect(
                clippy::collection_is_never_read,
                reason = "holding a stalled stream keeps its connection open"
            )]
            let mut stalled = Vec::new();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut request = [0_u8; 4096];
                let n = stream.read(&mut request).unwrap_or(0);
                let mut heads = heads.lock().unwrap();
                heads.push(String::from_utf8_lossy(&request[..n]).to_ascii_lowercase());
                let reply = answer(heads.len() - 1);
                drop(heads);
                drop(stream.write_all(reply.bytes.as_bytes()));
                if reply.stall {
                    stalled.push(stream);
                }
            }
        });
        (url, seen)
    }

    const HELLO: &str = "hello world";
    const HELLO_SHA: &str = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

    /// The head of `HELLO` and the first five bytes, after which the connection closes or stalls.
    fn cut_short(stall: bool) -> Reply {
        Reply {
            bytes: "HTTP/1.1 200 OK\r\nContent-Length: 11\r\nETag: \"v1\"\r\n\r\nhello".to_owned(),
            stall,
        }
    }

    /// Bytes 5 to 10 of a file of 11 bytes.
    fn rest(body: &str) -> Reply {
        range(5, body)
    }

    /// `body` as the bytes from `first` of a file of 11 bytes.
    fn range(first: usize, body: &str) -> Reply {
        range_of(first, body, 11)
    }

    /// `body` as the bytes from `first` of a file of `length` bytes.
    fn range_of(first: usize, body: &str, length: usize) -> Reply {
        let last = first + body.len() - 1;
        Reply {
            bytes: format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {first}-{last}/{length}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
            stall: false,
        }
    }

    #[derive(Default)]
    struct Recorder {
        updates: Vec<(u64, Option<u64>)>,
        finished: usize,
    }

    impl Progress for Recorder {
        fn update(&mut self, done: u64, total: Option<u64>, _rate: u64) {
            self.updates.push((done, total));
        }

        fn finish(&mut self) {
            self.finished += 1;
        }
    }

    #[tokio::test]
    async fn a_retry_resumes_with_a_range() {
        let (url, seen) = serve_script(|n| {
            if n == 0 {
                cut_short(false)
            } else {
                rest(" world")
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f");
        let mut recorder = Recorder::default();
        let digest = fetch(&agent(), &url, &dest, Some(HELLO_SHA), &mut recorder)
            .await
            .unwrap();
        assert_eq!(digest, HELLO_SHA);
        assert_eq!(fs::read_to_string(&dest).unwrap(), HELLO);
        let heads = seen.lock().unwrap().clone();
        assert_eq!(heads.len(), 2, "{heads:?}");
        assert!(!heads[0].contains("range:"), "{heads:?}");
        assert!(!heads[0].contains("no-cache"), "{heads:?}");
        assert!(heads[1].contains("range: bytes=5-\r\n"), "{heads:?}");
        assert!(
            heads[1].contains("cache-control: no-cache\r\n"),
            "{heads:?}"
        );
        assert!(heads[1].contains("pragma: no-cache\r\n"), "{heads:?}");
        assert!(heads[1].contains("if-range: \"v1\"\r\n"), "{heads:?}");
        assert_eq!(recorder.updates.last(), Some(&(11, Some(11))));
        assert_eq!(recorder.finished, 1);
    }

    #[tokio::test]
    async fn a_short_range_is_followed_by_a_request_for_the_rest() {
        let (url, seen) = serve_script(|n| match n {
            0 => cut_short(false),
            1 => range(5, " wo"),
            _ => range(8, "rld"),
        });
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f");
        let digest = fetch(&agent(), &url, &dest, None, &mut ()).await.unwrap();
        assert_eq!(digest, HELLO_SHA);
        assert_eq!(fs::read_to_string(&dest).unwrap(), HELLO);
        let heads = seen.lock().unwrap().clone();
        assert_eq!(heads.len(), 3, "{heads:?}");
        assert!(heads[2].contains("range: bytes=8-\r\n"), "{heads:?}");
    }

    #[tokio::test]
    async fn a_part_of_a_file_of_another_length_starts_over() {
        let (url, seen) = serve_script(|n| match n {
            0 => cut_short(false),
            1 => range_of(5, " world!", 12),
            _ => Reply {
                bytes: format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{HELLO}"
                ),
                stall: false,
            },
        });
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f");
        let digest = fetch(&agent(), &url, &dest, Some(HELLO_SHA), &mut ())
            .await
            .unwrap();
        assert_eq!(digest, HELLO_SHA);
        let heads = seen.lock().unwrap().clone();
        assert_eq!(heads.len(), 3, "{heads:?}");
        assert!(!heads[2].contains("range:"), "{heads:?}");
    }

    #[tokio::test]
    async fn attempts_that_add_to_the_file_do_not_use_up_the_retries() {
        // Each answer sends one byte of the rest and closes: eleven attempts in all.
        let (url, seen) = serve_script(|n| {
            let byte = char::from(HELLO.as_bytes()[n]);
            let head = if n == 0 {
                "HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n".to_owned()
            } else {
                format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {n}-10/11\r\nContent-Length: {}\r\n\r\n",
                    11 - n
                )
            };
            Reply {
                bytes: format!("{head}{byte}"),
                stall: false,
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f");
        let digest = fetch(&agent(), &url, &dest, Some(HELLO_SHA), &mut ())
            .await
            .unwrap();
        assert_eq!(digest, HELLO_SHA);
        assert_eq!(seen.lock().unwrap().len(), 11);
    }

    #[tokio::test]
    async fn a_whole_answer_to_a_range_starts_over() {
        let (url, seen) = serve_script(|n| {
            if n == 0 {
                cut_short(false)
            } else {
                Reply {
                    bytes: format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{HELLO}"
                    ),
                    stall: false,
                }
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f");
        let digest = fetch(&agent(), &url, &dest, Some(HELLO_SHA), &mut ())
            .await
            .unwrap();
        assert_eq!(digest, HELLO_SHA);
        assert_eq!(fs::read_to_string(&dest).unwrap(), HELLO);
        assert!(seen.lock().unwrap()[1].contains("range: bytes=5-\r\n"));
    }

    #[tokio::test]
    async fn a_stalled_body_times_out_and_resumes() {
        let (url, seen) = serve_script(|_| cut_short(true));
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f");
        let mut recorder = Recorder::default();
        let err = fetch(&agent(), &url, &dest, None, &mut recorder)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no data for"), "{err}");
        let heads = seen.lock().unwrap().clone();
        assert_eq!(heads.len(), 4, "{heads:?}");
        assert!(
            heads[1..].iter().all(|h| h.contains("range: bytes=5-\r\n")),
            "{heads:?}"
        );
        assert!(!dest.exists());
        assert!(!dest.with_extension("partial").exists());
        assert_eq!(recorder.finished, 1);
    }

    #[tokio::test]
    async fn a_resumed_file_with_the_wrong_digest_is_removed() {
        let (url, seen) = serve_script(|n| {
            if n == 0 {
                cut_short(false)
            } else {
                rest(" WORLD")
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f");
        let err = fetch(&agent(), &url, &dest, Some(HELLO_SHA), &mut ())
            .await
            .unwrap_err();
        assert!(matches!(err, HostError::Digest { .. }), "{err}");
        assert_eq!(seen.lock().unwrap().len(), 2);
        assert!(!dest.exists());
        assert!(!dest.with_extension("partial").exists());
    }

    #[test]
    fn content_ranges() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_RANGE, HeaderValue::from_static("bytes 5-10/11"));
        assert_eq!(content_range(&headers), Some((5, Some(11))));
        headers.insert(CONTENT_RANGE, HeaderValue::from_static("bytes 5-10/*"));
        assert_eq!(content_range(&headers), Some((5, None)));
        headers.insert(CONTENT_RANGE, HeaderValue::from_static("bytes */11"));
        assert_eq!(content_range(&headers), None);
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
