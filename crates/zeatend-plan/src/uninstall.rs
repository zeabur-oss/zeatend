//! What `uninstall` removes and keeps on a host, computed from which paths exist there.
//!
//! Each distribution's `footprint`, next to the code that plans its install, names what zeatend
//! and zeabur-installer leave on a node. A test checks that it holds every path an install
//! writes.

use std::net::Ipv4Addr;
use std::path::Path;

use crate::cluster::{Distribution, Rancher};
use crate::config::Role;
use crate::{installer, k3s, kubesolo, rke2};

/// The volumes of the local storage of Zeabur OSS.
pub const ZEABUR_STORAGE: &str = "/var/lib/zeabur/storage";
/// Longhorn's replicas.
pub const LONGHORN: &str = "/var/lib/longhorn";

/// K3s's and RKE2's pod network, their default `cluster-cidr`, which zeatend does not change.
pub const POD_CIDR: &str = "10.42.0.0/16";
const POD_NETWORK: (Ipv4Addr, u8) = (Ipv4Addr::new(10, 42, 0, 0), 16);

/// A cluster node on this host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node {
    /// Its distribution.
    pub distribution: Distribution,
    /// `KubeSolo` is a server.
    pub role: Role,
    /// It joined a cluster that another host runs.
    pub joined: bool,
}

impl Node {
    /// A node.
    #[must_use]
    pub const fn new(distribution: Distribution, role: Role, joined: bool) -> Self {
        Self {
            distribution,
            role,
            joined,
        }
    }

    /// Its name in the output.
    #[must_use]
    pub fn title(self) -> String {
        match (self.distribution, self.role) {
            (Distribution::KubeSolo, _) => "KubeSolo".to_owned(),
            (d, Role::Server) => format!("{} (server)", d.title()),
            (d, Role::Worker) => format!("{} (worker)", d.title()),
        }
    }

    /// The distribution's own uninstall, if it has one.
    #[must_use]
    pub const fn script(self) -> Option<&'static str> {
        self.distribution.layout().uninstall(self.role)
    }

    /// What the distribution's uninstall removes, in words.
    fn uninstall_text(self) -> String {
        let (layout, title) = (self.distribution.layout(), self.distribution.title());
        self.script().map_or_else(
            || {
                format!(
                    "zeatend stops {title} and its pods and removes its service, its binary, {}, \
                     {} and {}: the cluster's data, certificates and images.",
                    kubesolo::CONFIG_FILE,
                    kubesolo::CNI_CONFIG,
                    layout.data_dir,
                )
            },
            |script| {
                format!(
                    "{script} stops {title} and its pods and removes its service, its binary, {} \
                     and {}: the cluster's data, certificates and images on this host. zeatend \
                     then removes the routes into the pod network {POD_CIDR} that are left.",
                    layout.config_dir, layout.data_dir,
                )
            },
        )
    }

    /// The paths zeatend removes after the distribution's uninstall. Many are in directories
    /// that uninstall removes; zeatend removes them all the same, so a failed or partial
    /// uninstall leaves none.
    #[must_use]
    pub fn footprint(self) -> Vec<Remove> {
        match self.distribution {
            Distribution::K3s => k3s::footprint(self.role, self.joined),
            Distribution::Rke2 => rke2::footprint(self.role, self.joined),
            Distribution::KubeSolo => kubesolo::footprint(),
        }
    }

    /// Where volumes may be on this node.
    #[must_use]
    pub fn volumes(self) -> Vec<Volume> {
        // The uninstall removes the data directory around the local-path volumes.
        let stash = match self.distribution {
            Distribution::K3s => Some("/var/lib/rancher/.zeatend-kept-k3s-storage"),
            Distribution::KubeSolo => Some("/var/lib/.zeatend-kept-kubesolo-local-path-storage"),
            Distribution::Rke2 => None,
        };
        let local = self
            .distribution
            .layout()
            .local_path
            .zip(stash)
            .map(|(path, stash)| Volume {
                path,
                stash: Some(stash),
            });
        local
            .into_iter()
            .chain([ZEABUR_STORAGE, LONGHORN].map(|path| Volume { path, stash: None }))
            .collect()
    }
}

