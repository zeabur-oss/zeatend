//! `KubeSolo`'s pod network, read from the CNI configuration it writes.
//!
//! `KubeSolo` writes [`kubesolo::CNI_CONFIG`] with a bridge plugin and host-local addresses. The
//! plugins leave a network namespace per pod, the bridge, the addresses they gave
//! ([`NETWORKS_DIR`]) and libcni's cached results ([`RESULTS_DIR`]). K3s's flannel also names its
//! bridge `cni0` in `10.42.0.0/16`, so `uninstall` takes the names from this file only, and reads
//! it before the footprint removes it.

use std::path::Path;

use serde::Deserialize;

use crate::kubesolo;

/// Where host-local keeps the addresses it gave, one directory per network.
pub const NETWORKS_DIR: &str = "/var/lib/cni/networks";
/// Where libcni keeps the result of each attachment, as `<network>-<container>-<interface>`.
pub const RESULTS_DIR: &str = "/var/lib/cni/results";
/// The network of containerd's loopback attachment, which libcni keeps as
/// `cni-loopback-<container>-lo` beside the pod network's own result.
pub const LOOPBACK_NETWORK: &str = "cni-loopback";
/// The kubelet's directory, which K3s and RKE2 use too.
pub const KUBELET_DIR: &str = "/var/lib/kubelet";
/// The bridge plugin's bridge when its configuration names none.
const DEFAULT_BRIDGE: &str = "cni0";
/// Where `ip netns` keeps the namespaces it names.
const NETNS_DIRS: [&str; 2] = ["/run/netns", "/var/run/netns"];

/// The pod network that a CNI configuration list names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodNetwork {
    /// The network's name, such as `kubesolo-net`.
    pub name: String,
    /// The bridge of its bridge plugin, if it has one.
    pub bridge: Option<String>,
}

#[derive(Deserialize)]
struct Conflist {
    name: String,
    #[serde(default)]
    plugins: Vec<Plugin>,
}

#[derive(Deserialize)]
struct Plugin {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    bridge: Option<String>,
}

/// The parts of a libcni cached result that name the pod's sandbox and network namespace.
#[derive(Deserialize)]
struct CachedResult {
    #[serde(rename = "containerId", default)]
    container_id: String,
    #[serde(rename = "networkName", default)]
    network_name: String,
    #[serde(default)]
    netns: Option<String>,
    #[serde(default)]
    result: Option<Interfaces>,
}

#[derive(Deserialize)]
struct Interfaces {
    #[serde(default)]
    interfaces: Vec<Interface>,
}

#[derive(Deserialize)]
struct Interface {
    #[serde(default)]
    sandbox: Option<String>,
}

impl PodNetwork {
    /// Reads a CNI configuration list. `None` when it is not one, or names a network or bridge
    /// that is not a plain file or interface name.
    #[must_use]
    pub fn parse(conflist: &str) -> Option<Self> {
        let list: Conflist = serde_json::from_str(conflist).ok()?;
        let bridge = list
            .plugins
            .into_iter()
            .find(|p| p.kind == "bridge")
            .map(|p| p.bridge.unwrap_or_else(|| DEFAULT_BRIDGE.to_owned()));
        (plain_name(&list.name) && bridge.as_deref().is_none_or(plain_name)).then_some(Self {
            name: list.name,
            bridge,
        })
    }

    /// The directory of the addresses host-local gave on this network.
    #[must_use]
    pub fn addresses_dir(&self) -> String {
        format!("{NETWORKS_DIR}/{}", self.name)
    }

    /// Whether a file in [`RESULTS_DIR`] is a result of this network.
    #[must_use]
    pub fn owns_result(&self, file_name: &str) -> bool {
        file_name
            .strip_prefix(&self.name)
            .is_some_and(|rest| rest.starts_with('-'))
    }

    /// The pod sandbox that a cached result of this network records, and the name, under
    /// `/run/netns`, of its network namespace. `None` for another network or a sandbox that is not
    /// a plain name; no namespace when `ip netns` does not name it.
    #[must_use]
    pub fn result_attachment(&self, result: &str) -> Option<Attachment> {
        let cached: CachedResult = serde_json::from_str(result).ok()?;
        if cached.network_name != self.name || !plain_name(&cached.container_id) {
            return None;
        }
        let netns = cached
            .netns
            .into_iter()
            .chain(
                cached
                    .result
                    .into_iter()
                    .flat_map(|r| r.interfaces)
                    .filter_map(|i| i.sandbox),
            )
            .find_map(|path| netns_name(&path));
        Some(Attachment {
            container: cached.container_id,
            netns,
        })
    }
}

/// A pod sandbox's attachment to the pod network, from libcni's cached result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// The sandbox's container ID.
    pub container: String,
    /// The name of its network namespace under `/run/netns`.
    pub netns: Option<String>,
}

/// The name of a namespace that `ip netns` keeps at `path`.
fn netns_name(path: &str) -> Option<String> {
    let path = Path::new(path);
    let dir = path.parent()?.to_str()?;
    let name = path.file_name()?.to_str()?;
    (NETNS_DIRS.contains(&dir) && plain_name(name)).then(|| name.to_owned())
}

