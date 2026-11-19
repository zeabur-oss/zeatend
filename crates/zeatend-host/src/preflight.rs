//! Checks of the host: before `apply` changes anything, and by `doctor` on a host with a cluster.
//! The checks produce data; an interface words a [`Finding`].

use std::fs;
use std::net::TcpListener;
use std::path::Path;

use serde::Serialize;
use zeatend_plan::cluster::{Distribution, PortUse, Rancher};
use zeatend_plan::kubesolo;

use crate::{cmd, systemd};

/// How a check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    /// Fine.
    Pass,
    /// Worth reading; does not stop `apply`. A warning in `doctor`.
    Warn,
    /// Stops `apply`. Critical in `doctor`.
    Fail,
}

/// What a check looks at, serialized as a camelCase key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Topic {
    /// The process runs as root.
    Root,
    /// systemd is the init system.
    Systemd,
    /// The clock is synchronised.
    Time,
    /// Swap is off.
    Swap,
    /// No memory is kept for a crash kernel.
    CrashKernel,
    /// No files of another Kubernetes distribution.
    OtherKubernetes,
    /// The node's ports are free.
    Ports,
    /// The data directory has room.
    DiskSpace,
    /// Secrets and data are on encrypted disks.
    DiskEncryption,
    /// No Docker, and the FORWARD policy lets Pods out.
    Docker,
    /// The service of K3s, RKE2 or `KubeSolo` runs.
    Service,
    /// The Kubernetes API answers.
    Api,
    /// The install record of zeatend.
    InstallRecord,
    /// The nodes are Ready.
    Nodes,
    /// Each kubelet keeps memory from pods for the OS.
    KubeletReserves,
    /// The API server stores Secrets encrypted.
    SecretsEncryption,
}

/// Files of another Kubernetes distribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Leftover {
    /// The distribution's name, such as `K3s`.
    pub distribution: &'static str,
    /// The file or directory.
    pub path: &'static str,
}

/// A port that is in use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BusyPort {
    /// The TCP port.
    pub port: u16,
    /// What the node needs it for.
    pub purpose: PortUse,
}

/// The memory a node's kubelet keeps from pods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Kept {
    /// The node.
    pub name: String,
    /// Its memory less what pods may take, in MiB.
    pub mib: u64,
}

