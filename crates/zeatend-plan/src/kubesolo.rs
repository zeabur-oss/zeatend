//! `KubeSolo`: its files, ports and kubeconfig.
//!
//! `KubeSolo` is one process with `SQLite`, a bridge CNI and no scheduler. It enforces no
//! `NetworkPolicy`, so it isolates nothing between namespaces. It reads `/etc/kubesolo/config.yaml`
//! (`kubesolo --print-config` shows every key) and keeps its state in `/var/lib/kubesolo`.

use serde::{Deserialize, Serialize};

use crate::cluster::{APPARMOR_PROFILE, Distribution, HostFile, PortUse};
use crate::config::Role;
use crate::installer::{self, KUBESOLO_WEBHOOK_SCRIPT};
use crate::tuning::HostTuning;
use crate::uninstall::{Remove, parent};
use crate::{PlanError, ports};

/// Its configuration file.
pub const CONFIG_FILE: &str = "/etc/kubesolo/config.yaml";
/// The unit file.
pub const UNIT_FILE: &str = "/etc/systemd/system/kubesolo.service";
/// The kubeconfig zeatend writes for the person's own computer.
pub const KUBECONFIG: &str = "/etc/zeatend/kubeconfig.yaml";
/// The directory of the authentication file that `kubernetes.apiServer.authenticationConfigFile`
/// names.
pub const AUTHENTICATION_DIR: &str = "/etc/kubesolo/authentication";
/// The CNI configuration `KubeSolo` writes on every start.
pub const CNI_CONFIG: &str = "/etc/cni/net.d/10-bridge.conflist";

/// The least memory `KubeSolo` is planned for.
///
/// It is a host with 1.5 GiB, less what the kernel and firmware keep. `KubeSolo` with its own pods
/// (`CoreDNS`, local-path-provisioner) and a helm-controller used about 670 MiB on a 1 CPU /
/// 1.5 GiB arm64 VM. `KubeSolo` does not run a helm-controller and zeatend does not install one,
/// so it uses less.
pub const MIN_MEMORY_MIB: u64 = 1400;

/// The TCP ports `KubeSolo` listens on, and what for.
#[must_use]
pub fn ports() -> Vec<(u16, PortUse)> {
    vec![
        (ports::HTTP, PortUse::KubeSoloHttp),
        (ports::HTTPS, PortUse::KubeSoloHttps),
        (ports::API_SERVER, PortUse::KubernetesApi),
        (ports::KUBELET, PortUse::Kubelet),
        (ports::KUBESOLO_WEBHOOK, PortUse::KubeSoloWebhook),
    ]
}

/// What the configuration file sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The node's name, so a later change of the host name does not register a second node.
    pub node_name: String,
    /// The address the node registers with and the kubeconfig names; `None` lets `KubeSolo`
    /// choose, preferring a private address.
    pub node_ip: Option<String>,
    /// Extra API server certificate names.
    pub tls_san: Vec<String>,
    /// The JWT issuers the API server trusts.
    pub api_server: crate::authentication::ApiServer,
    /// The host's memory settings; the unit sets their `GOMEMLIMIT`.
    pub tuning: HostTuning,
    /// The unit runs zeabur-installer's webhook script after each start, as the installer's unit
    /// did.
    ///
    /// The installer adds helm-controller to `KubeSolo`. `KubeSolo`'s webhook adds the node to a
    /// new Job's `nodeSelector`, so helm-controller finds its install Job changed and no chart
    /// installs. The script takes Jobs out of the webhook's rules.
    pub installer_webhook: bool,
}

const HEADER: &str = "# Written by zeatend; `zeatend apply` writes it again.\n";

/// The keys zeatend writes and reads in the configuration file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// `kubesolo.io/v1alpha1`.
    #[serde(default)]
    pub api_version: String,
    /// `Config`.
    #[serde(default)]
    pub kind: String,
    /// The node's address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<Network>,
    /// The node and the API server.
    #[serde(default)]
    pub kubernetes: Kubernetes,
}

/// The `network` keys of [`Config`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Network {
    /// The address the node registers with.
    #[serde(rename = "nodeIP", default)]
    pub node_ip: String,
}