/// A name that is one path component and safe as an argument.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['.', '-'])
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// What `uninstall` removes of a pod network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    /// The network.
    pub network: PodNetwork,
    /// The network namespaces its results record; their `veth` links go with them.
    pub namespaces: Vec<String>,
    /// The sandboxes its results record, whose loopback results go too.
    pub containers: Vec<String>,
    /// The bridge is removed only when no other distribution is on the host.
    pub bridge: Option<String>,
    /// [`KUBELET_DIR`] is removed only when no other distribution is on the host.
    pub kubelet: bool,
}

impl Removal {
    /// What to remove of `network`, with the attachments its results record. `alone` is true
    /// when no other distribution is on the host.
    #[must_use]
    pub fn new(network: PodNetwork, attachments: Vec<Attachment>, alone: bool) -> Self {
        let mut namespaces: Vec<String> =
            attachments.iter().filter_map(|a| a.netns.clone()).collect();
        namespaces.sort();
        namespaces.dedup();
        let mut containers: Vec<String> = attachments.into_iter().map(|a| a.container).collect();
        containers.sort();
        containers.dedup();
        Self {
            bridge: network.bridge.clone().filter(|_| alone),
            network,
            namespaces,
            containers,
            kubelet: alone,
        }
    }

    /// The loopback results of the network's sandboxes, under [`RESULTS_DIR`].
    #[must_use]
    pub fn loopback_results(&self) -> Vec<String> {
        self.containers
            .iter()
            .map(|id| format!("{RESULTS_DIR}/{LOOPBACK_NETWORK}-{id}-lo"))
            .collect()
    }

    /// What `uninstall` shows before it asks.
    #[must_use]
    pub fn describe(&self) -> String {
        let name = &self.network.name;
        let mut parts = Vec::new();
        if let Some(bridge) = &self.bridge {
            parts.push(format!("the bridge {bridge} and its route"));
        }
        if !self.namespaces.is_empty() {
            parts.push(format!(
                "the network namespaces of its pods ({}) and their veth links",
                self.namespaces.join(", ")
            ));
        }
        parts.push(format!(
            "{} and {RESULTS_DIR}/{name}-*",
            self.network.addresses_dir()
        ));
        if !self.containers.is_empty() {
            parts.push(format!(
                "the loopback results of its pods ({RESULTS_DIR}/{LOOPBACK_NETWORK}-*)"
            ));
        }
        if self.kubelet {
            parts.push(KUBELET_DIR.to_owned());
        }
        format!(
            "zeatend removes KubeSolo's pod network {name}, as {} names it: {}.",
            kubesolo::CNI_CONFIG,
            parts.join("; ")
        )
    }
}