/// The directory that holds `path`.
#[must_use]
pub fn parent(path: &str) -> String {
    Path::new(path)
        .parent()
        .map_or_else(String::new, |p| p.display().to_string())
}

fn push_all<T: PartialEq>(list: &mut Vec<T>, items: impl IntoIterator<Item = T>) {
    for item in items {
        if !list.contains(&item) {
            list.push(item);
        }
    }
}

/// A path zeatend removes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remove {
    /// A file, or a directory with everything in it.
    Tree(String),
    /// A directory, only when it is empty: something else may keep files in it.
    EmptyDir(String),
}

impl Remove {
    /// The path.
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::Tree(path) | Self::EmptyDir(path) => path,
        }
    }
}

/// A directory that may hold volumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Volume {
    /// Where it is.
    pub path: &'static str,
    /// Where it waits, on the same file system, while the uninstall removes the directory around
    /// it. `None` when the uninstall does not touch it.
    pub stash: Option<&'static str>,
}

/// What `uninstall` does on a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uninstall {
    /// The nodes it removes.
    pub nodes: Vec<Node>,
    /// zeatend's paths, in the order to remove them.
    pub remove: Vec<Remove>,
    /// Where volumes may be.
    pub volumes: Vec<Volume>,
}

/// The nodes on a host where `exists` says which paths exist.
pub fn detect(exists: impl Fn(&str) -> bool) -> Vec<Node> {
    let mut nodes = Vec::new();
    // zeatend's join token, or zeabur-installer's on a node it joined.
    let joined = |rancher: Rancher| {
        exists(rancher.paths().join_token_file) || exists(&installer::join_token_file(rancher))
    };
    let k3s = Distribution::K3s.layout();
    let rke2 = Distribution::Rke2.layout();
    let kubesolo = Distribution::KubeSolo.layout();
    if k3s.server_uninstall.is_some_and(&exists) {
        // `join` takes no K3s server today; a server with a join token joined all the same.
        let joined = joined(Rancher::K3s);
        nodes.push(Node::new(Distribution::K3s, Role::Server, joined));
    }
    if k3s.agent_uninstall.is_some_and(&exists) {
        nodes.push(Node::new(Distribution::K3s, Role::Worker, true));
    }
    if rke2.server_uninstall.is_some_and(&exists) {
        // Only a server has its own token.
        let role = if exists(Rancher::Rke2.paths().server_token_file) {
            Role::Server
        } else {
            Role::Worker
        };
        let joined = role == Role::Worker || joined(Rancher::Rke2);
        nodes.push(Node::new(Distribution::Rke2, role, joined));
    }
    if exists(kubesolo.binary) || exists(kubesolo::UNIT_FILE) {
        nodes.push(Node::new(Distribution::KubeSolo, Role::Server, false));
    }
    nodes
}

/// Every kind of node, for a host where none is left: zeatend removes what any of them left.
const ALL_NODES: [Node; 5] = [
    Node::new(Distribution::K3s, Role::Server, false),
    Node::new(Distribution::K3s, Role::Worker, true),
    Node::new(Distribution::Rke2, Role::Server, true),
    Node::new(Distribution::Rke2, Role::Worker, true),
    Node::new(Distribution::KubeSolo, Role::Server, false),
];

/// What `uninstall` removes and keeps for `nodes`.
#[must_use]
pub fn plan(nodes: Vec<Node>) -> Uninstall {
    let sources = if nodes.is_empty() {
        ALL_NODES.to_vec()
    } else {
        nodes.clone()
    };
    let mut remove = Vec::new();
    let mut volumes = Vec::new();
    for node in &sources {
        push_all(&mut remove, node.footprint());
        push_all(&mut volumes, node.volumes());
    }
    // Files and trees first, then directories that only they kept from being empty.
    remove.sort_by_key(|r| matches!(r, Remove::EmptyDir(_)));
    Uninstall {
        nodes,
        remove,
        volumes,
    }
}