/// The `kubernetes` keys of [`Config`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Kubernetes {
    /// The node's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    /// Extra API server certificate names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_server: Option<ApiServer>,
}

/// The `kubernetes.apiServer` keys of [`Config`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiServer {
    /// Extra API server certificate names.
    #[serde(rename = "extraSANs", default)]
    pub extra_sans: Vec<String>,
    /// The file kube-apiserver's `--authentication-config` names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authentication_config_file: Option<String>,
}

/// What `KubeSolo`, zeatend and zeabur-installer leave on the host for `uninstall` to remove.
/// zeatend installs `KubeSolo` without its installer, so this is all of it.
#[must_use]
pub fn footprint() -> Vec<Remove> {
    let layout = Distribution::KubeSolo.layout();
    let trees = [
        UNIT_FILE,
        layout.binary,
        CONFIG_FILE,
        CNI_CONFIG,
        AUTHENTICATION_DIR,
        layout.data_dir,
        layout.state_dir,
        KUBECONFIG,
        APPARMOR_PROFILE,
        installer::KUBESOLO_STATE_DIR,
        installer::KUBESOLO_KUBECONFIG,
        KUBESOLO_WEBHOOK_SCRIPT,
    ];
    let dirs = [CONFIG_FILE, KUBECONFIG, installer::KUBESOLO_KUBECONFIG];
    trees
        .map(|path| Remove::Tree(path.to_owned()))
        .into_iter()
        .chain(crate::registry::footprint(
            Distribution::KubeSolo,
            Role::Server,
        ))
        .chain(dirs.map(|path| Remove::EmptyDir(parent(path))))
        .collect()
}

/// The configuration file and the unit, in the order to write them.
///
/// # Errors
///
/// When the configuration cannot be serialised.
pub fn host_files(options: &Options) -> Result<Vec<HostFile>, PlanError> {
    // KubeSolo passes no --anonymous-auth, so anonymous requests stay as they were.
    let authentication =
        crate::authentication::file(AUTHENTICATION_DIR, &options.api_server, false)?;
    let api_server = ApiServer {
        extra_sans: options.tls_san.clone(),
        authentication_config_file: authentication.as_ref().map(|f| f.path.clone()),
    };
    let config = Config {
        api_version: "kubesolo.io/v1alpha1".to_owned(),
        kind: "Config".to_owned(),
        network: options.node_ip.clone().map(|node_ip| Network { node_ip }),
        kubernetes: Kubernetes {
            node_name: Some(options.node_name.clone()),
            api_server: (api_server != ApiServer::default()).then_some(api_server),
        },
    };
    Ok(authentication
        .into_iter()
        .chain([
            HostFile::yaml(CONFIG_FILE.to_owned(), HEADER, &config)?,
            HostFile {
                path: UNIT_FILE.to_owned(),
                mode: 0o644,
                content: unit(options),
            },
        ])
        .collect())
}

/// The unit of `KubeSolo`'s own installer, without its environment file, with the
/// `GOMEMLIMIT` of the tuning and the installer's webhook script when the host has it.
fn unit(options: &Options) -> String {
    let mut unit = UNIT_TEXT.replacen(
        "ExecStart=",
        &format!(
            "{}ExecStart=",
            options.tuning.environment().unwrap_or_default()
        ),
        1,
    );
    if options.installer_webhook {
        // `-`: a failed script does not stop KubeSolo.
        unit = unit.replacen(
            "KillMode=",
            &format!("ExecStartPost=-{KUBESOLO_WEBHOOK_SCRIPT}\nKillMode="),
            1,
        );
    }
    unit
}

const UNIT_TEXT: &str = "# Written by zeatend; `zeatend apply` writes it again.
[Unit]
Description=KubeSolo
Documentation=https://github.com/portainer/kubesolo
Wants=network-online.target
After=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/kubesolo --config=/etc/kubesolo/config.yaml
KillMode=process
Delegate=yes
LimitNOFILE=65535
LimitNPROC=infinity
LimitCORE=infinity
TasksMax=infinity
TimeoutStartSec=0
Restart=always
RestartSec=3
OOMScoreAdjust=-500

[Install]
WantedBy=multi-user.target
";