/// What `uninstall` shows about the pod network when [`kubesolo::CNI_CONFIG`] is missing or
/// cannot be read: it removes nothing of the network, and names what to check.
#[must_use]
pub fn unread_text() -> String {
    format!(
        "{} is missing or is not a CNI configuration, so zeatend removes nothing of KubeSolo's pod \
         network. Check for what KubeSolo left with `ip link show type bridge`, `ip netns list`, \
         {NETWORKS_DIR}/, {RESULTS_DIR}/ and {KUBELET_DIR}.",
        kubesolo::CNI_CONFIG
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `KubeSolo` v1.3.0's `/etc/cni/net.d/10-bridge.conflist`.
    const CONFLIST: &str = r#"{
  "cniVersion": "1.0.0",
  "name": "kubesolo-net",
  "plugins": [
    {
      "bridge": "cni0",
      "capabilities": {"ips": true, "portMappings": true},
      "hairpinMode": true,
      "ipMasq": false,
      "ipam": {
        "ranges": [[{"subnet": "10.42.0.0/16"}]],
        "routes": [{"dst": "0.0.0.0/0"}],
        "type": "host-local"
      },
      "isGateway": true,
      "mtu": 1500,
      "type": "bridge"
    },
    {"capabilities": {"portMappings": true}, "type": "portmap"},
    {"type": "loopback"}
  ]
}"#;

    /// A result file from `/var/lib/cni/results/kubesolo-net-<container>-eth0`.
    const RESULT: &str = r#"{"kind":"cniCacheV1","containerId":"0f1e2d","config":"e30=","ifName":"eth0","networkName":"kubesolo-net","netns":"/var/run/netns/cni-5a0c3e1d-7b4f-2c8e-9d6a-1f3b5c7e9a2d","result":{"cniVersion":"1.0.0","interfaces":[{"name":"cni0","mac":"aa:bb:cc:dd:ee:ff"},{"name":"veth1a2b3c4d","mac":"aa:bb:cc:dd:ee:01"},{"name":"eth0","mac":"aa:bb:cc:dd:ee:02","sandbox":"/var/run/netns/cni-5a0c3e1d-7b4f-2c8e-9d6a-1f3b5c7e9a2d"}],"ips":[{"interface":2,"address":"10.42.0.5/16","gateway":"10.42.0.1"}]}}"#;

    fn network() -> PodNetwork {
        PodNetwork::parse(CONFLIST).unwrap()
    }

    #[test]
    fn the_names_and_paths_come_from_kubesolos_conflist() {
        let network = network();
        assert_eq!(network.name, "kubesolo-net");
        assert_eq!(network.bridge.as_deref(), Some("cni0"));
        assert_eq!(
            network.addresses_dir(),
            "/var/lib/cni/networks/kubesolo-net"
        );
        assert!(network.owns_result("kubesolo-net-0f1e2d-eth0"));
        assert!(!network.owns_result("kubesolo-network-0f1e2d-eth0"));
        assert!(!network.owns_result("cbr0-0f1e2d-eth0"));
        assert_eq!(
            network.result_attachment(RESULT),
            Some(Attachment {
                container: "0f1e2d".to_owned(),
                netns: Some("cni-5a0c3e1d-7b4f-2c8e-9d6a-1f3b5c7e9a2d".to_owned()),
            })
        );
    }

    #[test]
    fn the_namespace_comes_from_the_interfaces_when_the_result_names_none() {
        let older = RESULT.replace(
            r#""netns":"/var/run/netns/cni-5a0c3e1d-7b4f-2c8e-9d6a-1f3b5c7e9a2d","#,
            "",
        );
        assert_eq!(
            network()
                .result_attachment(&older)
                .and_then(|a| a.netns)
                .as_deref(),
            Some("cni-5a0c3e1d-7b4f-2c8e-9d6a-1f3b5c7e9a2d")
        );
    }

    #[test]
    fn a_result_of_another_network_or_namespace_is_left() {
        let network = network();
        let other = RESULT.replace(r#""networkName":"kubesolo-net""#, r#""networkName":"cbr0""#);
        assert_eq!(network.result_attachment(&other), None);
        let unsafe_id = RESULT.replace(r#""containerId":"0f1e2d""#, r#""containerId":"../x""#);
        assert_eq!(network.result_attachment(&unsafe_id), None);
        let proc = RESULT.replace("/var/run/netns/", "/proc/4242/ns/");
        assert_eq!(network.result_attachment(&proc).unwrap().netns, None);
        assert_eq!(network.result_attachment("not json"), None);
    }

    #[test]
    fn the_bridge_and_the_kubelet_directory_stay_when_another_distribution_is_on_the_host() {
        let attachment = |id: &str, ns: &str| Attachment {
            container: id.to_owned(),
            netns: Some(ns.to_owned()),
        };
        let attachments = || {
            vec![
                attachment("b1", "cni-b"),
                attachment("a1", "cni-a"),
                attachment("b1", "cni-b"),
            ]
        };
        let alone = Removal::new(network(), attachments(), true);
        assert_eq!(alone.namespaces, ["cni-a", "cni-b"]);
        assert_eq!(
            alone.loopback_results(),
            [
                "/var/lib/cni/results/cni-loopback-a1-lo",
                "/var/lib/cni/results/cni-loopback-b1-lo"
            ]
        );
        assert_eq!(alone.bridge.as_deref(), Some("cni0"));
        assert!(alone.kubelet);
        assert_eq!(
            alone.describe(),
            "zeatend removes KubeSolo's pod network kubesolo-net, as \
             /etc/cni/net.d/10-bridge.conflist names it: the bridge cni0 and its route; the \
             network namespaces of its pods (cni-a, cni-b) and their veth links; \
             /var/lib/cni/networks/kubesolo-net and /var/lib/cni/results/kubesolo-net-*; the \
             loopback results of its pods (/var/lib/cni/results/cni-loopback-*); /var/lib/kubelet."
        );
        let shared = Removal::new(network(), Vec::new(), false);
        assert_eq!(shared.bridge, None);
        assert!(!shared.kubelet);
        assert_eq!(
            shared.describe(),
            "zeatend removes KubeSolo's pod network kubesolo-net, as \
             /etc/cni/net.d/10-bridge.conflist names it: /var/lib/cni/networks/kubesolo-net and \
             /var/lib/cni/results/kubesolo-net-*."
        );
    }

    #[test]
    fn a_bridge_plugin_without_a_name_uses_cni0() {
        let unnamed = CONFLIST.replace(r#""bridge": "cni0","#, "");
        assert_eq!(
            PodNetwork::parse(&unnamed).unwrap().bridge.as_deref(),
            Some("cni0")
        );
    }

    #[test]
    fn a_conflist_with_unsafe_names_is_not_read() {
        for (from, to) in [
            (r#""name": "kubesolo-net""#, r#""name": "../etc""#),
            (r#""name": "kubesolo-net""#, r#""name": """#),
            (r#""bridge": "cni0""#, r#""bridge": "-x""#),
        ] {
            assert_eq!(PodNetwork::parse(&CONFLIST.replace(from, to)), None, "{to}");
        }
        assert_eq!(PodNetwork::parse("{}"), None);
    }
}
