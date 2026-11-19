//! The cluster, through the `kubectl` and admin kubeconfig of K3s or RKE2.
//!
//! A subprocess rather than a client library: zeatend reads a few objects and waits on them, and
//! `kubectl` is already on the host, matched to the server.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use zeatend_plan::cluster::Rancher;
use zeatend_plan::plan::MANAGER;

use crate::{HostError, cmd};

/// How long a read waits for the API server to answer. `kubectl` waits without end by default
/// (`--request-timeout` is `0`), so an API server that accepts a connection and does not answer
/// would hang the caller.
const READ_TIMEOUT: &str = "--request-timeout=10s";

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

    /// Gets an object, such as a `k8s_openapi` `Node`, or `None` when it does not exist.
    ///
    /// # Errors
    ///
    /// When the request fails for another reason, or the answer is not a `T`.
    pub async fn get<T: DeserializeOwned>(
        &self,
        kind: &str,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<Option<T>, HostError> {
        let mut args = vec![
            "get",
            kind,
            name,
            "--ignore-not-found",
            "--output=json",
            READ_TIMEOUT,
        ];
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

    /// Gets an API path, such as `/api/v1/nodes`.
    ///
    /// # Errors
    ///
    /// When the request fails or the answer is not a `T`.
    pub async fn raw<T: DeserializeOwned>(&self, path: &str) -> Result<T, HostError> {
        let raw = format!("--raw={path}");
        let out = self.run(&["get", &raw, READ_TIMEOUT], None).await?;
        serde_json::from_str(&out).map_err(|e| HostError::Parse {
            what: format!("kubectl get --raw {path}"),
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

    /// Waits for the API server to answer `/readyz`. Each try waits at most 10 s for an answer,
    /// so this returns at most 10 s after `timeout`.
    ///
    /// # Errors
    ///
    /// When it does not within `timeout`.
    pub async fn wait_ready(&self, timeout: Duration) -> Result<(), HostError> {
        poll(timeout, "the API server to become ready", async || {
            self.run(&["get", "--raw=/readyz", READ_TIMEOUT], None)
                .await
                .is_ok()
        })
        .await
    }
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
