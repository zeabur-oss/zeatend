//! `doctor`: checks this host and the cluster on it, and exits as a monitoring plugin does
//! (<https://www.monitoring-plugins.org/doc/guidelines.html#AEN78>): 0 when all is well, 1 for a
//! warning, 2 for something critical and 3 when the checks cannot run.
//!
//! The host checks are those of `apply`. The cluster checks read the service, the API server,
//! the install record, the nodes, the memory each kubelet keeps from pods and, on a K3s or RKE2
//! server, the encryption of Secrets. A worker has no admin kubeconfig, so it reads its own node
//! only, with the kubelet's credentials.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use k8s_openapi::List;
use k8s_openapi::api::core::v1;
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use serde::Deserialize;
use serde::de::IgnoredAny;
use zeatend_host::kubectl::Kubectl;
use zeatend_host::{HostError, download, facts, longhorn, preflight, systemd};
use zeatend_plan::cluster::{Distribution, Rancher};
use zeatend_plan::config::{Document, Role};
use zeatend_plan::finding::{Check, Finding, Kept, Status};
use zeatend_plan::longhorn::HostState;
use zeatend_plan::plan;
use zeatend_plan::record::{RECORD_FILE, Record, Writer, check_zeatend_version};
use zeatend_plan::registry::{self, Proxy, RegistryHost};
use zeatend_plan::tuning::OS_IDLE_MIB;
use zeatend_plan::uninstall::{self, Node};

use crate::{InstallError, i18n, join, load, output};

/// The exit code when the checks cannot run.
const UNKNOWN: u8 = 3;

/// How long the API server has to answer `/readyz`.
const API_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) async fn run(config: &Path) -> ExitCode {
    if !preflight::is_root() {
        output::line(i18n::messages().doctor_needs_root);
        return ExitCode::from(UNKNOWN);
    }
    let (require_encrypted_disk, storage, network) = match load::document_or_defaults(config) {
        Ok((Document::Install(config), _)) => (
            config.host.require_encrypted_disk,
            config.storage,
            Network {
                distribution: config.distribution,
                version: config.version,
                mirrors: config.registry_mirrors,
                hosts: config.registry_hosts,
                proxy: config.proxy,
            },
        ),
        Ok((Document::Join(config), _)) => (
            config.host.require_encrypted_disk,
            config.storage,
            Network {
                distribution: config.distribution,
                version: config.version,
                mirrors: config.registry_mirrors,
                hosts: config.registry_hosts,
                proxy: config.proxy,
            },
        ),
        Err(error) => {
            output::line(&format!("{config}: {error}", config = config.display()));
            return ExitCode::from(UNKNOWN);
        }
    };
    let root = Path::new("/");
    let installed = uninstall::detect(|path| crate::uninstall::exists(root, path))
        .first()
        .copied();
    let mut checks = match installed {
        None => vec![Finding::NotInstalled.into()],
        Some(node) => {
            let mut checks = preflight::run(preflight::Options {
                cluster_running: true,
                require_encrypted_disk,
                distribution: node.distribution,
                role: node.role,
            })
            .await;
            checks.extend(cluster(node).await.into_iter().map(Check::from));
            checks
        }
    };
    if storage.longhorn {
        checks.push(longhorn_check(&longhorn::read().await).into());
    }
    checks.extend(
        network
            .check(installed.is_none())
            .await
            .into_iter()
            .map(Check::from),
    );
    let messages = i18n::messages();
    for check in &checks {
        output::line(&messages.check_line(&check.finding));
    }
    ExitCode::from(exit_code(&checks))
}

/// Whether the node has what Longhorn needs.
fn longhorn_check(host: &HostState) -> Finding {
    let needs = zeatend_plan::longhorn::needs(host);
    if needs.is_empty() {
        Finding::LonghornReady
    } else {
        Finding::LonghornMissing { needs }
    }
}

