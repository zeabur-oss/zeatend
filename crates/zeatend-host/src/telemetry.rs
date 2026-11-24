//! The installation ID of the experience program and the one request that sends an event.

use std::fs;
use std::path::Path;
use std::time::Duration;

use zeatend_plan::telemetry::InstallationId;

use crate::{HostError, files};

/// `PostHog`'s batch endpoint, through Zeabur's own domain.
pub const ENDPOINT: &str = "https://oss-nori.zeabur.com/batch/";

/// The `PostHog` project key, `ZEATEND_TELEMETRY_KEY` at build time: only release builds have it.
/// Empty: nothing is sent.
pub const PROJECT_KEY: &str = match option_env!("ZEATEND_TELEMETRY_KEY") {
    Some(key) => key,
    None => "",
};

/// How long the whole request may take.
const TIMEOUT: Duration = Duration::from_secs(5);

/// The ID in the file at `path`; `None` when there is none or it is not an ID. Nothing is made.
#[must_use]
pub fn peek_id(path: &Path) -> Option<InstallationId> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| InstallationId::parse(&text))
}

/// The ID in the file at `path`, made and kept first (mode 0600) when there is none.
///
/// # Errors
///
/// When the file cannot be written or there is no random number.
pub fn id(path: &Path) -> Result<InstallationId, HostError> {
    if let Some(id) = peek_id(path) {
        return Ok(id);
    }
    let mut bytes = [0_u8; 16];
    aws_lc_rs::rand::fill(&mut bytes).map_err(|_unspecified| HostError::Random)?;
    let id = InstallationId::from_random(bytes);
    files::write(path, 0o600, format!("{}\n", id.as_str()).as_bytes())?;
    Ok(id)
}

/// Posts `body` to `endpoint` once, within five seconds. It goes through the proxy that
/// `HTTPS_PROXY` names, as downloads do.
///
/// # Errors
///
/// When there is no answer in time, or the request fails.
pub async fn post(endpoint: &str, body: String) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("zeatend/", env!("CARGO_PKG_VERSION")))
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map(drop)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    #[test]
    fn the_id_is_made_once_and_kept_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zeatend").join("installation-id");
        assert_eq!(peek_id(&path), None);
        let made = id(&path).unwrap();
        assert_eq!(id(&path).unwrap(), made);
        assert_eq!(peek_id(&path), Some(made));
        let bits = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(bits & 0o777, 0o600);
    }

    #[test]
    fn a_file_that_is_not_an_id_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installation-id");
        fs::write(&path, "not an id").unwrap();
        assert_eq!(peek_id(&path), None);
        let made = id(&path).unwrap();
        assert_eq!(peek_id(&path), Some(made));
    }

    #[tokio::test]
    async fn the_body_is_posted_as_json() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/batch/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            while !String::from_utf8_lossy(&request).ends_with("{\"a\":1}") {
                let n = stream.read(&mut chunk).unwrap();
                assert_ne!(n, 0, "the request ended early");
                request.extend_from_slice(chunk.get(..n).unwrap());
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        post(&url, "{\"a\":1}".to_owned()).await.unwrap();
        let request = server.join().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("post /batch/ "));
        assert!(request.contains("content-type: application/json"));
    }

    #[tokio::test]
    async fn no_answer_is_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/batch/", listener.local_addr().unwrap());
        drop(listener);
        assert!(post(&url, "{}".to_owned()).await.is_err());
    }
}
