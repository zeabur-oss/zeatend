//! The Kubernetes distributions: `KubeSolo`, K3s and RKE2, and the host files K3s and RKE2 share.
//!
//! RKE2's server is K3s's, so both read the same keys from `/etc/rancher/<name>/config.yaml`, then
//! `config.yaml.d/*.yaml` in name order. K3s is one process with `SQLite`: it takes workers but no
//! second server. RKE2 runs the control plane as static pods with etcd, and three servers keep it
//! up when one fails. K3s runs its packaged flannel VXLAN and the kube-router policy controller;
//! RKE2 brings Canal, Calico's policy engine on flannel. `KubeSolo` is one node with no workers,
//! for hosts with little memory.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::PlanError;
use crate::config::Component;
use crate::pins::{K3S, KUBESOLO, RKE2};
use crate::tuning::{HostTuning, MemoryTier};

/// The Kubernetes distribution zeatend installs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Distribution {
    /// `KubeSolo`: one node and no workers, for a host with little memory.
    #[serde(rename = "kubesolo")]
    KubeSolo,
    /// K3s, hardened like RKE2's `cis` profile, with flannel VXLAN and kube-router. One server
    /// and any number of workers.
    #[default]
    K3s,
    /// RKE2 with its `cis` profile and Canal. Servers and workers.
    Rke2,
}

impl Distribution {
    /// Its name in configuration files.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::KubeSolo => "kubesolo",
            Self::K3s => Rancher::K3s.id(),
            Self::Rke2 => Rancher::Rke2.id(),
        }
    }

    /// Its name for people.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::KubeSolo => "KubeSolo",
            Self::K3s => Rancher::K3s.title(),
            Self::Rke2 => Rancher::Rke2.title(),
        }
    }

    /// How the distribution packages `component`; `None` when it does not package it.
    #[must_use]
    pub const fn packaged(self, component: Component) -> Option<Packaged> {
        let (manifest, setting, item) = match (self, component) {
            (Self::K3s, Component::Traefik) => ("traefik", "disable", "traefik"),
            (Self::K3s, Component::MetricsServer) => {
                ("metrics-server", "disable", "metrics-server")
            }
            (Self::K3s, Component::LocalStorage) => ("local-storage", "disable", "local-storage"),
            // RKE2 chooses its ingress controller, and `traefik` is not an item of `disable`.
            (Self::Rke2, Component::Traefik) => ("rke2-traefik", "ingress-controller", "none"),
            (Self::Rke2, Component::MetricsServer) => {
                ("rke2-metrics-server", "disable", "rke2-metrics-server")
            }
            (Self::Rke2, Component::LocalStorage) | (Self::KubeSolo, _) => return None,
        };
        Some(Packaged {
            manifest,
            setting,
            item,
        })
    }

    /// K3s or RKE2, which take workers and share a server; `None` for `KubeSolo`.
    #[must_use]
    pub const fn rancher(self) -> Option<Rancher> {
        match self {
            Self::KubeSolo => None,
            Self::K3s => Some(Rancher::K3s),
            Self::Rke2 => Some(Rancher::Rke2),
        }
    }

    /// The pinned release.
    #[must_use]
    pub const fn pinned_version(self) -> &'static str {
        match self {
            Self::KubeSolo => KUBESOLO.version,
            Self::K3s => Rancher::K3s.pinned_version(),
            Self::Rke2 => Rancher::Rke2.pinned_version(),
        }
    }

    /// The Kubernetes minor of the pinned release.
    #[must_use]
    pub const fn kubernetes(self) -> &'static str {
        match self {
            Self::KubeSolo => KUBESOLO.kubernetes,
            Self::K3s => Rancher::K3s.kubernetes(),
            Self::Rke2 => Rancher::Rke2.kubernetes(),
        }
    }
}

impl From<Rancher> for Distribution {
    fn from(rancher: Rancher) -> Self {
        match rancher {
            Rancher::K3s => Self::K3s,
            Rancher::Rke2 => Self::Rke2,
        }
    }
}

/// What a node or the server uses a port for. Serialized as a camelCase key; an interface words
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PortUse {
    /// The Kubernetes API server.
    KubernetesApi,
    /// The K3s server, where the Kubernetes API and the K3s supervisor share one port.
    K3sApiAndSupervisor,
    /// The RKE2 supervisor, which nodes join through.
    Rke2Supervisor,
    /// The kubelet.
    Kubelet,
    /// etcd's clients.
    EtcdClient,
    /// etcd's peers.
    EtcdPeer,
    /// HTTP ingress: `ServiceLB` holds it on every node for a `LoadBalancer` Service.
    ServiceLbHttp,
    /// HTTPS ingress: `ServiceLB` holds it on every node for a `LoadBalancer` Service.
    ServiceLbHttps,
    /// HTTP ingress: `KubeSolo` gives a `LoadBalancer` Service this host's address.
    KubeSoloHttp,
    /// HTTPS ingress: `KubeSolo` gives a `LoadBalancer` Service this host's address.
    KubeSoloHttps,
    /// `KubeSolo`'s node-assignment webhook.
    KubeSoloWebhook,
    /// flannel's VXLAN, between pods on different K3s nodes.
    FlannelVxlan,
    /// Canal's VXLAN, between pods on different RKE2 nodes.
    CanalVxlan,
}