/// What `doctor` checks the network with, from the configuration.
struct Network {
    distribution: Distribution,
    /// The configured K3s or RKE2 release; `None` for the pinned one.
    version: Option<String>,
    mirrors: BTreeMap<String, Vec<String>>,
    hosts: BTreeMap<String, RegistryHost>,
    proxy: Option<Proxy>,
}

impl Network {
    /// Whether each registry mirror answers and, before an install, each release download.
    /// Any HTTP answer from a mirror passes: a mirror may ask for credentials at `/v2/`.
    async fn check(&self, downloads: bool) -> Vec<Finding> {
        let mut findings = Vec::new();
        if downloads {
            let agent = download::agent(self.proxy.as_ref(), &[]);
            let arch = facts::arch(std::env::consts::ARCH);
            for url in plan::release_urls(self.distribution, self.version.as_deref(), arch) {
                findings.push(match agent.status(&url).await {
                    Ok(status) if status < 400 => Finding::DownloadsOk { url },
                    Ok(status) => Finding::DownloadsNo {
                        url,
                        reason: format!("HTTP {status}"),
                    },
                    Err(error) => Finding::DownloadsNo {
                        url,
                        reason: reason(&error),
                    },
                });
            }
        }
        let direct = facts::addresses().await;
        for endpoint in self.mirrors.values().flatten() {
            let Some((url, host)) = registry::probe(endpoint) else {
                continue;
            };
            let ca = self.hosts.get(host).and_then(|h| h.ca_file.as_deref());
            let agent = match ca.map(fs::read) {
                None => download::agent(self.proxy.as_ref(), &direct),
                Some(Ok(pem)) => download::agent_trusting(self.proxy.as_ref(), &direct, &pem),
                Some(Err(error)) => {
                    findings.push(Finding::MirrorNo {
                        endpoint: endpoint.clone(),
                        reason: format!("{}: {error}", ca.unwrap_or_default()),
                    });
                    continue;
                }
            };
            findings.push(match agent.status(&url).await {
                Ok(_) => Finding::MirrorOk {
                    endpoint: endpoint.clone(),
                },
                Err(error) => Finding::MirrorNo {
                    endpoint: endpoint.clone(),
                    reason: reason(&error),
                },
            });
        }
        findings
    }
}

/// The cause of a failed download, without the URL the finding names already.
fn reason(error: &HostError) -> String {
    if let HostError::Download { reason, .. } = error {
        reason.clone()
    } else {
        error.to_string()
    }
}

/// The exit code of the worst check.
fn exit_code(checks: &[Check]) -> u8 {
    checks
        .iter()
        .map(|c| match c.status {
            Status::Pass => 0,
            Status::Warn => 1,
            Status::Fail => 2,
        })
        .max()
        .unwrap_or(0)
}

/// The checks of the cluster from this node. They stop at the first that leaves nothing more to
/// read: the service or the API server down.
async fn cluster(node: Node) -> Vec<Finding> {
    let layout = node.distribution.layout();
    let unit = layout.unit(node.role);
    let kubectl = match (node.distribution.rancher(), node.role) {
        (Some(_), Role::Server) => Kubectl::admin(node.distribution),
        (Some(rancher), Role::Worker) => join::own_kubectl(rancher),
        (None, _) => crate::kubesolo::kubectl(),
    };
    if !systemd::is_active(unit).await {
        return vec![Finding::ServiceStopped { unit }];
    }
    let mut found = vec![Finding::ServiceRunning { unit }];
    let admin = node.role == Role::Server;
    if admin {
        if kubectl.wait_ready(API_TIMEOUT).await.is_err() {
            found.push(Finding::ApiDown { unit });
            return found;
        }
        found.push(Finding::ApiReady);
    }
    found.push(record(node.distribution, layout.state_dir).await);
    match read_nodes(&kubectl, admin).await {
        Ok(nodes) if nodes.is_empty() => found.push(Finding::NodesNone { unit }),
        Ok(nodes) => {
            let kubectl = kubectl_command(node.distribution);
            found.push(readiness(&nodes, |name| {
                if admin {
                    format!("sudo {kubectl} describe node {name}")
                } else {
                    format!("sudo journalctl -u {unit} -n 100")
                }
            }));
            found.extend(reserves(&nodes, node.distribution));
        }
        Err(error) => found.push(Finding::NodesUnknown {
            reason: error.to_string(),
        }),
    }
    if let (Some(rancher), Role::Server) = (node.distribution.rancher(), node.role) {
        let path = format!("{}/server/cred/encryption-config.json", layout.data_dir);
        let config = fs::read_to_string(path).ok();
        found.push(secrets_encryption(rancher, config.as_deref()));
    }
    found
}