/// What `uninstall` shows before it asks: `remove` and `volumes` are the paths of the plan that
/// exist, and `configs` the configuration files that stay.
#[must_use]
pub fn describe(
    plan: &Uninstall,
    remove: &[&str],
    volumes: &[&str],
    delete_volumes: bool,
    configs: &[&str],
) -> String {
    let mut out = String::new();
    let mut line = |text: &str| {
        out.push_str(text);
        out.push('\n');
    };
    if plan.nodes.is_empty() {
        line("K3s, RKE2 and KubeSolo are not installed on this host.");
    } else {
        let titles: Vec<String> = plan.nodes.iter().map(|n| n.title()).collect();
        line(&format!(
            "zeatend uninstall removes {} from this host.",
            titles.join(" and ")
        ));
        for node in &plan.nodes {
            line(&format!("\n{}", node.uninstall_text()));
        }
    }
    if !remove.is_empty() {
        line("\nzeatend removes the files it left:");
        for path in remove {
            line(&format!("  {path}"));
        }
    }
    if delete_volumes && !volumes.is_empty() {
        line("\n--delete-volumes removes the volumes, and the data in them:");
        for path in volumes {
            line(&format!("  {path}"));
        }
    }
    let kept = kept(&plan.nodes, volumes, delete_volumes, configs);
    if !kept.is_empty() {
        line("\nThese stay:");
        for item in &kept {
            line(&format!("  {item}"));
        }
    }
    out
}

/// What stays on the host.
#[must_use]
pub fn kept(
    nodes: &[Node],
    volumes: &[&str],
    delete_volumes: bool,
    configs: &[&str],
) -> Vec<String> {
    let mut kept = Vec::new();
    if !delete_volumes {
        kept.extend(
            volumes
                .iter()
                .map(|path| format!("{path}  (volumes; --delete-volumes removes them)")),
        );
    }
    kept.extend(configs.iter().map(|&path| {
        let whose = if path == installer::CONFIG_FILE {
            "zeabur-installer's"
        } else {
            "zeatend's"
        };
        format!("{path}  ({whose} configuration)")
    }));
    if nodes
        .iter()
        .any(|n| (n.distribution, n.role) == (Distribution::Rke2, Role::Server))
    {
        kept.push(
            "the etcd user  (apply adds it when it is missing, so zeatend cannot tell whether \
             something else uses it)"
                .to_owned(),
        );
    }
    kept
}

/// What to run on a server after a node that joined a cluster left this host.
#[must_use]
pub fn leave_text(name: Option<&str>) -> String {
    format!(
        "This node was in a cluster that other hosts run. zeatend makes no changes through the \
         Kubernetes API, so the cluster still lists the node. On a server, remove it with:\n  \
         kubectl delete node {}",
        name.unwrap_or("<node name>")
    )
}

/// The route types `ip route` prints before the destination.
const ROUTE_TYPES: [&str; 10] = [
    "unicast",
    "local",
    "broadcast",
    "multicast",
    "throw",
    "unreachable",
    "prohibit",
    "blackhole",
    "nat",
    "anycast",
];

/// The routes into [`POD_CIDR`] in the output of `ip -4 route show`, each as the arguments of
/// `ip route del`.
///
/// The network plugin adds them, such as a blackhole route for the node's block of pod
/// addresses, and the distribution's uninstall can leave them. A later install on the host then
/// cannot reach its own pods.
#[must_use]
pub fn pod_routes(routes: &str) -> Vec<Vec<String>> {
    routes
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let first = words.next()?;
            let (kind, destination) = if ROUTE_TYPES.contains(&first) {
                (Some(first), words.next()?)
            } else {
                (None, first)
            };
            in_pod_network(destination).then(|| {
                ["route", "del"]
                    .into_iter()
                    .chain(kind)
                    .chain([destination])
                    .map(str::to_owned)
                    .collect()
            })
        })
        .collect()
}

