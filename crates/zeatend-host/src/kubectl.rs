//! The cluster, through the `kubectl` and admin kubeconfig of K3s, RKE2 or `KubeSolo`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use zeatend_plan::cluster::Distribution;

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
    pub fn admin(distribution: Distribution) -> Self {
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
                .map(|_| true)
        })
        .await
    }

    /// Waits for `object`, such as `deployment/coredns`, to exist and then for its rollout to
    /// complete, each within `timeout`.
    ///
    /// # Errors
    ///
    /// When it does not, with the reason `kubectl` gives.
    pub async fn rollout(
        &self,
        namespace: &str,
        object: &str,
        timeout: Duration,
    ) -> Result<(), HostError> {
        let timeout = format!("--timeout={}s", timeout.as_secs());
        let namespaced = ["--namespace", namespace, object, &timeout];
        self.wait(&[&namespaced[..], &["--for=create"]].concat())
            .await?;
        self.run(&[&["rollout", "status"][..], &namespaced].concat(), None)
            .await
            .map(drop)
    }
}

/// Calls `ready` every few seconds until it returns `Ok(true)`. It calls `ready` at least once.
///
/// # Errors
///
/// When it does not within `timeout`. The error holds the last error of `ready`, if the last try
/// failed.
pub async fn poll<E>(
    timeout: Duration,
    what: &str,
    mut ready: impl AsyncFnMut() -> Result<bool, E>,
) -> Result<(), E>
where
    E: From<HostError> + std::fmt::Display,
{
    let start = Instant::now();
    loop {
        let last = match ready().await {
            Ok(true) => return Ok(()),
            Ok(false) => None,
            Err(e) => Some(e),
        };
        if start.elapsed() >= timeout {
            let mut reason = format!("not done after {} s", timeout.as_secs());
            if let Some(e) = last {
                reason = format!("{reason}: {e}");
            }
            return Err(HostError::Command {
                command: format!("waiting for {what}"),
                reason,
            }
            .into());
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_timeout_reports_the_last_error() {
        let error = poll(Duration::ZERO, "the widget", async || {
            Err::<bool, HostError>(HostError::Command {
                command: "kubectl wait".to_owned(),
                reason: "deployments.apps \"widget\" not found".to_owned(),
            })
        })
        .await
        .unwrap_err()
        .to_string();
        assert!(
            error.starts_with("waiting for the widget: not done"),
            "{error}"
        );
        assert!(error.contains("\"widget\" not found"), "{error}");
    }

    #[tokio::test]
    async fn a_timeout_without_an_error_says_what_is_not_done() {
        let error = poll(Duration::ZERO, "the widget", async || {
            Ok::<_, HostError>(false)
        })
        .await
        .unwrap_err()
        .to_string();
        assert_eq!(error, "waiting for the widget: not done after 0 s");
    }
}