/// How a check came out, with what it found. Serialized with the outcome's key as `code` and
/// the fields in camelCase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all_fields = "camelCase")]
pub enum Finding {
    /// The process runs as root.
    #[serde(rename = "root.ok")]
    RootOk,
    /// The process does not run as root.
    #[serde(rename = "root.no")]
    RootNo,
    /// systemd is the init system.
    #[serde(rename = "systemd.ok")]
    SystemdOk,
    /// The host does not run systemd.
    #[serde(rename = "systemd.no")]
    SystemdNo,
    /// The clock is synchronised.
    #[serde(rename = "time.ok")]
    TimeOk,
    /// The clock is not synchronised.
    #[serde(rename = "time.drift")]
    TimeDrift,
    /// `timedatectl` did not answer.
    #[serde(rename = "time.unknown")]
    TimeUnknown,
    /// Swap is off.
    #[serde(rename = "swap.off")]
    SwapOff,
    /// Swap is on.
    #[serde(rename = "swap.on")]
    SwapOn,
    /// `/proc/swaps` could not be read.
    #[serde(rename = "swap.unknown")]
    SwapUnknown,
    /// No memory is kept for a crash kernel.
    #[serde(rename = "crash.none")]
    CrashNone,
    /// The kernel keeps memory for a crash kernel.
    #[serde(rename = "crash.kept")]
    CrashKept {
        /// The memory kept, in MiB.
        mib: u64,
    },
    /// No files of another distribution.
    #[serde(rename = "others.none")]
    OthersNone,
    /// Files of other distributions.
    #[serde(rename = "others.found")]
    OthersFound {
        /// What was found, in the order of the list of known files.
        found: Vec<Leftover>,
    },
    /// Every port the node needs is free.
    #[serde(rename = "ports.free")]
    PortsFree {
        /// The ports.
        ports: Vec<u16>,
    },
    /// Some ports the node needs are in use.
    #[serde(rename = "ports.busy")]
    PortsBusy {
        /// The ports in use.
        ports: Vec<BusyPort>,
    },
    /// The free space could not be read.
    #[serde(rename = "disk.unknown")]
    DiskUnknown {
        /// Where it was read.
        path: String,
    },
    /// Less free space than the cluster needs.
    #[serde(rename = "disk.low")]
    DiskLow {
        /// The free space in tenths of a GiB, rounded down.
        free_tenths_gib: u64,
        /// Where it was read.
        path: String,
        /// The least the cluster needs, in GiB.
        needed_gib: u64,
    },
    /// Enough free space, but little room for growth.
    #[serde(rename = "disk.tight")]
    DiskTight {
        /// The free space in tenths of a GiB, rounded down.
        free_tenths_gib: u64,
        /// Where it was read.
        path: String,
        /// The free space that leaves room, in GiB.
        roomy_gib: u64,
    },
    /// Room to spare.
    #[serde(rename = "disk.ok")]
    DiskOk {
        /// The free space in tenths of a GiB, rounded down.
        free_tenths_gib: u64,
        /// Where it was read.
        path: String,
    },
    /// Every path is on a dm-crypt device.
    #[serde(rename = "crypt.yes")]
    CryptYes {
        /// The paths, one per file system.
        paths: Vec<&'static str>,
    },
    /// A path is not encrypted, and encryption is required.
    #[serde(rename = "crypt.requiredPlain")]
    CryptRequiredPlain {
        /// The path.
        path: &'static str,
    },
    /// Whether a path is encrypted could not be told, and encryption is required.
    #[serde(rename = "crypt.requiredUnknown")]
    CryptRequiredUnknown {
        /// The path.
        path: &'static str,
    },
    /// A path is not encrypted.
    #[serde(rename = "crypt.no")]
    CryptNo {
        /// The path.
        path: &'static str,
    },
    /// Whether a path is encrypted could not be told.
    #[serde(rename = "crypt.unknown")]
    CryptUnknown {
        /// The path.
        path: &'static str,
    },
    /// No Docker, and the FORWARD policy is not DROP.
    #[serde(rename = "docker.none")]
    DockerNone,
    /// Docker is on the host.
    #[serde(rename = "docker.found")]
    DockerFound {
        /// Its paths and active units.
        found: Vec<String>,
    },
    /// The iptables FORWARD policy is DROP.
    #[serde(rename = "docker.forwardDrop")]
    ForwardDrop,
    /// K3s, RKE2 and `KubeSolo` are not installed.
    #[serde(rename = "service.none")]
    NotInstalled,
    /// The service runs.
    #[serde(rename = "service.running")]
    ServiceRunning {
        /// The systemd unit.
        unit: &'static str,
    },
    /// The service does not run.
    #[serde(rename = "service.stopped")]
    ServiceStopped {
        /// The systemd unit.
        unit: &'static str,
    },
    /// The API server answers `/readyz`.
    #[serde(rename = "api.ready")]
    ApiReady,
    /// The API server does not answer `/readyz`.
    #[serde(rename = "api.down")]
    ApiDown {
        /// The systemd unit that runs it.
        unit: &'static str,
    },
    /// zeatend, or the zeabur-installer it takes over from, recorded the install.
    #[serde(rename = "record.ok")]
    RecordOk {
        /// The tool that wrote the record, such as `zeatend 0.1.0`.
        writer: String,
    },
    /// No install record could be read.
    #[serde(rename = "record.missing")]
    RecordMissing {
        /// Where the record is kept.
        path: String,
    },
    /// A newer zeatend wrote the record.
    #[serde(rename = "record.newer")]
    RecordNewer {
        /// The version of the zeatend that wrote it.
        zeatend_version: String,
    },
    /// Every node is Ready.
    #[serde(rename = "nodes.ready")]
    NodesReady {
        /// The nodes.
        names: Vec<String>,
    },
    /// Some nodes are not Ready.
    #[serde(rename = "nodes.notReady")]
    NodesNotReady {
        /// The nodes that are not Ready.
        names: Vec<String>,
        /// The command that shows why.
        command: String,
    },
    /// The cluster has no nodes.
    #[serde(rename = "nodes.none")]
    NodesNone {
        /// The service whose log shows why.
        unit: &'static str,
    },
    /// The nodes could not be read.
    #[serde(rename = "nodes.unknown")]
    NodesUnknown {
        /// Why.
        reason: String,
    },
    /// Every kubelet keeps at least the memory the OS needs from pods.
    #[serde(rename = "reserves.set")]
    ReservesSet {
        /// The nodes.
        names: Vec<String>,
    },
    /// Kubelets that keep less memory from pods than the OS needs: no `system-reserved` or
    /// `kube-reserved`, at most a hard eviction threshold.
    #[serde(rename = "reserves.low")]
    ReservesLow {
        /// The nodes, with what each keeps.
        nodes: Vec<Kept>,
        /// What the OS needs at idle, in MiB.
        needed_mib: u64,
        /// The cluster's distribution, which says how to set them.
        distribution: Distribution,
    },
    /// A node reports no memory that can be read.
    #[serde(rename = "reserves.unknown")]
    ReservesUnknown {
        /// The node.
        name: String,
    },
    /// The API server writes Secrets encrypted.
    #[serde(rename = "secrets.encrypted")]
    SecretsEncrypted {
        /// The provider it writes them with, such as `aescbc`.
        provider: String,
    },
    /// The API server writes Secrets as they are.
    #[serde(rename = "secrets.plain")]
    SecretsPlain {
        /// The distribution.
        distribution: Rancher,
    },
}

