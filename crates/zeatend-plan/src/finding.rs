//! What a check of the host or a plan found: one shape for the host checks of `apply` and
//! `doctor` and for the warnings and blockers of a plan. An interface words a [`Finding`] from its
//! code and values.

use serde::Serialize;

use crate::cluster::{Distribution, PortUse, Rancher};
use crate::longhorn::Need;

/// How a finding weighs.
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

/// What a finding is about, serialized as a camelCase key.
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
    /// The host's memory against what the distribution needs.
    Memory,
    /// The release of K3s or RKE2.
    Release,
    /// The server a node joins, and how far it is.
    Server,
    /// The ports between the nodes.
    Firewall,
    /// How many servers the control plane has.
    HighAvailability,
    /// Where volumes are kept.
    Storage,
    /// The join token.
    JoinToken,
    /// The release downloads answer, before an install.
    Downloads,
    /// The registry mirrors answer.
    Mirrors,
    /// The node's name in Kubernetes.
    NodeName,
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

/// What a check of the host or a plan found. Serialized with the finding's key as `code` and
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
    /// A download of the release answers.
    #[serde(rename = "downloads.ok")]
    DownloadsOk {
        /// The download.
        url: String,
    },
    /// A download of the release does not answer, so `apply` cannot install it.
    #[serde(rename = "downloads.no")]
    DownloadsNo {
        /// The download.
        url: String,
        /// What failed: the connection, TLS, the proxy or an HTTP status.
        reason: String,
    },
    /// A registry mirror answers.
    #[serde(rename = "mirror.ok")]
    MirrorOk {
        /// The mirror endpoint.
        endpoint: String,
    },
    /// A registry mirror does not answer; containerd tries the next one, then the registry.
    #[serde(rename = "mirror.no")]
    MirrorNo {
        /// The mirror endpoint.
        endpoint: String,
        /// What failed: the connection, TLS or the proxy.
        reason: String,
    },
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
    /// The host has less memory than the distribution needs.
    #[serde(rename = "memory.low")]
    MemoryLow {
        /// The host's memory, in MiB.
        have_mib: u64,
        /// What the distribution needs, in MiB.
        min_mib: u64,
        /// The distribution.
        distribution: Distribution,
    },
    /// A release that this zeatend does not pin: its downloads are checked only against the
    /// release's own checksum list.
    #[serde(rename = "cluster.unsupported")]
    ReleaseUnpinned {
        /// K3s or RKE2.
        distribution: Rancher,
        /// The release tag.
        version: String,
    },
    /// The server's join port did not answer.
    #[serde(rename = "join.server.unreachable")]
    ServerUnreachable {
        /// The URL the node joins through.
        server: String,
        /// The join port.
        port: u16,
    },
    /// The server is farther than one region allows.
    #[serde(rename = "join.rtt.high")]
    ServerFar {
        /// The median round trip, in microseconds.
        median_us: u64,
        /// How many connections were timed.
        samples: usize,
        /// The port they were timed to.
        port: u16,
    },
    /// The server is so far that the cluster will be slow and unstable.
    #[serde(rename = "join.rtt.critical")]
    ServerTooFar {
        /// The median round trip, in microseconds.
        median_us: u64,
        /// How many connections were timed.
        samples: usize,
        /// The port they were timed to.
        port: u16,
    },
    /// Every node belongs at one provider in one region.
    #[serde(rename = "join.sameRegion")]
    SameRegion,
    /// Ports on the server besides the join port that did not answer.
    #[serde(rename = "join.port.closed")]
    ServerPortsClosed {
        /// The TCP ports.
        ports: Vec<u16>,
        /// The server's host.
        host: String,
    },
    /// The ports to allow between the nodes.
    #[serde(rename = "join.ports")]
    JoinPorts {
        /// K3s or RKE2.
        distribution: Rancher,
    },
    /// The UDP port between the nodes cannot be checked.
    #[serde(rename = "join.udp")]
    UdpUnchecked {
        /// The UDP port.
        port: u16,
    },
    /// A control plane survives losing a server only with three or more.
    #[serde(rename = "join.servers.ha")]
    ServersHa,
    /// Volumes on local-path storage stay on their node.
    #[serde(rename = "join.storage.local")]
    LocalStorage,
    /// The token has no CA hash, so the node trusts any server at the address.
    #[serde(rename = "join.token.unpinned")]
    TokenUnpinned,
    /// The node has what Longhorn needs.
    #[serde(rename = "longhorn.ready")]
    LonghornReady,
    /// The node lacks what Longhorn needs.
    #[serde(rename = "longhorn.missing")]
    LonghornMissing {
        /// What it lacks.
        needs: Vec<Need>,
    },
    /// The node name is `localhost`, which does not tell which machine a finding is about.
    #[serde(rename = "nodeName.localhost")]
    NodeNameLocalhost,
}

impl Finding {
    /// What it is about.
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
            Self::MemoryLow { .. } => Topic::Memory,
            Self::ReleaseUnpinned { .. } => Topic::Release,
            Self::ServerUnreachable { .. }
            | Self::ServerFar { .. }
            | Self::ServerTooFar { .. }
            | Self::SameRegion => Topic::Server,
            Self::ServerPortsClosed { .. } | Self::JoinPorts { .. } | Self::UdpUnchecked { .. } => {
                Topic::Firewall
            }
            Self::ServersHa => Topic::HighAvailability,
            Self::LocalStorage | Self::LonghornReady | Self::LonghornMissing { .. } => {
                Topic::Storage
            }
            Self::TokenUnpinned => Topic::JoinToken,
            Self::DownloadsOk { .. } | Self::DownloadsNo { .. } => Topic::Downloads,
            Self::MirrorOk { .. } | Self::MirrorNo { .. } => Topic::Mirrors,
            Self::NodeNameLocalhost => Topic::NodeName,
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
            | Self::SecretsEncrypted { .. }
            | Self::LonghornReady
            | Self::DownloadsOk { .. }
            | Self::MirrorOk { .. } => Status::Pass,
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
            | Self::ReservesUnknown { .. }
            | Self::MemoryLow { .. }
            | Self::ReleaseUnpinned { .. }
            | Self::ServerFar { .. }
            | Self::ServerTooFar { .. }
            | Self::SameRegion
            | Self::ServerPortsClosed { .. }
            | Self::JoinPorts { .. }
            | Self::UdpUnchecked { .. }
            | Self::ServersHa
            | Self::LocalStorage
            | Self::TokenUnpinned
            | Self::MirrorNo { .. }
            | Self::NodeNameLocalhost => Status::Warn,
            Self::RootNo
            | Self::SystemdNo
            | Self::PortsBusy { .. }
            | Self::DiskLow { .. }
            | Self::CryptRequiredPlain { .. }
            | Self::CryptRequiredUnknown { .. }
            | Self::DockerFound { .. }
            | Self::ForwardDrop
            | Self::NotInstalled
            | Self::DownloadsNo { .. }
            | Self::ServiceStopped { .. }
            | Self::ApiDown { .. }
            | Self::NodesNotReady { .. }
            | Self::NodesNone { .. }
            | Self::NodesUnknown { .. }
            | Self::SecretsPlain { .. }
            | Self::ServerUnreachable { .. }
            | Self::LonghornMissing { .. } => Status::Fail,
        }
    }
}

/// A finding with what it is about and how it weighs, as the JSON forms show it.
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
