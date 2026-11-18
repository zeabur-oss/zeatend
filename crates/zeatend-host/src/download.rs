//! Downloads checked against a SHA-256 digest.

use std::fs::{self, File};
use std::io::{self, BufRead as _, Write as _};
use std::path::Path;
use std::time::{Duration, SystemTime};

use reqwest::header::HeaderMap;
use reqwest::redirect::{self, Attempt};
use reqwest::{Client, Response};
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

/// How long reading an answer's body may take.
const BODY_TIME: Duration = Duration::from_hours(1);

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

/// Downloads `url` to `dest` unless a file with the expected digest is already there. Returns the
/// digest. With no expected digest the file is always downloaded; check the returned digest
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
        && is_cached(dest, expected)
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
        let mut response = response;
        let mut size: u64 = 0;
        let reading = async {
            while let Some(chunk) = response.chunk().await.map_err(|e| fail(describe(&e)))? {
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

/// A GET that succeeds, or the reason it did not.
async fn get_ok(https: &Client, url: &str) -> Result<Response, String> {
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
async fn get_retrying(https: &Client, url: &str) -> Result<Response, String> {
    let mut wait = BACKOFF;
    let mut attempt = 0;
    loop {
        let asked = match get(https, url).await {
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

/// One GET, with its redirects.
async fn get(https: &Client, url: &str) -> Result<Response, String> {
    tokio::time::timeout(HEAD_TIME, https.get(url).send())
        .await
        .map_err(|_elapsed| "the server did not answer in time".to_owned())?
        .map_err(|e| describe(&e))
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
        fetch(&agent(), url, &dest, None).await?;
        Ok(fs::read_to_string(&dest).unwrap())
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
    async fn rate_limits_and_gateway_timeouts_are_retried() {
        let (url, _) = serve(&[429, 504, 200], ABC);
        assert_eq!(fetch_text(&url).await.unwrap(), ABC);
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