/// The install record that `apply` or `join` wrote in `state_dir`, else the one of the
/// zeabur-installer install that zeatend takes over.
async fn record(distribution: Distribution, state_dir: &str) -> Finding {
    let path = Path::new(state_dir).join(RECORD_FILE).display().to_string();
    match crate::installer::read_record(distribution).await {
        Ok(Some(Record {
            writer: Writer::Zeatend(version),
            ..
        })) if check_zeatend_version(&version).is_err() => Finding::RecordNewer {
            zeatend_version: version,
        },
        Ok(Some(record)) => Finding::RecordOk {
            writer: record.writer.to_string(),
        },
        Ok(None) | Err(_) => Finding::RecordMissing { path },
    }
}

/// Every node with an admin kubeconfig; else this node, read with the kubelet's credentials.
async fn read_nodes(kubectl: &Kubectl, admin: bool) -> Result<Vec<v1::Node>, InstallError> {
    if admin {
        let list: List<v1::Node> = kubectl.raw("/api/v1/nodes").await?;
        return Ok(list.items);
    }
    let name = facts::installed_node_name(Path::new("/")).ok_or(InstallError::NoKubeletNode)?;
    let node = kubectl.get("node", &name, None).await?;
    Ok(vec![node.ok_or(InstallError::NoSuchNode(name))?])
}

/// The command line of the admin `kubectl` of `distribution`.
fn kubectl_command(distribution: Distribution) -> String {
    let layout = distribution.layout();
    format!(
        "{} --kubeconfig {}",
        layout.kubectl, layout.admin_kubeconfig
    )
}

fn name(node: &v1::Node) -> String {
    node.metadata.name.clone().unwrap_or_default()
}

/// Whether the nodes are Ready. `command` gives the command that shows why a node is not.
fn readiness(nodes: &[v1::Node], command: impl Fn(&str) -> String) -> Finding {
    let (ready, not_ready): (Vec<&v1::Node>, Vec<&v1::Node>) =
        nodes.iter().partition(|n| join::ready_since(n, None));
    let names = |nodes: Vec<&v1::Node>| nodes.into_iter().map(name).collect::<Vec<_>>();
    if not_ready.is_empty() {
        return Finding::NodesReady {
            names: names(ready),
        };
    }
    let names = names(not_ready);
    Finding::NodesNotReady {
        command: command(names.first().map_or("", String::as_str)),
        names,
    }
}

/// The memory each kubelet keeps from pods: capacity less allocatable. Too little leaves pods able
/// to take the memory the OS needs.
fn reserves(nodes: &[v1::Node], distribution: Distribution) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut low = Vec::new();
    for node in nodes {
        let memory = |field: Option<&BTreeMap<String, Quantity>>| {
            field?.get("memory").and_then(|q| bytes(&q.0))
        };
        let status = node.status.as_ref();
        match (
            memory(status.and_then(|s| s.capacity.as_ref())),
            memory(status.and_then(|s| s.allocatable.as_ref())),
        ) {
            (Some(capacity), Some(allocatable)) => {
                let mib = capacity.saturating_sub(allocatable) >> 20;
                if mib < OS_IDLE_MIB {
                    low.push(Kept {
                        name: name(node),
                        mib,
                    });
                }
            }
            _ => found.push(Finding::ReservesUnknown { name: name(node) }),
        }
    }
    if !low.is_empty() {
        found.push(Finding::ReservesLow {
            nodes: low,
            needed_mib: OS_IDLE_MIB,
            distribution,
        });
    } else if found.is_empty() && !nodes.is_empty() {
        found.push(Finding::ReservesSet {
            names: nodes.iter().map(name).collect(),
        });
    }
    found
}

