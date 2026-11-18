//! `KubeSolo`: its files, ports and kubeconfig.
//!
//! `KubeSolo` is one process with `SQLite`, a bridge CNI and no scheduler. It enforces no
//! `NetworkPolicy`, so it isolates nothing between namespaces. It reads `/etc/kubesolo/config.yaml`
//! (`kubesolo --print-config` shows every key) and keeps its state in `/var/lib/kubesolo`.

use serde_json::{Value, json};

use crate::PlanError;
use crate::cluster::{HostFile, PortUse};
use crate::tuning::HostTuning;

/// The systemd unit.
pub const UNIT: &str = "kubesolo";
/// The binary from the release archive.
pub const BINARY: &str = "/usr/local/bin/kubesolo";
/// Its state: the database, certificates and images.
pub const DATA_DIR: &str = "/var/lib/kubesolo";
/// Its configuration file.
pub const CONFIG_FILE: &str = "/etc/kubesolo/config.yaml";
/// The unit file.
pub const UNIT_FILE: &str = "/etc/systemd/system/kubesolo.service";
/// The kubelet's certificate, whose subject names the node.
pub const KUBELET_CERT: &str = "/var/lib/kubesolo/pki/kubelet/kubelet.crt";
/// The cluster-admin kubeconfig `KubeSolo` writes.
pub const ADMIN_KUBECONFIG: &str = "/var/lib/kubesolo/pki/admin/admin.kubeconfig";
/// zeatend's downloads and the kubectl it uses.
pub const STATE_DIR: &str = "/var/lib/zeatend";
/// The pinned kubectl.
pub const KUBECTL: &str = "/var/lib/zeatend/bin/kubectl";
/// The kubeconfig zeatend writes for the person's own computer.
pub const KUBECONFIG: &str = "/etc/zeatend/kubeconfig.yaml";

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
        (80, PortUse::KubeSoloHttp),
        (443, PortUse::KubeSoloHttps),
        (6443, PortUse::KubernetesApi),
        (10250, PortUse::Kubelet),
        (10443, PortUse::KubeSoloWebhook),
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
    /// The host's memory settings; the unit sets their `GOMEMLIMIT`.
    pub tuning: HostTuning,
}

const HEADER: &str = "# Written by zeatend; `zeatend apply` writes it again.\n";

/// The configuration file and the unit, in the order to write them.
#[must_use]
pub fn host_files(options: &Options) -> Vec<HostFile> {
    let mut lines = vec![
        HEADER.trim_end().to_owned(),
        "apiVersion: kubesolo.io/v1alpha1".to_owned(),
        "kind: Config".to_owned(),
    ];
    if let Some(ip) = &options.node_ip {
        lines.extend(["network:".to_owned(), format!("  nodeIP: {}", json!(ip))]);
    }
    lines.extend([
        "kubernetes:".to_owned(),
        format!("  nodeName: {}", json!(options.node_name)),
    ]);
    if !options.tls_san.is_empty() {
        lines.extend([
            "  apiServer:".to_owned(),
            format!("    extraSANs: {}", json!(options.tls_san)),
        ]);
    }
    let config = lines.join("\n") + "\n";
    vec![
        HostFile {
            path: CONFIG_FILE.to_owned(),
            mode: 0o600,
            content: config,
        },
        HostFile {
            path: UNIT_FILE.to_owned(),
            mode: 0o644,
            content: unit(&options.tuning),
        },
    ]
}

/// The unit of `KubeSolo`'s own installer, without its environment file, and with the
/// `GOMEMLIMIT` of `tuning`.
fn unit(tuning: &HostTuning) -> String {
    UNIT_TEXT.replacen(
        "ExecStart=",
        &format!("{}ExecStart=", tuning.environment().unwrap_or_default()),
        1,
    )
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

/// The admin kubeconfig as a file for the person's computer: only the certificate user, with the
/// cluster, user and context all named `name`.
///
/// # Errors
///
/// When the file has no context with a client certificate.
pub fn kubeconfig(admin: &Value, name: &str) -> Result<Value, PlanError> {
    let bad = |reason: &str| PlanError::Kubeconfig(format!("{ADMIN_KUBECONFIG}: {reason}"));
    let named = |list: &str, key: &str| -> Option<Value> {
        admin
            .get(list)?
            .as_array()?
            .iter()
            .find(|e| e.get("name").and_then(Value::as_str) == Some(key))
            .cloned()
    };
    let contexts = admin
        .get("contexts")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("no contexts"))?;
    let (cluster, user) = contexts
        .iter()
        .find_map(|c| {
            let ctx = c.get("context")?;
            let cluster = named("clusters", ctx.get("cluster")?.as_str()?)?;
            let user = named("users", ctx.get("user")?.as_str()?)?;
            user.pointer("/user/client-certificate-data")?;
            Some((cluster, user))
        })
        .ok_or_else(|| bad("no context with a client certificate"))?;
    let cluster = cluster.get("cluster").cloned().unwrap_or_default();
    Ok(json!({
        "apiVersion": "v1",
        "kind": "Config",
        "clusters": [{"name": name, "cluster": cluster}],
        "users": [{"name": name, "user": user.get("user").cloned().unwrap_or_default()}],
        "contexts": [{"name": name, "context": {"cluster": name, "user": name}}],
        "current-context": name,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::Distribution;
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
            tuning: tuning(),
        });
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
            tuning: tuning(),
        });
        let value: Value = serde_saphyr::from_str(&bare[0].content).unwrap();
        assert!(value.get("network").is_none());
        assert_eq!(value["kubernetes"], json!({"nodeName": "solo"}));
        let unit = files.iter().find(|f| f.path == UNIT_FILE).unwrap();
        assert!(
            unit.content
                .contains(&format!("ExecStart={BINARY} --config={CONFIG_FILE}"))
        );
        assert!(
            unit.content
                .contains("[Service]\nType=simple\nEnvironment=GOMEMLIMIT=500MiB\nExecStart=")
        );
    }

    #[test]
    fn the_kubeconfig_keeps_only_the_certificate_user() {
        let admin = json!({
            "clusters": [{"name": "kubesolo", "cluster": {"server": "https://10.0.0.5:6443", "certificate-authority-data": "Q0E="}}],
            "users": [
                {"name": "kubernetes-admin", "user": {"client-certificate-data": "Q0VSVA==", "client-key-data": "S0VZ"}},
                {"name": "admin-token", "user": {"tokenFile": "/nowhere"}},
            ],
            "contexts": [
                {"name": "admin-token@kubesolo", "context": {"cluster": "kubesolo", "user": "admin-token"}},
                {"name": "kubernetes-admin@kubesolo", "context": {"cluster": "kubesolo", "user": "kubernetes-admin"}},
            ],
            "current-context": "admin-token@kubesolo",
        });
        let config = kubeconfig(&admin, "solo").unwrap();
        assert_eq!(config["current-context"], "solo");
        assert!(!config.to_string().contains("admin-token"));
        assert_eq!(config["users"][0]["user"]["client-key-data"], "S0VZ");
        assert_eq!(
            config["clusters"][0]["cluster"]["server"],
            "https://10.0.0.5:6443"
        );
        assert_eq!(
            config["clusters"][0]["cluster"]["certificate-authority-data"],
            "Q0E="
        );
        assert!(kubeconfig(&json!({"contexts": []}), "solo").is_err());
    }
}
