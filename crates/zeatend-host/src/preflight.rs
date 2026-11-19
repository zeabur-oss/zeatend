//! Checks of the host: before `apply` changes anything, and by `doctor` on a host with a cluster.
//! The checks produce data; an interface words each [`Finding`].

use std::fs;
use std::net::TcpListener;
use std::path::Path;

use zeatend_plan::cluster::{PortUse, Rancher};
use zeatend_plan::finding::{BusyPort, Check, Finding, Leftover};
use zeatend_plan::kubesolo;

use crate::{cmd, systemd};

/// Whether a disk is encrypted, as far as the block devices tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encryption {
    /// A dm-crypt device is under it.
    Encrypted,
    /// No dm-crypt device is under it.
    Plain,
    /// The devices could not be read.
    Unknown,
}

/// What the checks need to know.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// The cluster from an earlier `apply` is running, so its ports are expected to be in use.
    pub cluster_running: bool,
    /// Fail on a disk that is not encrypted.
    pub require_encrypted_disk: bool,
    /// What this host runs.
    pub node: Node,
}

/// What a host runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Node {
    /// A server, which also listens on the API and join ports.
    Server(Rancher),
    /// A worker.
    Agent(Rancher),
    /// `KubeSolo`, one node.
    KubeSolo,
}

impl Node {
    /// The distribution id, as the `OTHERS` list names it.
    const fn kind(self) -> &'static str {
        match self {
            Self::Server(d) | Self::Agent(d) => d.id(),
            Self::KubeSolo => "kubesolo",
        }
    }

    const fn data_dir(self) -> &'static str {
        match self {
            Self::Server(_) | Self::Agent(_) => "/var/lib/rancher",
            Self::KubeSolo => kubesolo::DATA_DIR,
        }
    }

    /// Where the node keeps secrets or data: the root file system, the data directory, and the
    /// configuration with its tokens and kubeconfigs.
    fn secret_paths(self) -> Vec<&'static str> {
        let mut paths = vec!["/", self.data_dir()];
        match self {
            Self::Server(d) | Self::Agent(d) => paths.push(d.layout().config_dir),
            Self::KubeSolo => {
                paths.extend(
                    [kubesolo::CONFIG_FILE, kubesolo::KUBECONFIG]
                        .iter()
                        .filter_map(|f| Path::new(f).parent()?.to_str()),
                );
            }
        }
        paths
    }
}

/// Files left by Kubernetes distributions, and the id of the distribution that owns each, which
/// an install of the same distribution does not report.
const OTHERS: [(&str, &str, Option<&str>); 9] = [
    ("/usr/local/bin/k3s", "K3s", Some("k3s")),
    ("/etc/rancher/k3s", "K3s", Some("k3s")),
    ("/etc/kubernetes/admin.conf", "kubeadm", None),
    ("/var/lib/k0s", "k0s", None),
    ("/snap/bin/microk8s", "MicroK8s", None),
    ("/usr/local/bin/rke2", "RKE2", Some("rke2")),
    ("/etc/rancher/rke2", "RKE2", Some("rke2")),
    (kubesolo::BINARY, "KubeSolo", Some("kubesolo")),
    (kubesolo::DATA_DIR, "KubeSolo", Some("kubesolo")),
];
const DISK_NEEDED_GIB: u64 = 10;
const DISK_ROOMY_GIB: u64 = 30;

/// Runs every check.
#[must_use]
pub async fn run(options: Options) -> Vec<Check> {
    let mut checks = vec![
        root(),
        systemd(),
        time_sync().await,
        swap(),
        crash_kernel(),
        other_distributions(options.node),
    ];
    if options.node == Node::KubeSolo {
        checks.push(docker().await);
    }
    if !options.cluster_running {
        checks.push(ports(options.node));
    }
    checks.push(disk_space(options.node.data_dir()).await);
    checks.push(encryption(options.node, options.require_encrypted_disk).await);
    checks
}

/// Whether this process runs as root.
#[must_use]
pub fn is_root() -> bool {
    effective_uid(&fs::read_to_string("/proc/self/status").unwrap_or_default()) == Some(0)
}

fn root() -> Check {
    if is_root() {
        Finding::RootOk
    } else {
        Finding::RootNo
    }
    .into()
}

fn effective_uid(status: &str) -> Option<u32> {
    let line = status.lines().find(|l| l.starts_with("Uid:"))?;
    line.split_whitespace().nth(2)?.parse().ok()
}

fn systemd() -> Check {
    if Path::new("/run/systemd/system").is_dir() {
        Finding::SystemdOk
    } else {
        Finding::SystemdNo
    }
    .into()
}