/// A Kubernetes memory quantity, such as `3461052Ki` or `2Gi`, in bytes; `None` for a form a
/// node does not report, such as a fraction or an exponent.
fn bytes(quantity: &str) -> Option<u64> {
    let split = quantity
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(quantity.len());
    let (number, suffix) = quantity.split_at(split);
    let shift: u32 = match suffix {
        "" => 0,
        "Ki" => 10,
        "Mi" => 20,
        "Gi" => 30,
        "Ti" => 40,
        "k" => return number.parse::<u64>().ok()?.checked_mul(1000),
        "M" => return number.parse::<u64>().ok()?.checked_mul(1_000_000),
        "G" => return number.parse::<u64>().ok()?.checked_mul(1_000_000_000),
        _ => return None,
    };
    number.parse::<u64>().ok()?.checked_mul(1 << shift)
}

/// Whether kube-apiserver writes Secrets encrypted, from the first `secrets` provider of the
/// encryption configuration.
fn secrets_encryption(distribution: Rancher, config: Option<&str>) -> Finding {
    let config: EncryptionConfiguration = config
        .and_then(|c| serde_json::from_str(c).ok())
        .unwrap_or_default();
    let writer = config
        .resources
        .iter()
        .find(|r| r.resources.iter().any(|n| n == "secrets"))
        .and_then(|r| r.providers.first()?.keys().next().cloned());
    match writer {
        Some(provider) if provider != "identity" => Finding::SecretsEncrypted { provider },
        _ => Finding::SecretsPlain { distribution },
    }
}

/// The fields zeatend reads of kube-apiserver's `EncryptionConfiguration`
/// (`apiserver.config.k8s.io/v1`).
#[derive(Debug, Default, Deserialize)]
struct EncryptionConfiguration {
    #[serde(default)]
    resources: Vec<ResourceConfiguration>,
}

