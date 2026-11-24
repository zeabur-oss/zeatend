//! Clusters that zeabur-installer installed, which zeatend takes over.
//!
//! zeabur-installer (zeabur-oss/zeabur, `crates/zeabur-installer` at v0.2.0) installs K3s or RKE2
//! for a team (`use: team`, the default) and `KubeSolo` for one person (`use: personal`). Its
//! configuration has the keys of zeatend's and more. zeatend reads the keys it knows, and leaves
//! what the installer put on the cluster, such as Zeabur OSS and its catalog components, as it is.

use serde::Deserialize;

use crate::PlanError;
use crate::authentication::ApiServer;
use crate::cluster::{Distribution, Rancher, disabled_components};
use crate::config::{Config, Document, Host, Join, JoinConfig, Role};
use crate::longhorn::Storage;

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
    format!(
        "{}/zeabur-join-token",
        rancher.distribution().layout().config_dir
    )
}

/// The installer's state directory for `distribution`.
#[must_use]
pub const fn state_dir(distribution: Distribution) -> &'static str {
    match distribution {
        Distribution::KubeSolo => KUBESOLO_STATE_DIR,
        Distribution::K3s | Distribution::Rke2 => RANCHER_STATE_DIR,
    }
}

/// The keys of the installer's configuration file that zeatend reads. Its other keys are about
/// what runs on the cluster, which zeatend leaves alone.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallerConfig {
    #[serde(rename = "use")]
    usage: Option<String>,
    distribution: Option<Distribution>,
    k3s_version: Option<String>,
    rke2_version: Option<String>,
    kubesolo_version: Option<String>,
    host: Option<InstallerHost>,
    join: Option<InstallerJoin>,
    telemetry: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallerHost {
    #[serde(default)]
    tls_san: Vec<String>,
    #[serde(default)]
    require_encrypted_disk: bool,
    node_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallerJoin {
    server: String,
    #[serde(default)]
    role: Role,
    token: Option<String>,
    token_file: Option<String>,
    #[serde(default)]
    node_names: Vec<String>,
}

/// zeatend's configuration from the installer's configuration file `yaml`.
///
/// It keeps the distribution (`use: personal` is `KubeSolo`), the release settings, the host
/// settings and the join section.
///
/// # Errors
///
/// When the YAML does not parse or a value zeatend reads is invalid.
pub fn document(yaml: &str) -> Result<Document, PlanError> {
    let error = |e: &dyn std::fmt::Display| PlanError::Config(format!("{CONFIG_FILE}: {e}"));
    let installer: InstallerConfig = serde_saphyr::from_str(yaml).map_err(|e| error(&e))?;
    let personal = installer.usage.as_deref() == Some("personal");
    // A personal configuration has no distribution, release or join keys. Otherwise the installer
    // names the release of each distribution in a key of its own.
    let (distribution, version) = if personal {
        (Distribution::KubeSolo, None)
    } else {
        let distribution = installer.distribution.unwrap_or_default();
        let version = match distribution {
            Distribution::K3s => installer.k3s_version,
            Distribution::Rke2 => installer.rke2_version,
            Distribution::KubeSolo => installer.kubesolo_version,
        };
        (distribution, version)
    };
    let host = installer.host.map_or_else(Host::default, |host| Host {
        tls_san: host.tls_san,
        require_encrypted_disk: host.require_encrypted_disk,
        node_name: host.node_name,
    });
    let Some(installer_join) = installer.join.filter(|_| !personal) else {
        let config = Config {
            distribution,
            version,
            host,
            telemetry: installer.telemetry,
            ..Config::default()
        };
        config.check()?;
        return Ok(Document::Install(config));
    };
    let mut join = Join {
        server: installer_join.server,
        role: installer_join.role,
        token: installer_join.token,
        token_file: installer_join.token_file,
        node_names: installer_join.node_names,
    };
    if join.token.is_none() && join.token_file.is_none() {
        // The installer keeps the token of a node it joined in a file of its own.
        join.token_file = distribution.rancher().map(join_token_file);
    }
    let config = JoinConfig {
        join,
        distribution,
        version,
        disable: Vec::new(),
        api_server: ApiServer::default(),
        storage: Storage::default(),
        host,
        telemetry: installer.telemetry,
    };
    config.check()?;
    Ok(Document::Join(config))
}

/// `document` with `disable` set to what the installer's cluster turns off.
///
/// `rancher_config` is the installer's K3s or RKE2 configuration; `apply` then keeps its
/// components off. A worker deploys no packaged components and keeps none.
///
/// # Errors
///
/// When `rancher_config` does not parse, or turns off a component zeatend cannot name.
pub fn keep_disabled(mut document: Document, rancher_config: &str) -> Result<Document, PlanError> {
    let error = |e: PlanError| PlanError::Config(format!("the installer's cluster: {e}"));
    match &mut document {
        Document::Install(config) => {
            config.disable =
                disabled_components(config.distribution, rancher_config).map_err(error)?;
        }
        Document::Join(config) if config.join.role == Role::Server => {
            config.disable =
                disabled_components(config.distribution, rancher_config).map_err(error)?;
        }
        Document::Join(_) => {}
    }
    Ok(document)
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
    fn the_installers_release_key_of_the_distribution_is_the_version() {
        let version = |yaml: &str| match document(yaml).unwrap() {
            Document::Install(config) => config.version,
            Document::Join(config) => config.version,
        };
        assert_eq!(
            version("k3sVersion: v1.37.1+k3s1\nrke2Version: v1.37.1+rke2r1\n").as_deref(),
            Some("v1.37.1+k3s1")
        );
        assert_eq!(
            version("distribution: rke2\nk3sVersion: v1.37.1+k3s1\nrke2Version: v1.37.1+rke2r1\n")
                .as_deref(),
            Some("v1.37.1+rke2r1")
        );
        assert_eq!(
            version("distribution: rke2\nk3sVersion: v1.37.1+k3s1\n"),
            None
        );
    }

    #[test]
    fn a_personal_config_is_kubesolo_and_the_default_is_k3s() {
        let Document::Install(personal) = document("use: personal\ntelemetry: false\n").unwrap()
        else {
            panic!("an install");
        };
        assert_eq!(personal.distribution, Distribution::KubeSolo);
        assert_eq!(personal.telemetry, Some(false));
        let Document::Install(personal) =
            document("use: personal\ndistribution: rke2\njoin:\n  server: https://a:9345\n")
                .unwrap()
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

    #[test]
    fn the_installers_cluster_keeps_off_what_it_turned_off() {
        use crate::config::Component;
        let rke2 = keep_disabled(
            document("distribution: rke2\n").unwrap(),
            "ingress-controller: none\ndisable:\n  - rke2-metrics-server\n",
        )
        .unwrap();
        let Document::Install(config) = rke2 else {
            panic!("an install");
        };
        assert_eq!(
            config.disable,
            [Component::Traefik, Component::MetricsServer]
        );
        let worker = document(
            "join:\n  server: https://10.0.0.5:6443\n  tokenFile: /etc/zeatend/join-token\n",
        )
        .unwrap();
        assert_eq!(
            keep_disabled(worker.clone(), "disable: [traefik]\n").unwrap(),
            worker
        );
        assert!(keep_disabled(document("{}\n").unwrap(), "disable: [servicelb]\n").is_err());
    }
}