/// K3s or RKE2: the distributions that run the same server, take workers and read
/// `/etc/rancher/<name>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Rancher {
    /// K3s.
    K3s,
    /// RKE2.
    Rke2,
}

/// Where a distribution keeps its files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// Its configuration directory.
    pub config_dir: &'static str,
    /// The cluster-admin kubeconfig, kept for emergencies.
    pub admin_kubeconfig: &'static str,
    /// The `kubectl` that matches the server.
    pub kubectl: &'static str,
    /// Archives the distribution imports into containerd on every start.
    pub images_dir: &'static str,
    /// The join token of a joining node, mode 0600.
    pub join_token_file: &'static str,
    /// A server's own token; it joins servers and workers.
    pub server_token_file: &'static str,
    /// The kubelet's credentials on any node; a node may read its own Node object with them.
    pub kubelet_kubeconfig: &'static str,
    /// The kubelet's client certificate, whose subject names the node.
    pub kubelet_cert: &'static str,
    /// The server's certificates and keys. The distribution writes the certificates 0644; the
    /// CIS benchmark wants 0600.
    pub tls_dir: &'static str,
    /// The kernel parameters `protect-kernel-defaults` requires.
    pub sysctl_file: &'static str,
}

const K3S_LAYOUT: Layout = Layout {
    config_dir: "/etc/rancher/k3s",
    admin_kubeconfig: "/etc/rancher/k3s/k3s.yaml",
    kubectl: "/var/lib/rancher/k3s/data/current/bin/kubectl",
    images_dir: "/var/lib/rancher/k3s/agent/images",
    join_token_file: "/etc/rancher/k3s/zeatend-join-token",
    server_token_file: "/var/lib/rancher/k3s/server/token",
    kubelet_kubeconfig: "/var/lib/rancher/k3s/agent/kubelet.kubeconfig",
    kubelet_cert: "/var/lib/rancher/k3s/agent/client-kubelet.crt",
    tls_dir: "/var/lib/rancher/k3s/server/tls",
    sysctl_file: "/etc/sysctl.d/60-k3s-cis.conf",
};

const RKE2_LAYOUT: Layout = Layout {
    config_dir: "/etc/rancher/rke2",
    admin_kubeconfig: "/etc/rancher/rke2/rke2.yaml",
    kubectl: "/var/lib/rancher/rke2/bin/kubectl",
    images_dir: "/var/lib/rancher/rke2/agent/images",
    join_token_file: "/etc/rancher/rke2/zeatend-join-token",
    server_token_file: "/var/lib/rancher/rke2/server/token",
    kubelet_kubeconfig: "/var/lib/rancher/rke2/agent/kubelet.kubeconfig",
    kubelet_cert: "/var/lib/rancher/rke2/agent/client-kubelet.crt",
    tls_dir: "/var/lib/rancher/rke2/server/tls",
    sysctl_file: "/etc/sysctl.d/60-rke2-cis.conf",
};

/// containerd's `AppArmor` profile for containers, under the name containerd gives it.
pub const APPARMOR_PROFILE: &str = "/etc/apparmor.d/cri-containerd.apparmor.d";

/// containerd's default profile, which also lets a container's processes signal and trace each
/// other under the stacked label `cri-containerd.apparmor.d//&unconfined`.
///
/// Ubuntu's kernels from 7.0.0-41 on give containers that label when
/// `kernel.apparmor_restrict_unprivileged_unconfined` is 1, as on Ubuntu 26.04. containerd loads
/// its own profile only when none of that name is loaded, and `apparmor.service` loads this one at
/// boot.
#[must_use]
pub fn apparmor_profile() -> HostFile {
    HostFile {
        path: APPARMOR_PROFILE.to_owned(),
        mode: 0o644,
        content: include_str!("cri-containerd.apparmor.d").to_owned(),
    }
}

/// RKE2 and its control plane at idle, containerd, kubelet and the network included. On a 4 GiB VM
/// with the default components, `free` showed 2640 MiB used, about 500 of it the components. The
/// rest is split by the process figures from that VM: kube-apiserver about 980, etcd 97,
/// kube-controller-manager 92, canal 59, then kubelet, containerd and the rest of RKE2.
const RKE2_IDLE_MIB: u64 = 1730;
// What the RKE2 control plane's processes used at idle on a 4 GiB VM (`RKE2_IDLE_MIB`).
const RKE2_APISERVER_IDLE_MIB: u64 = 980;
const RKE2_ETCD_IDLE_MIB: u64 = 97;
const RKE2_CONTROLLER_MANAGER_IDLE_MIB: u64 = 92;
const RKE2_CANAL_IDLE_MIB: u64 = 59;

