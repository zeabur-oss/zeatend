//! Clusters that zeabur-installer installed, which zeatend takes over.
//!
//! zeabur-installer (zeabur-oss/zeabur, `crates/zeabur-installer` at v0.2.0) installs K3s or RKE2
//! for a team (`use: team`, the default) and `KubeSolo` for one person (`use: personal`). Its
//! configuration has the keys of zeatend's and more. zeatend reads the keys it knows, and leaves
//! what the installer put on the cluster, such as Zeabur and its catalog components, as it is.

use serde_json::{Map, Value};

use crate::PlanError;
use crate::cluster::{Distribution, Rancher};
use crate::config::{Config, Document, JoinConfig};

/// The installer's configuration file.
pub const CONFIG_FILE: &str = "/etc/zeabur-installer/config.yaml";
/// The installer's state directory on a K3s or RKE2 server.
pub const RANCHER_STATE_DIR: &str = "/var/lib/rancher/zeabur-installer";
/// The installer's state directory on a `KubeSolo` host.
pub const KUBESOLO_STATE_DIR: &str = "/var/lib/zeabur-installer";
/// The file in a state directory that holds the version of the installer that last applied.
pub const VERSION_FILE: &str = "installer-version";
/// The kubectl the installer downloads on a `KubeSolo` host.
pub const KUBESOLO_KUBECTL: &str = "/var/lib/zeabur-installer/bin/kubectl";
/// The kubeconfig the installer writes on a `KubeSolo` host for the person's computer.
pub const KUBESOLO_KUBECONFIG: &str = "/etc/zeabur-installer/kubeconfig.yaml";
/// The script the installer's `KubeSolo` unit runs after each start.
pub const KUBESOLO_WEBHOOK_SCRIPT: &str = "/usr/local/libexec/zeabur-kubesolo-webhook";

/// The file that holds the join token on a node the installer joined to a K3s or RKE2 cluster.
#[must_use]
pub fn join_token_file(rancher: Rancher) -> String {
    format!("{}/zeabur-join-token", rancher.layout().config_dir)
}

/// The installer's state directory for `distribution`.
#[must_use]
pub const fn state_dir(distribution: Distribution) -> &'static str {
    match distribution {
        Distribution::KubeSolo => KUBESOLO_STATE_DIR,
        Distribution::K3s | Distribution::Rke2 => RANCHER_STATE_DIR,
    }
}

/// zeatend's configuration from the installer's configuration file `yaml`.
///
/// It keeps the distribution (`use: personal` is `KubeSolo`), the release settings, the host
/// settings and the join section. The installer's other keys are about what runs on the cluster,
/// which zeatend leaves alone.
///
/// # Errors
///
/// When the YAML does not parse or a value zeatend reads is invalid.
pub fn document(yaml: &str) -> Result<Document, PlanError> {
    let installer: Value = serde_saphyr::from_str(yaml)
        .map_err(|e| PlanError::Config(format!("{CONFIG_FILE}: {e}")))?;
    let mut config = Map::new();
    if installer.get("use").and_then(Value::as_str) == Some("personal") {
        config.insert("distribution".to_owned(), Value::from("kubesolo"));
    }
    keep(
        &mut config,
        &installer,
        &["distribution", "rke2Version", "k3sVersion"],
    );
    if let Some(host) = installer.get("host") {
        let mut kept = Map::new();
        keep(
            &mut kept,
            host,
            &["tlsSan", "requireEncryptedDisk", "nodeName"],
        );
        config.insert("host".to_owned(), Value::Object(kept));
    }
    let error = |e: serde_json::Error| PlanError::Config(format!("{CONFIG_FILE}: {e}"));
    let Some(installer_join) = installer.get("join") else {
        let config: Config = serde_json::from_value(Value::Object(config)).map_err(error)?;
        config.check()?;
        return Ok(Document::Install(config));
    };
    let mut join = Map::new();
    keep(
        &mut join,
        installer_join,
        &["server", "role", "token", "tokenFile", "nodeNames"],
    );
    if !join.contains_key("token") && !join.contains_key("tokenFile") {
        // The installer keeps the token of a node it joined in a file of its own.
        let distribution: Distribution = serde_json::from_value(
            config
                .get("distribution")
                .cloned()
                .unwrap_or_else(|| Value::from("k3s")),
        )
        .map_err(error)?;
        if let Some(rancher) = distribution.rancher() {
            join.insert("tokenFile".to_owned(), join_token_file(rancher).into());
        }
    }
    config.insert("join".to_owned(), Value::Object(join));
    let config: JoinConfig = serde_json::from_value(Value::Object(config)).map_err(error)?;
    config.check()?;
    Ok(Document::Join(config))
}

/// Copies the values of `keys` that `from` has into `to`.
fn keep(to: &mut Map<String, Value>, from: &Value, keys: &[&str]) {
    for key in keys {
        if let Some(value) = from.get(key) {
            to.insert((*key).to_owned(), value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Role;

    #[test]
    fn a_team_config_keeps_the_keys_zeatend_knows() {
        let yaml = "distribution: rke2\n\
                    access:\n  mode: local\n  identity: id.example.com\n  zeabur: z.example.com\n\
                    telemetry: false\n\
                    host:\n  tlsSan: [api.example.com]\n  nodeName: node-1\n";
        let Document::Install(config) = document(yaml).unwrap() else {
            panic!("an install");
        };
        assert_eq!(config.distribution, Distribution::Rke2);
        assert_eq!(config.host.tls_san, ["api.example.com"]);
        assert_eq!(config.host.node_name.as_deref(), Some("node-1"));
    }

    #[test]
    fn a_personal_config_is_kubesolo_and_the_default_is_k3s() {
        let Document::Install(personal) = document("use: personal\ntelemetry: false\n").unwrap()
        else {
            panic!("an install");
        };
        assert_eq!(personal.distribution, Distribution::KubeSolo);
        let Document::Install(team) =
            document("access:\n  mode: domain\n  identity: a\n  zeabur: b\n").unwrap()
        else {
            panic!("an install");
        };
        assert_eq!(team.distribution, Distribution::K3s);
    }

    #[test]
    fn a_joined_node_reads_its_token_from_the_installer_file() {
        let yaml = "distribution: rke2\n\
                    join:\n  server: https://10.0.0.5:9345\n  role: server\n  \
                    nodeNames: [a]\n  longhorn: true\n";
        let Document::Join(config) = document(yaml).unwrap() else {
            panic!("a join");
        };
        assert_eq!(config.join.role, Role::Server);
        assert_eq!(
            config.join.token_file.as_deref(),
            Some("/etc/rancher/rke2/zeabur-join-token")
        );
        assert_eq!(config.join.node_names, ["a"]);
    }

    #[test]
    fn an_invalid_value_is_refused() {
        assert!(document("distribution: nomad\n").is_err());
        assert!(document("k3sVersion: latest\n").is_err());
    }
}