async fn time_sync() -> Check {
    match cmd::run(
        "timedatectl",
        &["show", "--property=NTPSynchronized", "--value"],
        None,
    )
    .await
    {
        Ok(out) if out.trim() == "yes" => Finding::TimeOk,
        Ok(_) => Finding::TimeDrift,
        Err(_) => Finding::TimeUnknown,
    }
    .into()
}

fn swap() -> Check {
    match fs::read_to_string("/proc/swaps") {
        Ok(swaps) if swaps.lines().skip(1).any(|l| !l.trim().is_empty()) => Finding::SwapOn,
        Ok(_) => Finding::SwapOff,
        Err(_) => Finding::SwapUnknown,
    }
    .into()
}

fn crash_kernel() -> Check {
    crash_reserve(
        fs::read_to_string("/sys/kernel/kexec_crash_size")
            .ok()
            .as_deref(),
    )
    .into()
}

fn crash_reserve(raw: Option<&str>) -> Finding {
    let mib = raw
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .map_or(0, |bytes| bytes >> 20);
    if mib == 0 {
        Finding::CrashNone
    } else {
        Finding::CrashKept { mib }
    }
}

fn other_distributions(node: Node) -> Check {
    let found: Vec<Leftover> = OTHERS
        .iter()
        .filter(|(_, _, own)| own.is_none_or(|own| own != node.kind()))
        .filter(|(path, ..)| Path::new(path).exists())
        .map(|&(path, distribution, _)| Leftover { distribution, path })
        .collect();
    if found.is_empty() {
        Finding::OthersNone
    } else {
        Finding::OthersFound { found }
    }
    .into()
}

/// Where Docker's binary and socket are. Docker sets the iptables FORWARD policy to DROP, so
/// Pods of `KubeSolo` cannot reach the internet.
const DOCKER_PATHS: [&str; 5] = [
    "/usr/bin/docker",
    "/usr/local/bin/docker",
    "/snap/bin/docker",
    "/run/docker.sock",
    "/var/run/docker.sock",
];
const DOCKER_UNITS: [&str; 2] = ["docker.service", "docker.socket"];

async fn docker() -> Check {
    let mut found: Vec<String> = DOCKER_PATHS
        .iter()
        .filter(|path| Path::new(path).exists())
        .map(|&path| path.to_owned())
        .collect();
    for unit in DOCKER_UNITS {
        if systemd::is_active(unit).await {
            found.push(unit.to_owned());
        }
    }
    let iptables = cmd::run("iptables", &["-S", "FORWARD"], None)
        .await
        .unwrap_or_default();
    docker_check(found, &iptables).into()
}

fn docker_check(found: Vec<String>, iptables: &str) -> Finding {
    if !found.is_empty() {
        Finding::DockerFound { found }
    } else if iptables.lines().any(|l| l.trim() == "-P FORWARD DROP") {
        Finding::ForwardDrop
    } else {
        Finding::DockerNone
    }
}

/// The TCP ports a node needs free.
#[must_use]
fn needed_ports(node: Node) -> Vec<(u16, PortUse)> {
    match node {
        Node::Server(d) => d.node_ports(true),
        Node::Agent(d) => d.node_ports(false),
        Node::KubeSolo => kubesolo::ports(),
    }
}

fn ports(node: Node) -> Check {
    let ports = needed_ports(node);
    let busy: Vec<BusyPort> = ports
        .iter()
        .filter(|(port, _)| TcpListener::bind(("0.0.0.0", *port)).is_err())
        .map(|&(port, purpose)| BusyPort { port, purpose })
        .collect();
    if busy.is_empty() {
        Finding::PortsFree {
            ports: ports.iter().map(|(port, _)| *port).collect(),
        }
    } else {
        Finding::PortsBusy { ports: busy }
    }
    .into()
}

fn existing_or_var_lib(dir: &str) -> &str {
    if Path::new(dir).exists() {
        dir
    } else {
        "/var/lib"
    }
}

async fn disk_space(data_dir: &str) -> Check {
    let target = existing_or_var_lib(data_dir);
    let kib = cmd::run("df", &["-Pk", target], None)
        .await
        .ok()
        .and_then(|out| available_kib(&out));
    disk_finding(kib, target).into()
}

fn disk_finding(kib: Option<u64>, target: &str) -> Finding {
    let path = target.to_owned();
    let Some(kib) = kib else {
        return Finding::DiskUnknown { path };
    };
    let free_tenths_gib = tenths_gib(kib);
    if kib < DISK_NEEDED_GIB << 20 {
        Finding::DiskLow {
            free_tenths_gib,
            path,
            needed_gib: DISK_NEEDED_GIB,
        }
    } else if kib < DISK_ROOMY_GIB << 20 {
        Finding::DiskTight {
            free_tenths_gib,
            path,
            roomy_gib: DISK_ROOMY_GIB,
        }
    } else {
        Finding::DiskOk {
            free_tenths_gib,
            path,
        }
    }
}