impl Finding {
    /// What the check looked at.
    #[must_use]
    pub const fn topic(&self) -> Topic {
        match self {
            Self::RootOk | Self::RootNo => Topic::Root,
            Self::SystemdOk | Self::SystemdNo => Topic::Systemd,
            Self::TimeOk | Self::TimeDrift | Self::TimeUnknown => Topic::Time,
            Self::SwapOff | Self::SwapOn | Self::SwapUnknown => Topic::Swap,
            Self::CrashNone | Self::CrashKept { .. } => Topic::CrashKernel,
            Self::OthersNone | Self::OthersFound { .. } => Topic::OtherKubernetes,
            Self::PortsFree { .. } | Self::PortsBusy { .. } => Topic::Ports,
            Self::DiskUnknown { .. }
            | Self::DiskLow { .. }
            | Self::DiskTight { .. }
            | Self::DiskOk { .. } => Topic::DiskSpace,
            Self::CryptYes { .. }
            | Self::CryptRequiredPlain { .. }
            | Self::CryptRequiredUnknown { .. }
            | Self::CryptNo { .. }
            | Self::CryptUnknown { .. } => Topic::DiskEncryption,
            Self::DockerNone | Self::DockerFound { .. } | Self::ForwardDrop => Topic::Docker,
            Self::NotInstalled | Self::ServiceRunning { .. } | Self::ServiceStopped { .. } => {
                Topic::Service
            }
            Self::ApiReady | Self::ApiDown { .. } => Topic::Api,
            Self::RecordOk { .. } | Self::RecordMissing { .. } | Self::RecordNewer { .. } => {
                Topic::InstallRecord
            }
            Self::NodesReady { .. }
            | Self::NodesNotReady { .. }
            | Self::NodesNone { .. }
            | Self::NodesUnknown { .. } => Topic::Nodes,
            Self::ReservesSet { .. } | Self::ReservesLow { .. } | Self::ReservesUnknown { .. } => {
                Topic::KubeletReserves
            }
            Self::SecretsEncrypted { .. } | Self::SecretsPlain { .. } => Topic::SecretsEncryption,
        }
    }