// RKE2 asks for 512Mi for etcd and 256Mi for kube-controller-manager, which idle near 100Mi;
// less leaves more of what the reserves allow to pods.
const RKE2_ETCD_REQUEST_MIB: u64 = 256;
const RKE2_CONTROLLER_MANAGER_REQUEST_MIB: u64 = 128;
/// The memory requests of RKE2's static pods: kube-apiserver 1024, etcd, kube-controller-manager,
/// then kube-scheduler, kube-proxy and cloud-controller-manager at 128 each.
const RKE2_STATIC_PODS_REQUEST_MIB: u64 =
    1024 + RKE2_ETCD_REQUEST_MIB + RKE2_CONTROLLER_MANAGER_REQUEST_MIB + 3 * 128;
/// rke2-coredns and its autoscaler.
const RKE2_COREDNS_REQUEST_MIB: u64 = 144;
/// The least memory K3s needs: a 2 GiB host, less what the kernel and firmware keep, with the
/// reserves of [`MemoryTier::Small`]. Below it, only `KubeSolo` fits.
const K3S_MIN_MEMORY_MIB: u64 = 1900;

impl Rancher {
    /// Both, in the order an interface lists them.
    pub const ALL: [Self; 2] = [Self::K3s, Self::Rke2];

    /// The directory whose manifests the server's deploy controller applies.
    #[must_use]
    pub const fn manifests_dir(self) -> &'static str {
        match self {
            Self::K3s => crate::k3s::MANIFESTS_DIR,
            Self::Rke2 => crate::rke2::MANIFESTS_DIR,
        }
    }

    /// Its name in configuration files.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::K3s => "k3s",
            Self::Rke2 => "rke2",
        }
    }

    /// Its name for people.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::K3s => "K3s",
            Self::Rke2 => "RKE2",
        }
    }

    /// Where it keeps its files.
    #[must_use]
    pub const fn layout(self) -> &'static Layout {
        match self {
            Self::K3s => &K3S_LAYOUT,
            Self::Rke2 => &RKE2_LAYOUT,
        }
    }

    /// The systemd unit of a server.
    #[must_use]
    pub const fn server_unit(self) -> &'static str {
        match self {
            Self::K3s => "k3s",
            Self::Rke2 => "rke2-server",
        }
    }

    /// The systemd unit of a worker.
    #[must_use]
    pub const fn agent_unit(self) -> &'static str {
        match self {
            Self::K3s => "k3s-agent",
            Self::Rke2 => "rke2-agent",
        }
    }

    /// The server port a node joins through: RKE2's supervisor, or K3s's, which shares the
    /// Kubernetes API's port.
    #[must_use]
    pub const fn join_port(self) -> u16 {
        match self {
            Self::K3s => 6443,
            Self::Rke2 => 9345,
        }
    }

    /// Whether another server can join, for a control plane that survives losing one.
    #[must_use]
    pub const fn takes_servers(self) -> bool {
        matches!(self, Self::Rke2)
    }

    /// The Kubernetes minor of the pinned release.
    #[must_use]
    pub const fn kubernetes(self) -> &'static str {
        match self {
            Self::K3s => K3S.kubernetes,
            Self::Rke2 => RKE2.kubernetes,
        }
    }

    /// The pinned release.
    #[must_use]
    pub const fn pinned_version(self) -> &'static str {
        match self {
            Self::K3s => K3S.version,
            Self::Rke2 => RKE2.version,
        }
    }

    /// The memory the kubelet keeps from pods on an RKE2 server for RKE2 (`kube-reserved`).
    /// RKE2 runs the control plane in static pods, whose requests already count against what pods
    /// may use: this is what is left of RKE2 at idle without kube-apiserver, etcd,
    /// kube-controller-manager and canal.
    pub(crate) const RKE2_KUBE_RESERVED_MIB: u64 = RKE2_IDLE_MIB
        - (RKE2_APISERVER_IDLE_MIB
            + RKE2_ETCD_IDLE_MIB
            + RKE2_CONTROLLER_MANAGER_IDLE_MIB
            + RKE2_CANAL_IDLE_MIB);

    /// The kubelet's arguments. With the reserves, the pods' cgroup ends short of the OS and the
    /// distribution, so memory running out kills or evicts a pod, and not `k3s-server`. A worker
    /// has no control plane and reserves only the OS. None when `tuning` has no reserves.
    pub(crate) fn kubelet_args(tuning: &HostTuning, server: bool) -> String {
        let Some(reserves) = tuning.kubelet else {
            return String::new();
        };
        let kube = if server {
            format!("  - kube-reserved=memory={}Mi\n", reserves.kube)
        } else {
            String::new()
        };
        let soft = reserves.eviction_soft.map_or_else(String::new, |soft| {
            format!(
                "  - eviction-soft=memory.available<{soft}Mi\n  \
                 - eviction-soft-grace-period=memory.available=30s\n"
            )
        });
        format!(
            "{KUBELET_ARGS}  - system-reserved=memory={}Mi\n{kube}  \
             - eviction-hard=memory.available<{}Mi,{DISK_EVICTION_HARD}\n{soft}",
            reserves.system, reserves.eviction_hard
        )
    }

    /// The least memory a server needs.
    ///
    /// K3s: 1900 MiB, a 2 GiB host less what the kernel and firmware keep. RKE2: the scheduler
    /// places a pod only where its memory request fits in what pods may use, so that must hold the
    /// requests of RKE2's static pods and `CoreDNS` on top of the reserves and the hard eviction
    /// threshold, rounded up to a hundred.
    #[must_use]
    pub const fn min_memory_mib(self) -> u64 {
        match self {
            Self::K3s => K3S_MIN_MEMORY_MIB,
            Self::Rke2 => {
                let own_pods = RKE2_STATIC_PODS_REQUEST_MIB + RKE2_COREDNS_REQUEST_MIB;
                match HostTuning::new(Distribution::Rke2, MemoryTier::Small).kubelet {
                    Some(r) => (own_pods + r.system + r.kube + r.eviction_hard).div_ceil(100) * 100,
                    None => own_pods,
                }
            }
        }
    }

    /// The label `ServiceLB` puts on its pods.
    #[must_use]
    pub const fn servicelb_label(self) -> &'static str {
        match self {
            Self::K3s => "svccontroller.k3s.cattle.io/svcname",
            Self::Rke2 => "svccontroller.rke2.cattle.io/svcname",
        }
    }

    /// The UDP port pods on different nodes reach each other through, and what uses it.
    #[must_use]
    pub const fn vxlan(self) -> (u16, PortUse) {
        match self {
            Self::K3s => (8472, PortUse::FlannelVxlan),
            Self::Rke2 => (8472, PortUse::CanalVxlan),
        }
    }

    /// The TCP ports a node listens on, and what for, that must be free before it starts.
    ///
    /// Every node takes 80 and 443: `ServiceLB` gives an ingress's `LoadBalancer` Service a pod on
    /// each node that holds them as host ports, so a request to any node's address reaches the
    /// ingress. A worker runs no API server or supervisor.
    #[must_use]
    pub fn node_ports(self, server: bool) -> Vec<(u16, PortUse)> {
        let mut ports = vec![(80, PortUse::ServiceLbHttp), (443, PortUse::ServiceLbHttps)];
        if server {
            ports.push((6443, PortUse::KubernetesApi));
            if self == Self::Rke2 {
                ports.push((9345, PortUse::Rke2Supervisor));
            }
        }
        ports.push((10250, PortUse::Kubelet));
        ports
    }

    /// The files of a server, in the order to write them.
    #[must_use]
    pub fn server_files(self, options: &Options) -> Vec<HostFile> {
        let mut files = match self {
            Self::K3s => crate::k3s::host_files(options),
            Self::Rke2 => crate::rke2::host_files(options),
        };
        files.push(self.servicelb_policy());
        files
    }

    /// The `NetworkPolicy` that lets outside traffic into the `ServiceLB` pods, as a manifest the
    /// server's deploy controller applies.
    ///
    /// `kube-system/default-network-policy` lets in only pods of `kube-system`: RKE2's `cis`
    /// profile adds it, and zeatend adds it on K3s. Without this policy, a `ServiceLB` pod on a
    /// worker gets no outside requests. Each distribution labels the pods with its own prefix.
    fn servicelb_policy(self) -> HostFile {
        let manifests = self.manifests_dir();
        let policy = serde_json::json!({
            "apiVersion": "networking.k8s.io/v1",
            "kind": "NetworkPolicy",
            "metadata": {
                "name": "zeatend-servicelb",
                "namespace": "kube-system",
                "labels": {"app.kubernetes.io/managed-by": crate::plan::MANAGER},
            },
            "spec": {
                "podSelector": {"matchExpressions": [
                    {"key": self.servicelb_label(), "operator": "Exists"},
                ]},
                "policyTypes": ["Ingress"],
                "ingress": [{}],
            },
        });
        HostFile {
            path: format!("{manifests}/zeatend-servicelb-policy.yaml"),
            mode: 0o600,
            content: format!(
                "# Written by zeatend; {} applies it on every start.\n---\n{policy}\n",
                self.title()
            ),
        }
    }

    /// The files of a worker.
    #[must_use]
    pub fn agent_files(self, tuning: &HostTuning) -> Vec<HostFile> {
        match self {
            Self::K3s => crate::k3s::agent_files(tuning),
            Self::Rke2 => crate::rke2::agent_files(tuning),
        }
    }

    /// The kernel parameters `protect-kernel-defaults` makes the kubelet check; it refuses to
    /// start when they differ. Every node needs them.
    #[must_use]
    pub fn sysctl(self) -> HostFile {
        HostFile {
            path: self.layout().sysctl_file.to_owned(),
            mode: 0o644,
            content: format!(
                "# Written by zeatend: the kernel parameters {} requires.\n\
                 vm.panic_on_oom=0\nvm.overcommit_memory=1\nkernel.panic=10\nkernel.panic_on_oops=1\n",
                self.title()
            ),
        }
    }

    /// The drop-in naming the server to join and the token file.
    #[must_use]
    pub fn join_file(self, server: &str) -> HostFile {
        let layout = self.layout();
        HostFile {
            path: format!("{}/config.yaml.d/10-join.yaml", layout.config_dir),
            mode: 0o600,
            content: format!(
                "{HEADER}server: {server}\ntoken-file: {}\n",
                layout.join_token_file
            ),
        }
    }

    /// The drop-in naming the node, so a later change of the host name does not register a
    /// second node.
    #[must_use]
    pub fn node_name_file(self, name: &str) -> HostFile {
        HostFile {
            path: self.node_name_path(),
            mode: 0o600,
            // Quoted, so names like `on` or `001` stay strings for K3s's YAML parser.
            content: format!("{HEADER}node-name: {}\n", serde_json::json!(name)),
        }
    }

    /// Where [`Self::node_name_file`] goes.
    #[must_use]
    fn node_name_path(self) -> String {
        format!(
            "{}/config.yaml.d/15-node-name.yaml",
            self.layout().config_dir
        )
    }

    /// The drop-in naming the addresses the node registers with, so it does not pick a temporary
    /// IPv6 address. `None` when the host's addresses could not be read.
    #[must_use]
    pub fn node_ip_file(self, addresses: &[String]) -> Option<HostFile> {
        if addresses.is_empty() {
            return None;
        }
        let mut content = format!("{HEADER}node-ip:\n");
        for address in addresses {
            for part in ["  - ", address, "\n"] {
                content.push_str(part);
            }
        }
        Some(HostFile {
            path: format!("{}/config.yaml.d/20-node-ip.yaml", self.layout().config_dir),
            mode: 0o600,
            content,
        })
    }
}