/// The fields zeatend reads and writes of a kubeconfig file (`kind: Config`, `apiVersion: v1`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Kubeconfig {
    /// `v1`.
    #[serde(rename = "apiVersion", default)]
    pub api_version: String,
    /// `Config`.
    #[serde(default)]
    pub kind: String,
    /// The clusters by name.
    #[serde(default)]
    pub clusters: Vec<NamedCluster>,
    /// The users by name.
    #[serde(default)]
    pub users: Vec<NamedUser>,
    /// The contexts by name.
    #[serde(default)]
    pub contexts: Vec<NamedContext>,
    /// The context `kubectl` uses.
    #[serde(default)]
    pub current_context: String,
}

/// A cluster of a [`Kubeconfig`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedCluster {
    /// Its name.
    pub name: String,
    /// Where the API server is.
    pub cluster: Cluster,
}

/// The API server and its certificate authority.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Cluster {
    /// The API server's URL.
    pub server: String,
    /// The certificate authority, base64-encoded PEM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate_authority_data: Option<String>,
}

/// A user of a [`Kubeconfig`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedUser {
    /// Its name.
    pub name: String,
    /// Its credentials.
    #[serde(default)]
    pub user: User,
}

/// A client certificate and its key, base64-encoded PEM. zeatend reads no other credentials.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct User {
    /// The client certificate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_certificate_data: Option<String>,
    /// The client key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_key_data: Option<String>,
}

/// A context of a [`Kubeconfig`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedContext {
    /// Its name.
    pub name: String,
    /// The cluster and user it names.
    pub context: Context,
}

/// The cluster and user of a context.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// The cluster's name.
    pub cluster: String,
    /// The user's name.
    pub user: String,
}

