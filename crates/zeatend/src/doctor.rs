//! `doctor`: checks this host and the cluster on it, and exits as a monitoring plugin does
//! (<https://www.monitoring-plugins.org/doc/guidelines.html#AEN78>): 0 when all is well, 1 for a
//! warning, 2 for something critical and 3 when the checks cannot run.
//!
//! The host checks are those of `apply`. The cluster checks read the service, the API server,
//! the install record, the nodes, the memory each kubelet keeps from pods and, on a K3s or RKE2
//! server, the encryption of Secrets. A worker has no admin kubeconfig, so it reads its own node
//! only, with the kubelet's credentials.

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::Value;
use zeatend_host::kubectl::Kubectl;
use zeatend_host::preflight::{self, Check, Finding, Kept, Status};
use zeatend_host::{facts, systemd};
use zeatend_plan::cluster::{Distribution, Rancher};
use zeatend_plan::config::{Document, Role};
use zeatend_plan::kubesolo;
use zeatend_plan::record::RECORD_FILE;
use zeatend_plan::tuning::OS_IDLE_MIB;

use crate::uninstall::{self, Node};
use crate::{apply, i18n, join, load, output};

/// The exit code when the checks cannot run.
const UNKNOWN: u8 = 3;

/// How long the API server has to answer `/readyz`.
const API_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) async fn run(config: &Path) -> ExitCode {
    if !preflight::is_root() {
        output::line(i18n::messages().doctor_needs_root);
        return ExitCode::from(UNKNOWN);
    }
    let require_encrypted_disk = match load::document_or_defaults(config) {
        Ok((Document::Install(config), _)) => config.host.require_encrypted_disk,
        Ok((Document::Join(config), _)) => config.host.require_encrypted_disk,
        Err(error) => {
            output::line(&format!("{config}: {error}", config = config.display()));
            return ExitCode::from(UNKNOWN);
        }
    };
    let root = Path::new("/");
    let checks = match uninstall::detect(|path| uninstall::exists(root, path))
        .first()
        .copied()
    {
        None => vec![Finding::NotInstalled.into()],
        Some(node) => {
            let mut checks = preflight::run(preflight::Options {
                cluster_running: true,
                require_encrypted_disk,
                node: host_node(node),
            })
            .await;
            checks.extend(cluster(node).await.into_iter().map(Check::from));
            checks
        }
    };
    let messages = i18n::messages();
    for check in &checks {
        output::line(&messages.check_line(&check.finding));
    }
    ExitCode::from(exit_code(&checks))
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

/// The node, as the host checks know it.
const fn host_node(node: Node) -> preflight::Node {
    match (node.distribution.rancher(), node.role) {
        (Some(rancher), Role::Server) => preflight::Node::Server(rancher),
        (Some(rancher), Role::Worker) => preflight::Node::Agent(rancher),
        (None, _) => preflight::Node::KubeSolo,
    }
}

/// The checks of the cluster from this node. They stop at the first that leaves nothing more to
/// read: the service or the API server down.
async fn cluster(node: Node) -> Vec<Finding> {
    let (unit, kubectl, state_dir) = match (node.distribution.rancher(), node.role) {
        (Some(rancher), Role::Server) => (
            rancher.server_unit(),
            Kubectl::admin(rancher),
            apply::STATE_DIR,
        ),
        (Some(rancher), Role::Worker) => (
            rancher.agent_unit(),
            join::own_kubectl(rancher),
            apply::STATE_DIR,
        ),
        (None, _) => (
            kubesolo::UNIT,
            crate::kubesolo::kubectl(),
            kubesolo::STATE_DIR,
        ),
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
    found.push(record(Path::new(state_dir)));
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
        Err(reason) => found.push(Finding::NodesUnknown { reason }),
    }
    if let (Some(rancher), Role::Server) = (node.distribution.rancher(), node.role) {
        let path = format!(
            "/var/lib/rancher/{}/server/cred/encryption-config.json",
            rancher.id()
        );
        let config = std::fs::read_to_string(path).ok();
        found.push(secrets_encryption(rancher, config.as_deref()));
    }
    found
}

/// The install record that `apply` or `join` wrote in `state_dir`.
fn record(state_dir: &Path) -> Finding {
    let path = state_dir.join(RECORD_FILE).display().to_string();
    match apply::read_record(state_dir) {
        Ok(Some(record)) if record.check_zeatend_version().is_err() => Finding::RecordNewer {
            zeatend_version: record.zeatend_version,
        },
        Ok(Some(record)) => Finding::RecordOk {
            zeatend_version: record.zeatend_version,
        },
        Ok(None) | Err(_) => Finding::RecordMissing { path },
    }
}

/// Every node with an admin kubeconfig; else this node, read with the kubelet's credentials.
async fn read_nodes(kubectl: &Kubectl, admin: bool) -> Result<Vec<Value>, String> {
    if admin {
        let list = kubectl
            .raw("/api/v1/nodes")
            .await
            .map_err(|e| e.to_string())?;
        return Ok(list
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default());
    }
    let name = facts::installed_node_name(Path::new("/"))
        .ok_or_else(|| "the kubelet's certificate names no node".to_owned())?;
    match kubectl.get("node", &name, None).await {
        Ok(Some(node)) => Ok(vec![node]),
        Ok(None) => Err(format!("the cluster has no node {name}")),
        Err(e) => Err(e.to_string()),
    }
}

/// The command line of the admin `kubectl` of `distribution`.
fn kubectl_command(distribution: Distribution) -> String {
    distribution.rancher().map_or_else(
        || {
            format!(
                "{} --kubeconfig {}",
                kubesolo::KUBECTL,
                kubesolo::ADMIN_KUBECONFIG
            )
        },
        |rancher| {
            let layout = rancher.layout();
            format!(
                "{} --kubeconfig {}",
                layout.kubectl, layout.admin_kubeconfig
            )
        },
    )
}

fn name(node: &Value) -> String {
    node.pointer("/metadata/name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Whether the nodes are Ready. `command` gives the command that shows why a node is not.
fn readiness(nodes: &[Value], command: impl Fn(&str) -> String) -> Finding {
    let (ready, not_ready): (Vec<&Value>, Vec<&Value>) =
        nodes.iter().partition(|n| join::ready_since(n, None));
    let names = |nodes: Vec<&Value>| nodes.into_iter().map(name).collect::<Vec<_>>();
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

/// The memory each kubelet keeps from pods: the node's capacity less its allocatable memory,
/// which is `system-reserved`, `kube-reserved` and the hard eviction threshold together
/// (<https://kubernetes.io/docs/tasks/administer-cluster/reserve-compute-resources/#node-allocatable>).
/// A kubelet that keeps less than the OS needs at idle has no reserves, at most its default
/// threshold of 100 MiB, and pods may take the memory the OS and the kubelet need.
fn reserves(nodes: &[Value], distribution: Distribution) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut low = Vec::new();
    for node in nodes {
        let memory = |field: &str| {
            node.pointer(&format!("/status/{field}/memory"))
                .and_then(Value::as_str)
                .and_then(bytes)
        };
        match (memory("capacity"), memory("allocatable")) {
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

/// Whether kube-apiserver writes Secrets encrypted, from the encryption configuration K3s or RKE2
/// writes: the first provider for `secrets` is the one it writes with, and `identity` writes
/// them as they are
/// (<https://kubernetes.io/docs/tasks/administer-cluster/encrypt-data/#understanding-the-encryption-at-rest-configuration>).
fn secrets_encryption(distribution: Rancher, config: Option<&str>) -> Finding {
    let config: Value = config
        .and_then(|c| serde_json::from_str(c).ok())
        .unwrap_or_default();
    let writer = config
        .get("resources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|r| {
            r.get("resources")
                .and_then(Value::as_array)
                .is_some_and(|names| names.iter().any(|n| n == "secrets"))
        })
        .and_then(|r| {
            r.get("providers")?
                .get(0)?
                .as_object()?
                .keys()
                .next()
                .cloned()
        });
    match writer {
        Some(provider) if provider != "identity" => Finding::SecretsEncrypted { provider },
        _ => Finding::SecretsPlain { distribution },
    }
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

    fn node(name: &str, ready: &str, capacity: &str, allocatable: &str) -> Value {
        json!({"metadata": {"name": name}, "status": {
            "conditions": [{"type": "Ready", "status": ready}],
            "capacity": {"memory": capacity},
            "allocatable": {"memory": allocatable},
        }})
    }

    #[test]
    fn nodes_that_are_not_ready_are_named_with_the_command_for_the_first() {
        let nodes = [
            node("a", "True", "", ""),
            node("b", "Unknown", "", ""),
            json!({"metadata": {"name": "c"}}),
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
            reserves(&[json!({"metadata": {"name": "x"}})], Distribution::K3s),
            [Finding::ReservesUnknown {
                name: "x".to_owned()
            }]
        );
        assert_eq!(reserves(&[], Distribution::K3s), []);
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
        let rke2 = r#"{"kind":"EncryptionConfiguration","apiVersion":"apiserver.config.k8s.io/v1",
            "resources":[{"resources":["secrets"],
            "providers":[{"aescbc":{"keys":[{"name":"aescbckey","secret":"x"}]}},{"identity":{}}]}]}"#;
        assert_eq!(
            secrets_encryption(Rancher::Rke2, Some(rke2)),
            Finding::SecretsEncrypted {
                provider: "aescbc".to_owned()
            }
        );
        let plain = Finding::SecretsPlain {
            distribution: Rancher::K3s,
        };
        let identity = rke2.replacen("aescbc", "identity", 1);
        assert_eq!(secrets_encryption(Rancher::K3s, Some(&identity)), plain);
        assert_eq!(secrets_encryption(Rancher::K3s, Some("{}")), plain);
        assert_eq!(secrets_encryption(Rancher::K3s, None), plain);
    }

    #[test]
    fn host_checks_run_for_the_installed_node() {
        let node = |distribution, role| Node::new(distribution, role, false);
        assert_eq!(
            host_node(node(Distribution::K3s, Role::Server)),
            preflight::Node::Server(Rancher::K3s)
        );
        assert_eq!(
            host_node(node(Distribution::Rke2, Role::Worker)),
            preflight::Node::Agent(Rancher::Rke2)
        );
        assert_eq!(
            host_node(node(Distribution::KubeSolo, Role::Server)),
            preflight::Node::KubeSolo
        );
    }
}