/// A file zeatend owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFile {
    /// Absolute path.
    pub path: String,
    /// Unix mode.
    pub mode: u32,
    /// Exact content.
    pub content: String,
}

/// A component that K3s or RKE2 packages: the manifest the server deploys it from, and the item
/// of a list setting in the server's configuration that turns it off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packaged {
    /// The manifest in the manifests directory, without `.yaml`; it names the Addon.
    pub manifest: &'static str,
    /// The list setting, such as `disable`.
    pub setting: &'static str,
    /// The item of `setting` that turns the component off.
    pub item: &'static str,
}

/// Server settings from the configuration and the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Extra API server certificate names.
    pub tls_san: Vec<String>,
    /// The host's memory settings.
    pub tuning: HostTuning,
    /// K3s only: an earlier install turned flannel off for a network plugin of its own, which
    /// zeatend keeps.
    pub own_network: bool,
    /// Packaged components that the configuration turns off.
    pub disable: Vec<Component>,
}

/// The packaged components that the K3s or RKE2 configuration `config` of an earlier install
/// turns off, for zeatend's `disable`.
///
/// zeabur-installer turns off Traefik, metrics-server and K3s's `local-storage`, as the Zeabur
/// catalog installs its own; turning them on again would give two of each.
///
/// # Errors
///
/// When the configuration does not parse, or turns off a component zeatend cannot name.
pub fn disabled_components(
    distribution: Distribution,
    config: &str,
) -> Result<Vec<Component>, PlanError> {
    let value: Value =
        serde_saphyr::from_str(config).map_err(|e| PlanError::Config(e.to_string()))?;
    let table: Vec<(Component, Packaged)> = Component::ALL
        .into_iter()
        .filter_map(|c| Some((c, distribution.packaged(c)?)))
        .collect();
    let mut off = Vec::new();
    for setting in settings(&table) {
        for item in list(value.get(setting)) {
            let (component, _) = table
                .iter()
                .find(|(_, p)| p.setting == setting && p.item == item)
                .ok_or_else(|| {
                    PlanError::Config(format!("{setting}: zeatend has no component for {item}"))
                })?;
            off.push(*component);
        }
    }
    Ok(Component::ALL
        .into_iter()
        .filter(|c| off.contains(c))
        .collect())
}

