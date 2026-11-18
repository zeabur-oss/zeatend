//! The cluster, through the `kubectl` and admin kubeconfig of K3s or RKE2.
//!
//! A subprocess rather than a client library: zeatend reads a few objects and waits on them, and
//! `kubectl` is already on the host, matched to the server.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use zeatend_plan::cluster::Rancher;
use zeatend_plan::plan::MANAGER;

use crate::{HostError, cmd};

/// `kubectl` with a kubeconfig.
#[derive(Debug, Clone)]
pub struct Kubectl {
    binary: PathBuf,
    kubeconfig: PathBuf,
}

impl Kubectl {
    /// The `kubectl` of the distribution with its admin kubeconfig.
    #[must_use]
    pub fn admin(distribution: Rancher) -> Self {
        let layout = distribution.layout();
        Self::new(layout.kubectl, layout.admin_kubeconfig)
    }

    /// A `kubectl` binary with another kubeconfig, such as the kubelet's on an agent.
    #[must_use]
    pub fn new(binary: impl Into<PathBuf>, kubeconfig: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            kubeconfig: kubeconfig.into(),
        }
    }

    /// Runs `kubectl` with the kubeconfig.
    async fn run(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<String, HostError> {
        let kubeconfig = format!("--kubeconfig={}", self.kubeconfig.display());
        let mut all = vec![kubeconfig.as_str()];
        all.extend_from_slice(args);
        cmd::run(&self.binary, &all, stdin).await
    }

    /// Server-side applies documents into `namespace` when they name none.
    ///
    /// An admission webhook can be registered before its pods serve, and a kind can be applied
    /// just after its `CustomResourceDefinition`, before the API server serves it. Such objects
    /// are retried for a few minutes.
    ///
    /// # Errors
    ///
    /// When the server rejects them.
    pub async fn apply_in(&self, namespace: &str, documents: &[u8]) -> Result<String, HostError> {
        let manager = format!("--field-manager={MANAGER}");
        let args = [
            "apply",
            "--server-side",
            "--force-conflicts",
            &manager,
            "--namespace",
            namespace,
            "-f",
            "-",
        ];
        let start = Instant::now();
        loop {
            match self.run(&args, Some(documents)).await {
                Err(HostError::Command { reason, .. })
                    if not_serving_yet(&reason) && start.elapsed() < Duration::from_mins(5) =>
                {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                result => return result,
            }
        }
    }

    /// Gets an object as JSON, or `None` when it does not exist.
    ///
    /// # Errors
    ///
    /// When the request fails for another reason.
    pub async fn get(
        &self,
        kind: &str,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<Option<Value>, HostError> {
        let mut args = vec!["get", kind, name, "--ignore-not-found", "--output=json"];
        if let Some(ns) = namespace {
            args.extend(["--namespace", ns]);
        }
        let out = self.run(&args, None).await?;
        if out.trim().is_empty() {
            return Ok(None);
        }
        serde_json::from_str(&out)
            .map(Some)
            .map_err(|e| HostError::Parse {
                what: format!("kubectl get {kind} {name}"),
                reason: e.to_string(),
            })
    }

    /// `kubectl wait` with the given arguments.
    ///
    /// # Errors
    ///
    /// When the condition is not met in time.
    pub async fn wait(&self, args: &[&str]) -> Result<(), HostError> {
        let mut all = vec!["wait"];
        all.extend_from_slice(args);
        self.run(&all, None).await.map(drop)
    }

    /// Waits for the API server to answer `/readyz`.
    ///
    /// # Errors
    ///
    /// When it does not within `timeout`.
    pub async fn wait_ready(&self, timeout: Duration) -> Result<(), HostError> {
        poll(timeout, "the API server to become ready", async || {
            self.run(&["get", "--raw=/readyz"], None).await.is_ok()
        })
        .await
    }
}

/// A Kubernetes timestamp (`2026-10-23T08:00:30Z`, RFC 3339) in seconds since the Unix epoch.
#[must_use]
pub fn unix_seconds(text: &str) -> Option<u64> {
    let at: jiff::Timestamp = text.parse().ok()?;
    u64::try_from(at.as_second()).ok()
}

/// Calls `ready` every few seconds until it returns `true`.
///
/// # Errors
///
/// When it does not within `timeout`.
pub async fn poll(
    timeout: Duration,
    what: &str,
    mut ready: impl AsyncFnMut() -> bool,
) -> Result<(), HostError> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if ready().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    Err(HostError::Command {
        command: format!("waiting for {what}"),
        reason: format!("not done after {} s", timeout.as_secs()),
    })
}

fn not_serving_yet(reason: &str) -> bool {
    reason.contains("failed calling webhook")
        || reason.contains("no endpoints available for service")
        || reason.contains("ensure CRDs are installed first")
}

/// A Secret object, labelled as zeatend's.
#[must_use]
pub fn secret(namespace: &str, name: &str, data: &BTreeMap<String, Vec<u8>>) -> Value {
    let data: BTreeMap<&str, String> = data
        .iter()
        .map(|(k, v)| (k.as_str(), STANDARD.encode(v)))
        .collect();
    json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {"name": name, "namespace": namespace, "labels": {"app.kubernetes.io/managed-by": MANAGER}},
        "type": "Opaque",
        "data": data,
    })
}

/// A key of a Secret object, decoded.
#[must_use]
pub fn secret_key(secret: &Value, key: &str) -> Option<Vec<u8>> {
    STANDARD
        .decode(secret.get("data")?.get(key)?.as_str()?)
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps() {
        assert_eq!(unix_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_seconds("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(unix_seconds("2026-10-23T08:00:30Z"), Some(1_792_742_430));
        assert_eq!(unix_seconds("2026-10-23T08:00:30.5Z"), Some(1_792_742_430));
        assert_eq!(
            unix_seconds("2026-10-23T16:00:30+08:00"),
            Some(1_792_742_430)
        );
        assert_eq!(unix_seconds("1969-12-31T23:59:59Z"), None);
        assert_eq!(unix_seconds("yesterday"), None);
    }

    #[test]
    fn secrets_round_trip() {
        let data = BTreeMap::from([("k".to_owned(), b"v".to_vec())]);
        let s = secret("ns", "n", &data);
        assert_eq!(s["data"]["k"], "dg==");
        assert_eq!(secret_key(&s, "k").unwrap(), b"v");
        assert!(secret_key(&s, "x").is_none());
    }

    #[test]
    fn webhooks_not_yet_serving_are_retried() {
        assert!(not_serving_yet(
            "exit status: 1\nError from server (InternalError): Internal error occurred: failed \
             calling webhook \"validate.widgets.example.com\": failed to call webhook: no \
             endpoints available for service \"widget-webhook\""
        ));
        assert!(not_serving_yet(
            "error: resource mapping not found for name: \"one\" namespace: \"\" from \"STDIN\": \
             no matches for kind \"Widget\" in version \"example.com/v1\"\nensure CRDs are \
             installed first"
        ));
        assert!(!not_serving_yet(
            "Error from server (Invalid): Widget.example.com \"one\" is invalid"
        ));
    }
}