    /// Whether it passes, warns or stops `apply`.
    #[must_use]
    pub const fn status(&self) -> Status {
        match self {
            Self::RootOk
            | Self::SystemdOk
            | Self::TimeOk
            | Self::SwapOff
            | Self::CrashNone
            | Self::OthersNone
            | Self::PortsFree { .. }
            | Self::DiskOk { .. }
            | Self::CryptYes { .. }
            | Self::DockerNone
            | Self::ServiceRunning { .. }
            | Self::ApiReady
            | Self::RecordOk { .. }
            | Self::NodesReady { .. }
            | Self::ReservesSet { .. }
            | Self::SecretsEncrypted { .. } => Status::Pass,
            Self::TimeDrift
            | Self::TimeUnknown
            | Self::SwapOn
            | Self::SwapUnknown
            | Self::CrashKept { .. }
            | Self::OthersFound { .. }
            | Self::DiskUnknown { .. }
            | Self::DiskTight { .. }
            | Self::CryptNo { .. }
            | Self::CryptUnknown { .. }
            | Self::RecordMissing { .. }
            | Self::RecordNewer { .. }
            | Self::ReservesLow { .. }
            | Self::ReservesUnknown { .. } => Status::Warn,
            Self::RootNo
            | Self::SystemdNo
            | Self::PortsBusy { .. }
            | Self::DiskLow { .. }
            | Self::CryptRequiredPlain { .. }
            | Self::CryptRequiredUnknown { .. }
            | Self::DockerFound { .. }
            | Self::ForwardDrop
            | Self::NotInstalled
            | Self::ServiceStopped { .. }
            | Self::ApiDown { .. }
            | Self::NodesNotReady { .. }
            | Self::NodesNone { .. }
            | Self::NodesUnknown { .. }
            | Self::SecretsPlain { .. } => Status::Fail,
        }
    }
}

/// One check: what it looked at, how it came out and what it found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// What was checked.
    pub name: Topic,
    /// The outcome.
    pub status: Status,
    /// What was found.
    #[serde(flatten)]
    pub finding: Finding,
}

impl From<Finding> for Check {
    fn from(finding: Finding) -> Self {
        Self {
            name: finding.topic(),
            status: finding.status(),
            finding,
        }
    }
}

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
    fn a_check_serializes_its_finding_with_a_code_and_named_fields() {
        let check = |finding| serde_json::to_value(Check::from(finding)).unwrap();
        assert_eq!(
            check(Finding::DiskLow {
                free_tenths_gib: 31,
                path: "/var/lib".to_owned(),
                needed_gib: 10,
            }),
            serde_json::json!({
                "name": "diskSpace",
                "status": "fail",
                "code": "disk.low",
                "freeTenthsGib": 31,
                "path": "/var/lib",
                "neededGib": 10,
            })
        );
        assert_eq!(
            check(Finding::RootOk),
            serde_json::json!({"name": "root", "status": "pass", "code": "root.ok"})
        );
        assert_eq!(
            check(Finding::PortsBusy {
                ports: vec![BusyPort {
                    port: 6443,
                    purpose: PortUse::KubernetesApi
                }],
            }),
            serde_json::json!({
                "name": "ports",
                "status": "fail",
                "code": "ports.busy",
                "ports": [{"port": 6443, "purpose": "kubernetesApi"}],
            })
        );
        assert_eq!(
            check(Finding::OthersFound {
                found: vec![Leftover {
                    distribution: "K3s",
                    path: "/etc/rancher/k3s"
                }],
            }),
            serde_json::json!({
                "name": "otherKubernetes",
                "status": "warn",
                "code": "others.found",
                "found": [{"distribution": "K3s", "path": "/etc/rancher/k3s"}],
            })
        );
        assert_eq!(
            check(Finding::CryptRequiredPlain { path: "/" }),
            serde_json::json!({
                "name": "diskEncryption",
                "status": "fail",
                "code": "crypt.requiredPlain",
                "path": "/",
            })
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