/// Resources and the providers that write and read them, in order. Each provider has one key,
/// its name.
#[derive(Debug, Deserialize)]
struct ResourceConfiguration {
    #[serde(default)]
    resources: Vec<String>,
    #[serde(default)]
    #[expect(
        clippy::zero_sized_map_values,
        reason = "a provider is a map with one key, its name, which is all that is read"
    )]
    providers: Vec<BTreeMap<String, IgnoredAny>>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_worst_check_sets_the_exit_code() {
        let checks = |findings: Vec<Finding>| -> Vec<Check> {
            findings.into_iter().map(Check::from).collect()
        };
        assert_eq!(exit_code(&[]), 0);
        assert_eq!(exit_code(&checks(vec![Finding::ApiReady])), 0);
        let warn = Finding::RecordMissing {
            path: String::new(),
        };
        assert_eq!(exit_code(&checks(vec![Finding::ApiReady, warn.clone()])), 1);
        let fail = Finding::ServiceStopped { unit: "k3s" };
        assert_eq!(exit_code(&checks(vec![fail, warn, Finding::ApiReady])), 2);
    }

    fn node(name: &str, ready: &str, capacity: &str, allocatable: &str) -> v1::Node {
        typed(json!({"metadata": {"name": name}, "status": {
            "conditions": [{"type": "Ready", "status": ready}],
            "capacity": {"memory": capacity},
            "allocatable": {"memory": allocatable},
        }}))
    }

    fn typed(value: serde_json::Value) -> v1::Node {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn nodes_that_are_not_ready_are_named_with_the_command_for_the_first() {
        let nodes = [
            node("a", "True", "", ""),
            node("b", "Unknown", "", ""),
            typed(json!({"metadata": {"name": "c"}})),
        ];
        let describe = |name: &str| format!("sudo kubectl describe node {name}");
        assert_eq!(
            readiness(&nodes, describe),
            Finding::NodesNotReady {
                names: vec!["b".to_owned(), "c".to_owned()],
                command: "sudo kubectl describe node b".to_owned(),
            }
        );
        assert_eq!(
            readiness(&nodes[..1], describe),
            Finding::NodesReady {
                names: vec!["a".to_owned()],
            }
        );
    }

    #[test]
    fn a_kubelet_that_keeps_less_than_the_os_needs_is_named() {
        // What K3s on a 2 GiB host keeps with zeatend's reserves: 350 + 100 + 200 MiB.
        let k3s = node("k3s", "True", "2000000Ki", "1334400Ki");
        // KubeSolo without reserves: the kubelet's default hard threshold of 100 MiB, as read
        // from a KubeSolo node.
        let kubesolo = node("solo", "True", "3461052Ki", "3358652Ki");
        let bare = node("bare", "True", "4Gi", "4Gi");
        assert_eq!(
            reserves(&[k3s.clone(), kubesolo, bare], Distribution::KubeSolo),
            [Finding::ReservesLow {
                nodes: vec![
                    Kept {
                        name: "solo".to_owned(),
                        mib: 100
                    },
                    Kept {
                        name: "bare".to_owned(),
                        mib: 0
                    },
                ],
                needed_mib: OS_IDLE_MIB,
                distribution: Distribution::KubeSolo,
            }]
        );
        assert_eq!(
            reserves(std::slice::from_ref(&k3s), Distribution::K3s),
            [Finding::ReservesSet {
                names: vec!["k3s".to_owned()]
            }]
        );
        assert_eq!(
            reserves(
                &[typed(json!({"metadata": {"name": "x"}}))],
                Distribution::K3s
            ),
            [Finding::ReservesUnknown {
                name: "x".to_owned()
            }]
        );
        assert_eq!(reserves(&[], Distribution::K3s), []);
    }

    #[test]
    fn the_node_list_of_the_api_reads_as_nodes() {
        let node: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/node.json")).unwrap();
        let list: List<v1::Node> = serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "NodeList",
            "metadata": {"resourceVersion": "812"},
            "items": [node],
        }))
        .unwrap();
        let describe = |name: &str| format!("sudo kubectl describe node {name}");
        assert_eq!(
            readiness(&list.items, describe),
            Finding::NodesReady {
                names: vec!["k3s-1".to_owned()]
            }
        );
        assert_eq!(
            reserves(&list.items, Distribution::K3s),
            [Finding::ReservesSet {
                names: vec!["k3s-1".to_owned()]
            }]
        );
    }

    #[test]
    fn memory_quantities_read_as_bytes() {
        assert_eq!(bytes("3461052Ki"), Some(3_461_052 * 1024));
        assert_eq!(bytes("2Gi"), Some(2 << 30));
        assert_eq!(bytes("1000"), Some(1000));
        assert_eq!(bytes("2G"), Some(2_000_000_000));
        assert_eq!(bytes("1.5Gi"), None);
        assert_eq!(bytes(""), None);
    }

    #[test]
    fn secrets_are_encrypted_only_when_the_first_provider_is_not_identity() {
        let config = |first: &str| {
            json!({
                "kind": "EncryptionConfiguration",
                "apiVersion": "apiserver.config.k8s.io/v1",
                "resources": [{
                    "resources": ["secrets"],
                    "providers": [
                        {(first): {"keys": [{"name": "aescbckey", "secret": "x"}]}},
                        {"identity": {}},
                    ],
                }],
            })
            .to_string()
        };
        let rke2 = config("aescbc");
        assert_eq!(
            secrets_encryption(Rancher::Rke2, Some(&rke2)),
            Finding::SecretsEncrypted {
                provider: "aescbc".to_owned()
            }
        );
        let plain = Finding::SecretsPlain {
            distribution: Rancher::K3s,
        };
        let identity = config("identity");
        assert_eq!(secrets_encryption(Rancher::K3s, Some(&identity)), plain);
        assert_eq!(secrets_encryption(Rancher::K3s, Some("{}")), plain);
        assert_eq!(secrets_encryption(Rancher::K3s, None), plain);
    }
}