/// The list settings of `table`, each once, in the order of the table.
fn settings(table: &[(Component, Packaged)]) -> Vec<&'static str> {
    let mut settings: Vec<&'static str> = Vec::new();
    for (_, packaged) in table {
        if !settings.contains(&packaged.setting) {
            settings.push(packaged.setting);
        }
    }
    settings
}

/// The items of a list setting, which a configuration writes as a list or a comma-separated
/// string.
fn list(value: Option<&Value>) -> Vec<String> {
    let items: Vec<String> = match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::String(items)) => items.split(',').map(|i| i.trim().to_owned()).collect(),
        _ => Vec::new(),
    };
    items.into_iter().filter(|item| !item.is_empty()).collect()
}

/// The settings that turn off the components in `disable`.
pub(crate) fn turn_off(out: &mut String, distribution: Distribution, disable: &[Component]) {
    let table: Vec<(Component, Packaged)> = disable
        .iter()
        .filter_map(|c| Some((*c, distribution.packaged(*c)?)))
        .collect();
    for setting in settings(&table) {
        let items: Vec<String> = table
            .iter()
            .filter(|(_, p)| p.setting == setting)
            .map(|(_, p)| p.item.to_owned())
            .collect();
        yaml_list(out, setting, &items);
    }
}

/// The objects that the server has not removed yet for the components in `disable`.
///
/// The result has `addon/<name>` and `helmchart/<name>` items. `addons` and `charts` are the
/// API's lists of the Addons and the `HelmCharts` in `kube-system`; `exists` tells whether a file
/// exists on the host.
///
/// The server keeps an Addon for each manifest, with the checksum of the manifest, and applies a
/// manifest only when its checksum changes. To turn a component off, K3s deletes the manifest,
/// and RKE2 empties the manifest of an ingress controller. The server then deletes the
/// `HelmCharts` that the manifest made, which uninstalls them, and K3s also deletes the Addon.
///
/// When the server restarts before the removal completes and `disable` turns the component on
/// again, the Addon of a deleted manifest gets the checksum again while the uninstall deletes the
/// `HelmChart`. The server then does not apply the manifest again, and the component does not
/// come back. An Addon whose manifest exists has the checksum of the emptied manifest, which
/// changes when the component is turned on again.
#[must_use]
pub fn left_to_remove(
    rancher: Rancher,
    disable: &[Component],
    addons: &Value,
    charts: &Value,
    exists: impl Fn(&str) -> bool,
) -> Vec<String> {
    let manifests = rancher.manifests_dir();
    let names: Vec<&str> = disable
        .iter()
        .filter_map(|c| Some(Distribution::from(rancher).packaged(*c)?.manifest))
        .collect();
    let items = |list: &'_ Value| -> Vec<Value> {
        list.get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let text = |object: &Value, pointer: &str| -> String {
        object
            .pointer(pointer)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let addons = items(addons).into_iter().filter(|addon| {
        let source = text(addon, "/spec/source");
        !exists(&source)
            && names.iter().any(|name| {
                source == format!("{manifests}/{name}.yaml")
                    || source.starts_with(&format!("{manifests}/{name}/"))
            })
    });
    let charts = items(charts).into_iter().filter(|chart| {
        let owner = text(
            chart,
            "/metadata/annotations/objectset.rio.cattle.io~1owner-name",
        );
        names.contains(&owner.as_str())
    });
    addons
        .map(|addon| format!("addon/{}", text(&addon, "/metadata/name")))
        .chain(charts.map(|chart| format!("helmchart/{}", text(&chart, "/metadata/name"))))
        .collect()
}

pub(crate) const HEADER: &str = "# Written by zeatend; `zeatend apply` writes it again.\n\
                      # Put your own settings in config.yaml.d/90-local.yaml instead of editing this file.\n";

/// Metadata only, never bodies, so Secret values stay out of the log. Every request on Secrets
/// is kept, reads included, with who made it. Of the rest, only changes by people and workloads
/// are kept: reads, events, leases, token and access reviews, and requests from nodes and
/// control-plane components are frequent and routine. The first rule that matches decides. The
/// log keeps 30 days, 10 files of 100 MB, under `/var/lib/rancher/<name>/server/logs`.
pub(crate) const AUDIT_POLICY: &str = r#"# Written by zeatend.
apiVersion: audit.k8s.io/v1
kind: Policy
omitStages: [RequestReceived]
rules:
  - level: Metadata
    resources:
      - group: ""
        resources: [secrets]
  - level: None
    nonResourceURLs: [/healthz*, /livez*, /readyz*, /version]
  - level: None
    userGroups: [system:nodes]
  - level: None
    users: [system:apiserver, system:kube-controller-manager, system:kube-scheduler, system:kube-proxy]
  - level: None
    verbs: [get, list, watch]
  - level: None
    resources:
      - group: ""
        resources: [events]
      - group: events.k8s.io
        resources: [events]
      - group: coordination.k8s.io
        resources: [leases]
      - group: authentication.k8s.io
        resources: [tokenreviews]
      - group: authorization.k8s.io
        resources: [subjectaccessreviews]
  - level: Metadata
"#;

/// The kubelet settings every node gets: a fork bomb in one pod cannot take the node's process
/// table, an idle exec or log stream closes after five minutes, and the port serves only the cipher
/// suites the CIS benchmark allows.
const KUBELET_ARGS: &str = "kubelet-arg:\n  - pod-max-pids=4096\n  \
     - streaming-connection-idle-timeout=5m\n  - tls-cipher-suites=\
TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,\
TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,\
TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305,TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305\n";

/// The disk thresholds K3s sets in its kubelet configuration, which RKE2 runs as its agent
/// (<https://github.com/k3s-io/k3s/blob/3acf8db8f2096ae0ea34f4b4ce1048e6472b89be/pkg/daemons/agent/agent.go#L192-L195>).
/// The flag replaces the whole map, and a threshold it leaves out is zero
/// (<https://kubernetes.io/docs/concepts/scheduling-eviction/node-pressure-eviction/#hard-eviction-thresholds>).
const DISK_EVICTION_HARD: &str = "nodefs.available<5%,imagefs.available<5%";

/// RKE2's `control-plane-resource-requests` for etcd and kube-controller-manager.
pub(crate) fn control_plane_requests() -> String {
    format!(
        "control-plane-resource-requests:\n  \
           - etcd-memory={RKE2_ETCD_REQUEST_MIB}Mi\n  \
           - kube-controller-manager-memory={RKE2_CONTROLLER_MANAGER_REQUEST_MIB}Mi\n"
    )
}

/// A YAML list under `key`, or nothing when it is empty.
pub(crate) fn yaml_list(out: &mut String, key: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(key);
    out.push_str(":\n");
    for item in items {
        out.push_str("  - ");
        out.push_str(item);
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_gets_the_servicelb_policy_as_a_manifest() {
        for rancher in Rancher::ALL {
            let tuning = HostTuning::new(rancher.into(), MemoryTier::Standard);
            let options = Options {
                tls_san: Vec::new(),
                tuning,
                own_network: false,
                disable: Vec::new(),
            };
            let file = rancher
                .server_files(&options)
                .into_iter()
                .find(|f| f.path.ends_with("/zeatend-servicelb-policy.yaml"))
                .unwrap();
            assert!(file.path.starts_with(rancher.manifests_dir()));
            assert_eq!(file.mode, 0o600);
            let policy: Value = serde_saphyr::from_str(&file.content).unwrap();
            assert_eq!(policy["kind"], "NetworkPolicy");
            assert_eq!(
                policy["spec"]["podSelector"]["matchExpressions"][0]["key"],
                rancher.servicelb_label()
            );
            assert_eq!(
                policy["metadata"]["labels"]["app.kubernetes.io/managed-by"],
                crate::plan::MANAGER
            );
            assert!(
                rancher
                    .agent_files(&tuning)
                    .iter()
                    .all(|f| !f.path.ends_with("/zeatend-servicelb-policy.yaml"))
            );
        }
    }

    #[test]
    fn the_minimum_holds_the_reserves_of_a_host_that_has_it() {
        for distribution in Rancher::ALL {
            let min = distribution.min_memory_mib();
            let reserves = HostTuning::new(distribution.into(), MemoryTier::of(min))
                .kubelet
                .unwrap();
            let kept = reserves.system + reserves.kube + reserves.eviction_hard;
            assert!(min > kept, "{distribution:?}");
            assert_eq!(min % 100, 0);
        }
        assert_eq!(Rancher::K3s.min_memory_mib(), 1900);
    }

    #[test]
    fn the_memory_threshold_keeps_the_disk_thresholds_k3s_sets() {
        for distribution in Rancher::ALL {
            for (tier, server) in [
                (MemoryTier::Small, true),
                (MemoryTier::Standard, true),
                (MemoryTier::Standard, false),
            ] {
                let args =
                    Rancher::kubelet_args(&HostTuning::new(distribution.into(), tier), server);
                let hard = args
                    .lines()
                    .find_map(|l| l.trim().strip_prefix("- eviction-hard="))
                    .unwrap();
                for threshold in [
                    "memory.available<",
                    "nodefs.available<5%",
                    "imagefs.available<5%",
                ] {
                    assert!(hard.contains(threshold), "{distribution:?}: {hard}");
                }
            }
        }
    }

    #[test]
    fn the_audit_policy_keeps_every_secret_request_first_and_never_bodies() {
        let policy: Value = serde_saphyr::from_str(AUDIT_POLICY).unwrap();
        let rules = policy["rules"].as_array().unwrap();
        assert_eq!(
            rules[0],
            serde_json::json!({"level": "Metadata",
                               "resources": [{"group": "", "resources": ["secrets"]}]})
        );
        assert!(
            rules
                .iter()
                .all(|r| ["None", "Metadata"].contains(&r["level"].as_str().unwrap()))
        );
    }

    #[test]
    fn join_files_name_the_server_and_the_node_addresses() {
        for d in Rancher::ALL {
            let join = d.join_file("https://10.0.0.5:6443");
            let value: Value = serde_saphyr::from_str(&join.content).unwrap();
            assert_eq!(value["server"], "https://10.0.0.5:6443");
            assert_eq!(value["token-file"], d.layout().join_token_file);
            assert_eq!(join.mode, 0o600);
            assert!(join.path.starts_with(d.layout().config_dir));
            let ip = d
                .node_ip_file(&["10.0.0.6".to_owned(), "fd00::6".to_owned()])
                .unwrap();
            let value: Value = serde_saphyr::from_str(&ip.content).unwrap();
            assert_eq!(value["node-ip"], serde_json::json!(["10.0.0.6", "fd00::6"]));
            assert_eq!(d.node_ip_file(&[]), None);
        }
    }

    #[test]
    fn only_rke2_listens_on_a_separate_supervisor_port() {
        let has = |d: Rancher, port| d.node_ports(true).iter().any(|(p, _)| *p == port);
        assert!(has(Rancher::Rke2, 9345));
        assert!(!has(Rancher::K3s, 9345));
        for d in Rancher::ALL {
            assert!(has(d, d.join_port()));
            assert!(!d.node_ports(false).iter().any(|(p, _)| *p == 6443));
        }
    }

    #[test]
    fn containerds_profile_keeps_its_name_and_lets_stacked_containers_signal_each_other() {
        let profile = apparmor_profile();
        let name = profile.path.rsplit('/').next().unwrap();
        assert!(
            profile
                .content
                .contains(&format!("\nprofile {name} flags="))
        );
        for rule in [
            "signal (send,receive)",
            "ptrace (trace,tracedby,read,readby)",
        ] {
            assert!(
                profile
                    .content
                    .contains(&format!("  {rule} peer={name}//&unconfined,\n")),
                "{rule}"
            );
        }
    }

    #[test]
    fn an_earlier_install_reads_back_as_the_components_it_turned_off() {
        use crate::config::Component;
        for distribution in [Distribution::K3s, Distribution::Rke2] {
            let packaged: Vec<Component> = Component::ALL
                .into_iter()
                .filter(|c| distribution.packaged(*c).is_some())
                .collect();
            let mut config = String::new();
            turn_off(&mut config, distribution, &packaged);
            assert_eq!(
                disabled_components(distribution, &config).unwrap(),
                packaged,
                "{distribution:?}"
            );
        }
        // What zeabur-installer v0.2.0 writes.
        assert_eq!(
            disabled_components(
                Distribution::K3s,
                "disable: traefik, local-storage, metrics-server\n"
            )
            .unwrap(),
            Component::ALL
        );
        assert_eq!(
            disabled_components(
                Distribution::Rke2,
                "ingress-controller: none\ndisable:\n  - rke2-metrics-server\n"
            )
            .unwrap(),
            [Component::Traefik, Component::MetricsServer]
        );
        assert_eq!(
            disabled_components(
                Distribution::K3s,
                "{}
"
            )
            .unwrap(),
            []
        );
        assert!(disabled_components(Distribution::K3s, "disable: [servicelb]\n").is_err());
        assert!(disabled_components(Distribution::Rke2, "disable: [traefik]\n").is_err());
        // zeatend writes the setting again and would deploy Traefik instead.
        assert!(
            disabled_components(Distribution::Rke2, "ingress-controller: ingress-nginx\n").is_err()
        );
    }

    #[test]
    fn left_to_remove_are_the_turned_off_manifests_addons_and_charts() {
        let addon = |name: &str, source: &str| serde_json::json!({"metadata": {"name": name}, "spec": {"source": source}});
        let chart = |name: &str, owner: &str| {
            serde_json::json!({"metadata": {
                "name": name,
                "annotations": {"objectset.rio.cattle.io/owner-name": owner},
            }})
        };
        let k3s = crate::k3s::MANIFESTS_DIR;
        let addons = serde_json::json!({"items": [
            addon("traefik", &format!("{k3s}/traefik.yaml")),
            addon("metrics-apiservice", &format!("{k3s}/metrics-server/metrics-apiservice.yaml")),
            addon("coredns", &format!("{k3s}/coredns.yaml")),
            addon("local-storage", &format!("{k3s}/local-storage.yaml")),
        ]});
        let charts = serde_json::json!({"items": [
            chart("traefik-crd", "traefik"),
            chart("gateway-api-crd", "gateway-api-crd"),
        ]});
        let disable = [Component::Traefik, Component::MetricsServer];
        assert_eq!(
            left_to_remove(Rancher::K3s, &disable, &addons, &charts, |_| false),
            [
                "addon/traefik",
                "addon/metrics-apiservice",
                "helmchart/traefik-crd"
            ]
        );
        assert_eq!(
            left_to_remove(Rancher::K3s, &[], &addons, &charts, |_| false),
            Vec::<String>::new()
        );
        let none = serde_json::json!({"items": []});
        assert_eq!(
            left_to_remove(Rancher::K3s, &disable, &none, &charts, |_| false),
            ["helmchart/traefik-crd"]
        );

        let rke2 = crate::rke2::MANIFESTS_DIR;
        let addons = serde_json::json!({"items": [
            addon("rke2-traefik", &format!("{rke2}/rke2-traefik.yaml")),
            addon("rke2-traefik-crd", &format!("{rke2}/rke2-traefik-crd.yaml")),
        ]});
        assert_eq!(
            left_to_remove(Rancher::Rke2, &[Component::Traefik], &addons, &none, |_| {
                true
            }),
            Vec::<String>::new()
        );
        let charts = serde_json::json!({"items": [chart("rke2-traefik", "rke2-traefik")]});
        assert_eq!(
            left_to_remove(
                Rancher::Rke2,
                &[Component::Traefik],
                &addons,
                &charts,
                |_| true
            ),
            ["helmchart/rke2-traefik"]
        );
    }
}