/// KiB as tenths of a GiB, rounded down.
const fn tenths_gib(kib: u64) -> u64 {
    kib * 10 / (1 << 20)
}

fn available_kib(df: &str) -> Option<u64> {
    df.lines().nth(1)?.split_whitespace().nth(3)?.parse().ok()
}

/// The nearest of `path` and its parents that exists, so a directory the install has yet to make
/// is checked on the file system it will be on.
fn existing_ancestor(path: &str) -> &str {
    Path::new(path)
        .ancestors()
        .find(|p| p.exists())
        .and_then(Path::to_str)
        .unwrap_or("/")
}

/// The first path that is not encrypted, or else the first that could not be told, or else
/// `Encrypted`, naming the paths.
fn least_encrypted(found: &[(&'static str, Encryption)]) -> (Encryption, Vec<&'static str>) {
    for worst in [Encryption::Plain, Encryption::Unknown] {
        if let Some((path, _)) = found.iter().find(|(_, e)| *e == worst) {
            return (worst, vec![*path]);
        }
    }
    (
        Encryption::Encrypted,
        found.iter().map(|(path, _)| *path).collect(),
    )
}

/// Whether every file system `node` keeps secrets or data on is encrypted; each one is read once.
async fn encryption(node: Node, required: bool) -> Check {
    let mut sources: Vec<String> = Vec::new();
    let mut found = Vec::new();
    for path in node.secret_paths() {
        let Some(source) = mount_source(existing_ancestor(path)).await else {
            found.push((path, Encryption::Unknown));
            continue;
        };
        if !sources.contains(&source) {
            found.push((path, device_encryption(&source).await));
            sources.push(source);
        }
    }
    encryption_finding(&found, required).into()
}

fn encryption_finding(found: &[(&'static str, Encryption)], required: bool) -> Finding {
    let (found, paths) = least_encrypted(found);
    let path = paths.first().copied().unwrap_or("/");
    match (found, required) {
        (Encryption::Encrypted, _) => Finding::CryptYes { paths },
        (Encryption::Plain, true) => Finding::CryptRequiredPlain { path },
        (Encryption::Unknown, true) => Finding::CryptRequiredUnknown { path },
        (Encryption::Plain, false) => Finding::CryptNo { path },
        (Encryption::Unknown, false) => Finding::CryptUnknown { path },
    }
}

/// The device of the file system that holds `path`; `None` when it cannot be read.
async fn mount_source(path: &str) -> Option<String> {
    let source = cmd::run(
        "findmnt",
        &["--noheadings", "--output", "SOURCE", "--target", path],
        None,
    )
    .await
    .ok()?;
    let source = source.trim();
    // Btrfs subvolumes read as `/dev/sda2[/@]`.
    Some(source.split('[').next().unwrap_or(source).to_owned())
}

/// Whether `device` has a dm-crypt device beneath it.
async fn device_encryption(device: &str) -> Encryption {
    match cmd::run(
        "lsblk",
        &["--noheadings", "--inverse", "--output", "TYPE", device],
        None,
    )
    .await
    {
        Ok(types) if types.split_whitespace().any(|t| t == "crypt") => Encryption::Encrypted,
        Ok(_) => Encryption::Plain,
        Err(_) => Encryption::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use zeatend_plan::finding::Status;

    use super::*;

    #[test]
    fn docker_or_a_dropping_forward_policy_stops_the_install() {
        let docker = || vec!["/usr/bin/docker".to_owned()];
        let found = Finding::DockerFound { found: docker() };
        let policy = "-P FORWARD ACCEPT\n";
        let drop = "-P FORWARD DROP\n-A FORWARD -j DOCKER-USER\n";
        assert_eq!(docker_check(vec![], policy), Finding::DockerNone);
        assert_eq!(docker_check(vec![], ""), Finding::DockerNone);
        assert_eq!(docker_check(docker(), policy), found);
        assert_eq!(docker_check(vec![], drop), Finding::ForwardDrop);
        assert_eq!(docker_check(docker(), drop), found);
        assert_eq!(found.status(), Status::Fail);
        assert_eq!(Finding::ForwardDrop.status(), Status::Fail);
        assert_eq!(Finding::DockerNone.status(), Status::Pass);
    }

    #[test]
    fn parsing() {
        assert_eq!(effective_uid("Name:\tx\nUid:\t1000\t0\t0\t0\n"), Some(0));
        assert_eq!(effective_uid("Uid:\t0\t1000\t1000\t1000\n"), Some(1000));
        let df = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda2 61255492 9032 49094708 16% /\n";
        assert_eq!(available_kib(df), Some(49_094_708));
        assert_eq!(tenths_gib(3_250_586), 31);
        assert_eq!(tenths_gib(10 << 20), 100);
        assert_eq!(crash_reserve(None), Finding::CrashNone);
        assert_eq!(crash_reserve(Some("0\n")), Finding::CrashNone);
        assert_eq!(
            crash_reserve(Some("536870912\n")),
            Finding::CrashKept { mib: 512 }
        );
        assert_eq!(Finding::CrashKept { mib: 512 }.status(), Status::Warn);
        let ports = |node| -> Vec<u16> { needed_ports(node).iter().map(|(p, _)| *p).collect() };
        assert_eq!(ports(Node::Agent(Rancher::Rke2)), [80, 443, 10250]);
        assert_eq!(
            ports(Node::Server(Rancher::Rke2)),
            [80, 443, 6443, 9345, 10250]
        );
        assert_eq!(ports(Node::Server(Rancher::K3s)), [80, 443, 6443, 10250]);
        assert_eq!(ports(Node::KubeSolo), [80, 443, 6443, 10250, 10443]);
    }

    #[test]
    fn free_space_sets_the_outcome() {
        let path = || "/var/lib".to_owned();
        assert_eq!(
            disk_finding(None, "/var/lib"),
            Finding::DiskUnknown { path: path() }
        );
        assert_eq!(
            disk_finding(Some(3_250_586), "/var/lib"),
            Finding::DiskLow {
                free_tenths_gib: 31,
                path: path(),
                needed_gib: 10
            }
        );
        assert_eq!(
            disk_finding(Some(10 << 20), "/var/lib"),
            Finding::DiskTight {
                free_tenths_gib: 100,
                path: path(),
                roomy_gib: 30
            }
        );
        assert_eq!(
            disk_finding(Some(30 << 20), "/var/lib"),
            Finding::DiskOk {
                free_tenths_gib: 300,
                path: path()
            }
        );
    }

    #[test]
    fn encryption_names_the_paths_or_the_worst_one() {
        use Encryption::{Encrypted, Plain, Unknown};
        let both = [("/", Encrypted), ("/var/lib/rancher", Encrypted)];
        assert_eq!(
            encryption_finding(&both, true),
            Finding::CryptYes {
                paths: vec!["/", "/var/lib/rancher"]
            }
        );
        let plain = [("/", Encrypted), ("/var/lib/rancher", Plain)];
        assert_eq!(
            encryption_finding(&plain, true),
            Finding::CryptRequiredPlain {
                path: "/var/lib/rancher"
            }
        );
        assert_eq!(
            encryption_finding(&plain, false),
            Finding::CryptNo {
                path: "/var/lib/rancher"
            }
        );
        let unknown = [("/etc/rancher/rke2", Unknown)];
        assert_eq!(
            encryption_finding(&unknown, true),
            Finding::CryptRequiredUnknown {
                path: "/etc/rancher/rke2"
            }
        );
        assert_eq!(
            encryption_finding(&unknown, false),
            Finding::CryptUnknown {
                path: "/etc/rancher/rke2"
            }
        );
    }

    #[test]
    fn encryption_covers_where_each_node_keeps_secrets_and_data() {
        assert_eq!(
            Node::Server(Rancher::K3s).secret_paths(),
            ["/", "/var/lib/rancher", "/etc/rancher/k3s"]
        );
        assert_eq!(
            Node::Agent(Rancher::K3s).secret_paths(),
            ["/", "/var/lib/rancher", "/etc/rancher/k3s"]
        );
        for node in [Node::Server(Rancher::Rke2), Node::Agent(Rancher::Rke2)] {
            assert_eq!(
                node.secret_paths(),
                ["/", "/var/lib/rancher", "/etc/rancher/rke2"]
            );
        }
        assert_eq!(
            Node::KubeSolo.secret_paths(),
            ["/", "/var/lib/kubesolo", "/etc/kubesolo", "/etc/zeatend"]
        );
        assert_eq!(existing_ancestor("/no/such/dir/here"), "/");
    }

    #[test]
    fn one_plain_file_system_is_enough_to_fail() {
        use Encryption::{Encrypted, Plain, Unknown};
        assert_eq!(
            least_encrypted(&[("/", Encrypted), ("/var/lib/rancher", Encrypted)]),
            (Encrypted, vec!["/", "/var/lib/rancher"])
        );
        assert_eq!(
            least_encrypted(&[
                ("/", Encrypted),
                ("/etc/rancher/rke2", Unknown),
                ("/var/lib/rancher", Plain),
            ]),
            (Plain, vec!["/var/lib/rancher"])
        );
        assert_eq!(
            least_encrypted(&[("/", Encrypted), ("/etc/rancher/rke2", Unknown)]),
            (Unknown, vec!["/etc/rancher/rke2"])
        );
    }
}
