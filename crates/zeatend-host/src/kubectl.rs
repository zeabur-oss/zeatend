//! The cluster, through the `kubectl` and admin kubeconfig of K3s or RKE2.
//!
//! A subprocess rather than a client library: zeatend applies a handful of objects and
//! waits on them, and `kubectl` is already on the host, matched to the server.

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

    /// The same `kubectl` as a command line a person can run on this machine.
    #[must_use]
    pub fn command_line(&self) -> String {
        format!(
            "{} --kubeconfig {}",
            self.binary.display(),
            self.kubeconfig.display()
        )
    }

    /// Runs `kubectl` with the kubeconfig.
    ///
    /// # Errors
    ///
    /// When it fails.
    pub async fn run(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<String, HostError> {
        let kubeconfig = format!("--kubeconfig={}", self.kubeconfig.display());
        let mut all = vec![kubeconfig.as_str()];
        all.extend_from_slice(args);
        cmd::run(&self.binary, &all, stdin).await
    }

    /// Server-side applies YAML or JSON documents as zeatend's field manager.
    ///
    /// # Errors
    ///
    /// When the server rejects them.
    pub async fn apply(&self, documents: &[u8]) -> Result<String, HostError> {
        self.apply_as(MANAGER, documents).await
    }

    /// Server-side applies documents as `manager`. Fields another manager set stay; only the ones
    /// `manager` set before and leaves out now are removed.
    ///
    /// # Errors
    ///
    /// When the server rejects them.
    pub async fn apply_as(&self, manager: &str, documents: &[u8]) -> Result<String, HostError> {
        let manager = format!("--field-manager={manager}");
        self.run(
            &[
                "apply",
                "--server-side",
                "--force-conflicts",
                &manager,
                "-f",
                "-",
            ],
            Some(documents),
        )
        .await
    }

    /// Server-side applies documents into `namespace` when they name none.
    ///
    /// A chart's install job finishes before its pods serve, so objects checked by its webhook or
    /// of a kind it has only just defined are retried for a few minutes.
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

    /// The kubeconfig, for other tools such as a person's shell.
    #[must_use]
    pub fn kubeconfig(&self) -> &std::path::Path {
        &self.kubeconfig
    }

    /// Server-side applies one object.
    ///
    /// # Errors
    ///
    /// When the server rejects it.
    pub async fn apply_object(&self, object: &Value) -> Result<(), HostError> {
        self.apply(object.to_string().as_bytes()).await.map(drop)
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
             calling webhook \"vlsingles.operator.victoriametrics.com\": failed to call webhook: \
             no endpoints available for service \"victoria-metrics-operator\""
        ));
        assert!(!not_serving_yet(
            "Error from server (Invalid): VLSingle.operator.victoriametrics.com \"logs\" is invalid"
        ));
    }
}