/// The admin kubeconfig as a file for the person's computer: only the certificate user, with the
/// cluster, user and context all named `name`.
///
/// # Errors
///
/// When the file has no context with a client certificate.
pub fn kubeconfig(admin: &Kubeconfig, name: &str) -> Result<Kubeconfig, PlanError> {
    let (cluster, user) = admin
        .contexts
        .iter()
        .find_map(|c| {
            let cluster = admin
                .clusters
                .iter()
                .find(|n| n.name == c.context.cluster)?;
            let user = admin.users.iter().find(|n| n.name == c.context.user)?;
            user.user.client_certificate_data.as_ref()?;
            Some((cluster, user))
        })
        .ok_or_else(|| {
            PlanError::Kubeconfig(format!(
                "{}: no context with a client certificate",
                Distribution::KubeSolo.layout().admin_kubeconfig
            ))
        })?;
    Ok(Kubeconfig {
        api_version: "v1".to_owned(),
        kind: "Config".to_owned(),
        clusters: vec![NamedCluster {
            name: name.to_owned(),
            cluster: cluster.cluster.clone(),
        }],
        users: vec![NamedUser {
            name: name.to_owned(),
            user: user.user.clone(),
        }],
        contexts: vec![NamedContext {
            name: name.to_owned(),
            context: Context {
                cluster: name.to_owned(),
                user: name.to_owned(),
            },
        }],
        current_context: name.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::tuning::MemoryTier;

    fn tuning() -> HostTuning {
        HostTuning::new(Distribution::KubeSolo, MemoryTier::Small)
    }

    #[test]
    fn the_config_names_the_address_and_certificate_names() {
        let files = host_files(&Options {
            node_name: "solo".to_owned(),
            node_ip: Some("10.0.0.5".to_owned()),
            tls_san: vec!["solo.example.com".to_owned()],
            api_server: crate::authentication::ApiServer::default(),
            tuning: tuning(),
            installer_webhook: false,
        })
        .unwrap();
        let config = files.iter().find(|f| f.path == CONFIG_FILE).unwrap();
        assert_eq!(config.mode, 0o600);
        let value: Value = serde_saphyr::from_str(&config.content).unwrap();
        assert_eq!(value["kind"], "Config");
        assert_eq!(value["network"]["nodeIP"], "10.0.0.5");
        assert_eq!(value["kubernetes"]["nodeName"], "solo");
        assert_eq!(
            value["kubernetes"]["apiServer"]["extraSANs"],
            json!(["solo.example.com"])
        );
        let bare = host_files(&Options {
            node_name: "solo".to_owned(),
            node_ip: None,
            tls_san: Vec::new(),
            api_server: crate::authentication::ApiServer::default(),
            tuning: tuning(),
            installer_webhook: false,
        })
        .unwrap();
        let value: Value = serde_saphyr::from_str(&bare[0].content).unwrap();
        assert!(value.get("network").is_none());
        assert_eq!(value["kubernetes"], json!({"nodeName": "solo"}));
        let unit = files.iter().find(|f| f.path == UNIT_FILE).unwrap();
        assert!(unit.content.contains(&format!(
            "ExecStart={} --config={CONFIG_FILE}",
            Distribution::KubeSolo.layout().binary
        )));
        assert!(
            unit.content
                .contains("[Service]\nType=simple\nEnvironment=GOMEMLIMIT=500MiB\nExecStart=")
        );
    }

    #[test]
    fn an_authentication_config_is_named_in_the_config() {
        let files = host_files(&Options {
            node_name: "solo".to_owned(),
            node_ip: None,
            tls_san: Vec::new(),
            api_server: crate::authentication::example(),
            tuning: tuning(),
            installer_webhook: false,
        })
        .unwrap();
        let authentication = format!("{AUTHENTICATION_DIR}/config.yaml");
        assert_eq!(files[0].path, authentication);
        let file: Value = serde_saphyr::from_str(&files[0].content).unwrap();
        assert!(
            file.get("anonymous").is_none(),
            "KubeSolo keeps its default"
        );
        let config = files.iter().find(|f| f.path == CONFIG_FILE).unwrap();
        let value: Value = serde_saphyr::from_str(&config.content).unwrap();
        assert_eq!(
            value["kubernetes"]["apiServer"],
            json!({"extraSANs": [], "authenticationConfigFile": authentication})
        );
    }

    #[test]
    fn the_unit_keeps_the_webhook_script_of_an_earlier_install() {
        let unit = |installer_webhook| {
            host_files(&Options {
                node_name: "solo".to_owned(),
                node_ip: None,
                tls_san: Vec::new(),
                api_server: crate::authentication::ApiServer::default(),
                tuning: tuning(),
                installer_webhook,
            })
            .unwrap()
            .into_iter()
            .find(|f| f.path == UNIT_FILE)
            .unwrap()
            .content
        };
        assert!(!unit(false).contains("ExecStartPost"));
        assert!(unit(true).contains(
            "\nExecStartPost=-/usr/local/libexec/zeabur-kubesolo-webhook\nKillMode=process\n"
        ));
    }

    #[test]
    fn the_kubeconfig_keeps_only_the_certificate_user() {
        // The shape of the admin kubeconfig KubeSolo writes.
        let admin: Kubeconfig = serde_saphyr::from_str(
            "apiVersion: v1
kind: Config
clusters:
- name: kubesolo
  cluster:
    server: https://10.0.0.5:6443
    certificate-authority-data: Q0E=
users:
- name: kubernetes-admin
  user:
    client-certificate-data: Q0VSVA==
    client-key-data: S0VZ
- name: admin-token
  user:
    tokenFile: /nowhere
contexts:
- name: admin-token@kubesolo
  context: {cluster: kubesolo, user: admin-token}
- name: kubernetes-admin@kubesolo
  context: {cluster: kubesolo, user: kubernetes-admin}
current-context: admin-token@kubesolo
",
        )
        .unwrap();
        let config = kubeconfig(&admin, "solo").unwrap();
        let yaml = serde_saphyr::to_string(&config).unwrap();
        assert!(!yaml.contains("admin-token"), "{yaml}");
        let value: Value = serde_saphyr::from_str(&yaml).unwrap();
        assert_eq!(value["current-context"], "solo");
        assert_eq!(value["contexts"][0]["context"]["user"], "solo");
        assert_eq!(value["users"][0]["user"]["client-key-data"], "S0VZ");
        assert_eq!(
            value["users"][0]["user"]["client-certificate-data"],
            "Q0VSVA=="
        );
        assert_eq!(
            value["clusters"][0]["cluster"]["server"],
            "https://10.0.0.5:6443"
        );
        assert_eq!(
            value["clusters"][0]["cluster"]["certificate-authority-data"],
            "Q0E="
        );
        assert!(kubeconfig(&Kubeconfig::default(), "solo").is_err());
    }
}