/// Whether the IPv4 destination `a.b.c.d/len`, or `a.b.c.d` for one address, is in
/// [`POD_CIDR`].
fn in_pod_network(destination: &str) -> bool {
    let (address, len) = destination.split_once('/').unwrap_or((destination, "32"));
    let (Ok(address), Ok(len)) = (address.parse::<Ipv4Addr>(), len.parse::<u8>()) else {
        return false;
    };
    let (network, prefix) = POD_NETWORK;
    let mask = u32::MAX << (32 - prefix);
    len >= prefix && len <= 32 && u32::from(address) & mask == u32::from(network)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::APPARMOR_PROFILE;
    use crate::config::Document;
    use crate::join::{self, JoinFacts};
    use crate::plan::Facts;

    const K3S_UNINSTALL: &str = "/usr/local/bin/k3s-uninstall.sh";
    const K3S_AGENT_UNINSTALL: &str = "/usr/local/bin/k3s-agent-uninstall.sh";
    const RKE2_UNINSTALL: &str = "/usr/local/bin/rke2-uninstall.sh";
    const RKE2_KILLALL: &str = "/usr/local/bin/rke2-killall.sh";
    const K3S_LOCAL_PATH: &str = "/var/lib/rancher/k3s/storage";
    const RANCHER_STATE_DIR: &str = "/var/lib/rancher/zeatend";
    const KUBESOLO_BINARY: &str = "/usr/local/bin/kubesolo";
    const KUBESOLO_DATA_DIR: &str = "/var/lib/kubesolo";
    const KUBESOLO_STATE_DIR: &str = "/var/lib/zeatend";
    const KUBESOLO_LOCAL_PATH: &str = "/var/lib/kubesolo/local-path-storage";
    const ZEATEND_CONFIG: &str = "/etc/zeatend/config.yaml";

    const K3S_SERVER: Node = Node::new(Distribution::K3s, Role::Server, false);
    const K3S_WORKER: Node = Node::new(Distribution::K3s, Role::Worker, true);
    const RKE2_FIRST: Node = Node::new(Distribution::Rke2, Role::Server, false);
    const RKE2_JOINED: Node = Node::new(Distribution::Rke2, Role::Server, true);
    const RKE2_WORKER: Node = Node::new(Distribution::Rke2, Role::Worker, true);
    const KUBESOLO: Node = Node::new(Distribution::KubeSolo, Role::Server, false);

    fn tree_paths(node: Node) -> Vec<String> {
        node.footprint()
            .into_iter()
            .filter_map(|r| match r {
                Remove::Tree(path) => Some(path),
                Remove::EmptyDir(_) => None,
            })
            .collect()
    }

    /// Whether `uninstall` removes `path` of `node`: it is in the footprint or under a directory
    /// there.
    fn removed(node: Node, path: &str) -> bool {
        tree_paths(node)
            .iter()
            .any(|tree| path == tree || path.starts_with(&format!("{tree}/")))
    }

    fn facts(memory_mib: u64, variant: bool) -> Facts {
        Facts {
            arch: "amd64".to_owned(),
            hostname: "vps-1".to_owned(),
            node_name: None,
            host_hex: "a1b2c3".to_owned(),
            memory_mib,
            node_ips: if variant {
                Vec::new()
            } else {
                vec!["10.0.0.5".to_owned()]
            },
            record: None,
            kubernetes: None,
            apparmor_restricts_unconfined: variant,
            k3s_own_network: variant,
            installer_webhook: variant,
            longhorn: Some(crate::longhorn::HostState {
                package_manager: Some(crate::longhorn::PackageManager::Apt),
                multipathd_running: variant,
                ..crate::longhorn::HostState::default()
            }),
        }
    }

    /// What `apply` and `join` write on each kind of node, for every setting that changes which
    /// files they write.
    fn installs() -> Vec<(Node, Vec<String>)> {
        let mut installs = Vec::new();
        for distribution in ["k3s", "rke2", "kubesolo"] {
            for extra in [
                "",
                "disable: [traefik, metrics-server]\n",
                "disable: [traefik, metrics-server, local-storage]\n",
                "storage: {longhorn: true}\n",
                "apiServer:\n  authenticationConfig:\n    apiVersion: apiserver.config.k8s.io/v1\n    kind: AuthenticationConfiguration\n    jwt:\n      - issuer: {url: \"https://identity.example.com\", audiences: [kubernetes]}\n        claimMappings: {username: {claim: sub, prefix: \"zeabur:\"}}\n",
            ] {
                // Not every distribution takes every component.
                let Ok(Document::Install(config)) =
                    Document::parse(&format!("distribution: {distribution}\n{extra}"))
                else {
                    continue;
                };
                for memory_mib in [1400, 2048, 4096, 65536] {
                    for variant in [false, true] {
                        let Ok(plan) = crate::plan::plan(&config, &facts(memory_mib, variant))
                        else {
                            continue;
                        };
                        let node = Node::new(config.distribution, Role::Server, false);
                        let layout = config.distribution.layout();
                        let mut paths = plan.host_file_paths;
                        paths.extend(
                            plan.longhorn
                                .iter()
                                .flat_map(|p| &p.files)
                                .map(|f| f.path.clone()),
                        );
                        paths.push(format!("{}/record", layout.state_dir));
                        paths.push(format!("{}/downloads/v1/file", layout.state_dir));
                        match config.distribution {
                            Distribution::K3s => paths.push(k3s::AUDIT_LOG_DIR.to_owned()),
                            Distribution::KubeSolo => paths.extend(
                                [layout.binary, layout.kubectl]
                                    .into_iter()
                                    .chain(layout.kubeconfig)
                                    .map(str::to_owned),
                            ),
                            Distribution::Rke2 => {}
                        }
                        installs.push((node, paths));
                    }
                }
            }
        }
        for (distribution, role) in [("k3s", "worker"), ("rke2", "worker"), ("rke2", "server")] {
            let yaml = format!(
                "distribution: {distribution}\njoin:\n  server: https://10.0.0.5:9345\n  \
                 role: {role}\n  tokenFile: /etc/zeatend/join-token\nstorage: {{longhorn: true}}\n{}",
                if role == "server" {
                    "apiServer:\n  authenticationConfig:\n    apiVersion: apiserver.config.k8s.io/v1\n    kind: AuthenticationConfiguration\n    jwt:\n      - issuer: {url: \"https://identity.example.com\", audiences: [kubernetes]}\n        claimMappings: {username: {claim: sub, prefix: \"zeabur:\"}}\n"
                } else {
                    ""
                }
            );
            let Document::Join(config) = Document::parse(&yaml).unwrap() else {
                panic!("not a join");
            };
            for variant in [false, true] {
                let plan = join::plan(
                    &config,
                    &JoinFacts {
                        facts: facts(8192, variant),
                        rtt_us: vec![500],
                        ports: Vec::new(),
                        token_pinned: true,
                        unit_running: false,
                    },
                )
                .unwrap();
                let rancher = plan.cluster.distribution;
                let node = Node::new(rancher.into(), config.join.role, true);
                let mut paths = plan.host_file_paths;
                paths.extend(
                    plan.longhorn
                        .iter()
                        .flat_map(|p| &p.files)
                        .map(|f| f.path.clone()),
                );
                paths.push(rancher.paths().join_token_file.to_owned());
                paths.push(format!(
                    "{}/record",
                    rancher.distribution().layout().state_dir
                ));
                installs.push((node, paths));
            }
        }
        installs
    }

    #[test]
    fn uninstall_removes_every_path_an_install_writes() {
        let installs = installs();
        for node in [K3S_SERVER, K3S_WORKER, RKE2_FIRST, RKE2_WORKER, KUBESOLO] {
            let (role, d) = (node.role, node.distribution);
            assert!(
                installs
                    .iter()
                    .any(|(n, _)| (n.distribution, n.role) == (d, role)),
                "no install of {}",
                node.title()
            );
        }
        for (node, paths) in installs {
            for path in paths {
                assert!(removed(node, &path), "{}: {path}", node.title());
            }
        }
    }

    #[test]
    fn the_node_is_found_by_its_uninstall_script() {
        let host = |paths: &'static [&'static str]| detect(move |p| paths.contains(&p));
        assert_eq!(host(&[]), Vec::new());
        assert_eq!(host(&[K3S_UNINSTALL]), vec![K3S_SERVER]);
        assert_eq!(
            host(&[K3S_UNINSTALL, "/etc/rancher/k3s/zeatend-join-token"]),
            vec![Node::new(Distribution::K3s, Role::Server, true)]
        );
        assert_eq!(host(&[K3S_AGENT_UNINSTALL]), vec![K3S_WORKER]);
        assert_eq!(host(&[RKE2_UNINSTALL]), vec![RKE2_WORKER]);
        assert_eq!(
            host(&[RKE2_UNINSTALL, "/var/lib/rancher/rke2/server/token"]),
            vec![RKE2_FIRST]
        );
        assert_eq!(
            Rancher::Rke2.paths().server_token_file,
            "/var/lib/rancher/rke2/server/token"
        );
        assert_eq!(
            host(&[
                RKE2_UNINSTALL,
                "/var/lib/rancher/rke2/server/token",
                "/etc/rancher/rke2/zeatend-join-token"
            ]),
            vec![RKE2_JOINED]
        );
        assert_eq!(
            host(&[K3S_AGENT_UNINSTALL, "/etc/rancher/k3s/zeabur-join-token"]),
            vec![K3S_WORKER],
            "a node zeabur-installer joined"
        );
        assert_eq!(
            host(&[
                RKE2_UNINSTALL,
                "/var/lib/rancher/rke2/server/token",
                "/etc/rancher/rke2/zeabur-join-token"
            ]),
            vec![RKE2_JOINED]
        );
        assert_eq!(host(&[KUBESOLO_BINARY]), vec![KUBESOLO]);
        assert_eq!(host(&[kubesolo::UNIT_FILE]), vec![KUBESOLO]);
        // Volumes that an earlier uninstall kept are not an install.
        assert_eq!(host(&[KUBESOLO_DATA_DIR, K3S_LOCAL_PATH]), Vec::new());
    }

    #[test]
    fn the_scripts_are_where_the_releases_put_them() {
        let rke2 = Distribution::Rke2.layout();
        for script in [rke2.server_uninstall, rke2.agent_uninstall, rke2.killall] {
            assert!(script.is_some_and(|s| s.starts_with(rke2::PREFIX)));
        }
        assert_eq!(rke2.killall, Some(RKE2_KILLALL));
        assert_eq!(parent(K3S_UNINSTALL), parent(k3s::BINARY));
        assert_eq!(K3S_SERVER.script(), Some(K3S_UNINSTALL));
        assert_eq!(K3S_WORKER.script(), Some(K3S_AGENT_UNINSTALL));
        assert_eq!(RKE2_WORKER.script(), Some(RKE2_UNINSTALL));
        assert_eq!(KUBESOLO.script(), None);
        assert!(KUBESOLO_LOCAL_PATH.starts_with(KUBESOLO_DATA_DIR));
        assert_eq!(
            Distribution::KubeSolo.layout().local_path,
            Some(KUBESOLO_LOCAL_PATH)
        );
    }

    #[test]
    fn a_k3s_server_loses_zeatends_files_and_state_but_no_join_token() {
        let paths = tree_paths(K3S_SERVER);
        for path in [
            RANCHER_STATE_DIR,
            k3s::MEMORY_DROP_IN,
            k3s::AUDIT_LOG_DIR,
            APPARMOR_PROFILE,
        ] {
            assert!(paths.contains(&path.to_owned()), "{path}");
        }
        assert!(!paths.iter().any(|p| p.contains("join")), "{paths:?}");
        assert!(K3S_SERVER.footprint().contains(&Remove::EmptyDir(
            "/etc/systemd/system/k3s.service.d".to_owned()
        )));
    }

    #[test]
    fn a_worker_loses_its_join_token_and_no_server_files() {
        for worker in [K3S_WORKER, RKE2_WORKER] {
            let paths = tree_paths(worker);
            let config_dir = worker.distribution.layout().config_dir;
            assert!(paths.contains(&format!("{config_dir}/zeatend-join-token")));
            assert!(paths.contains(&format!("{config_dir}/config.yaml.d/10-join.yaml")));
            assert!(!paths.iter().any(|p| p.contains("manifests")), "{paths:?}");
            assert!(!paths.iter().any(|p| p.contains("audit")), "{paths:?}");
        }
        assert!(!tree_paths(RKE2_FIRST).iter().any(|p| p.contains("join")));
        assert!(!tree_paths(RKE2_WORKER).iter().any(|p| p.contains("k3s")));
    }

    #[test]
    fn what_zeabur_installer_left_is_removed_too() {
        assert!(tree_paths(K3S_SERVER).contains(&"/var/lib/rancher/zeabur-installer".to_owned()));
        assert!(
            tree_paths(RKE2_JOINED).contains(&"/etc/rancher/rke2/zeabur-join-token".to_owned())
        );
        let solo = tree_paths(KUBESOLO);
        for path in [
            "/var/lib/zeabur-installer",
            "/etc/zeabur-installer/kubeconfig.yaml",
            "/usr/local/libexec/zeabur-kubesolo-webhook",
        ] {
            assert!(solo.contains(&path.to_owned()), "{path}");
        }
        let text = describe(
            &plan(vec![KUBESOLO]),
            &[],
            &[],
            false,
            &[installer::CONFIG_FILE],
        );
        assert!(
            text.contains("/etc/zeabur-installer/config.yaml  (zeabur-installer's configuration)"),
            "{text}"
        );
    }

    #[test]
    fn kubesolo_is_removed_by_zeatend_itself() {
        let paths = tree_paths(KUBESOLO);
        for path in [
            kubesolo::UNIT_FILE,
            KUBESOLO_BINARY,
            KUBESOLO_DATA_DIR,
            kubesolo::CNI_CONFIG,
        ] {
            assert!(paths.contains(&path.to_owned()), "{path}");
        }
        assert!(!paths.contains(&RANCHER_STATE_DIR.to_owned()));
        // The person's configuration in /etc/zeatend stays; the directory goes only when empty.
        assert!(!removed(KUBESOLO, ZEATEND_CONFIG));
        assert!(
            KUBESOLO
                .footprint()
                .contains(&Remove::EmptyDir("/etc/zeatend".to_owned()))
        );
    }

    #[test]
    fn volumes_are_named_for_each_distribution() {
        let paths = |node: Node| node.volumes().iter().map(|v| v.path).collect::<Vec<_>>();
        assert_eq!(
            paths(K3S_SERVER),
            [K3S_LOCAL_PATH, ZEABUR_STORAGE, LONGHORN]
        );
        assert_eq!(paths(RKE2_WORKER), [ZEABUR_STORAGE, LONGHORN]);
        assert_eq!(
            paths(KUBESOLO),
            [KUBESOLO_LOCAL_PATH, ZEABUR_STORAGE, LONGHORN]
        );
        // Only the volumes inside a directory the uninstall removes move out of its way.
        for node in [K3S_SERVER, K3S_WORKER, RKE2_FIRST, KUBESOLO] {
            for volume in node.volumes() {
                assert_eq!(
                    volume.stash.is_some(),
                    [K3S_LOCAL_PATH, KUBESOLO_LOCAL_PATH].contains(&volume.path)
                );
            }
        }
    }

    #[test]
    fn a_host_without_a_node_is_checked_for_what_any_node_left() {
        let leftovers = plan(Vec::new());
        assert_eq!(leftovers.nodes, Vec::new());
        for path in [RANCHER_STATE_DIR, KUBESOLO_STATE_DIR] {
            assert!(leftovers.remove.contains(&Remove::Tree(path.to_owned())));
        }
        let paths: Vec<&str> = leftovers.volumes.iter().map(|v| v.path).collect();
        for path in [
            K3S_LOCAL_PATH,
            KUBESOLO_LOCAL_PATH,
            ZEABUR_STORAGE,
            LONGHORN,
        ] {
            assert!(paths.contains(&path));
        }
        let first_empty_dir = leftovers
            .remove
            .iter()
            .position(|r| matches!(r, Remove::EmptyDir(_)))
            .unwrap();
        assert!(
            leftovers.remove[first_empty_dir..]
                .iter()
                .all(|r| matches!(r, Remove::EmptyDir(_)))
        );
    }

    #[test]
    fn the_description_names_what_goes_and_what_stays() {
        let k3s = plan(vec![K3S_SERVER]);
        let text = describe(
            &k3s,
            &[RANCHER_STATE_DIR],
            &[K3S_LOCAL_PATH],
            false,
            &[ZEATEND_CONFIG],
        );
        assert!(
            text.contains("removes K3s (server) from this host"),
            "{text}"
        );
        assert!(text.contains(K3S_UNINSTALL), "{text}");
        assert!(text.contains(POD_CIDR), "{text}");
        assert!(text.contains(&format!("  {RANCHER_STATE_DIR}\n")), "{text}");
        assert!(
            text.contains(&format!(
                "These stay:\n  {K3S_LOCAL_PATH}  (volumes; --delete-volumes removes them)\n  \
                 {ZEATEND_CONFIG}  (zeatend's configuration)"
            )),
            "{text}"
        );
        let deleting = describe(&k3s, &[], &[K3S_LOCAL_PATH, LONGHORN], true, &[]);
        assert!(
            deleting.contains(&format!(
                "--delete-volumes removes the volumes, and the data in them:\n  {K3S_LOCAL_PATH}\n  {LONGHORN}\n"
            )),
            "{deleting}"
        );
        assert!(!deleting.contains("These stay"), "{deleting}");
        let rke2 = describe(&plan(vec![RKE2_FIRST]), &[], &[], false, &[]);
        assert!(rke2.contains("the etcd user"), "{rke2}");
        let solo = describe(&plan(vec![KUBESOLO]), &[], &[], false, &[]);
        assert!(solo.contains("zeatend stops KubeSolo"), "{solo}");
    }

    #[test]
    fn a_node_that_joined_is_deleted_from_a_server() {
        assert!(leave_text(Some("worker-1")).ends_with("kubectl delete node worker-1"));
        assert!(leave_text(None).ends_with("kubectl delete node <node name>"));
    }

    #[test]
    fn routes_into_the_pod_network_are_deleted_by_type_and_destination() {
        // A host after an uninstall of RKE2, with K3s's flannel running.
        let routes = "default via 192.168.1.1 dev eth0 proto dhcp src 192.168.1.20 metric 100\n\
                      blackhole 10.42.0.0/24 proto bird\n\
                      10.42.0.0/16 dev cni0 proto kernel scope link src 10.42.0.1\n\
                      10.42.1.0/24 via 10.42.1.0 dev flannel.1 onlink\n\
                      10.43.0.0/16 dev kube-ipvs0 scope link\n\
                      10.4.0.0/14 dev eth1 scope link\n\
                      10.0.0.0/8 dev eth2 scope link\n\
                      unreachable 10.42.7.9\n\
                      192.168.1.0/24 dev eth0 proto kernel scope link src 192.168.1.20\n";
        assert_eq!(
            pod_routes(routes),
            [
                vec!["route", "del", "blackhole", "10.42.0.0/24"],
                vec!["route", "del", "10.42.0.0/16"],
                vec!["route", "del", "10.42.1.0/24"],
                vec!["route", "del", "unreachable", "10.42.7.9"],
            ]
        );
        assert_eq!(pod_routes(""), Vec::<Vec<String>>::new());
        assert_eq!(
            POD_CIDR,
            format!("{}/{}", POD_NETWORK.0, POD_NETWORK.1),
            "the text and the network agree"
        );
    }
}
