#![doc = include_str!("../README.md")]

pub mod authentication;
pub mod cluster;
pub mod config;
pub mod finding;
pub mod installer;
pub mod join;
pub mod k3s;
pub mod kubesolo;
pub mod longhorn;
pub mod node_name;
pub mod pins;
pub mod plan;
pub mod ports;
pub mod precondition;
pub mod record;
pub mod rke2;
pub mod token;
pub mod tuning;
pub mod uninstall;
pub mod upgrade;

/// Why a plan cannot be made.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PlanError {
    /// The configuration file.
    #[error("configuration: {0}")]
    Config(String),
    /// The install record was written by a newer zeatend.
    #[error(
        "this host was installed with zeatend {recorded}, which is newer than this one ({own}). \
         Use zeatend {recorded} or newer: https://github.com/zeabur-oss/zeatend/releases"
    )]
    NewerZeatend {
        /// The recorded version.
        recorded: String,
        /// This zeatend's version.
        own: String,
    },
    /// The host's CPU architecture.
    #[error("there is no release for the {0} architecture; amd64 and arm64 are supported")]
    Arch(String),
    /// Too little memory for K3s or RKE2.
    #[error(
        "{} needs a host with at least {min} MiB of memory; this one has {have} MiB. For a host \
         with less memory, set `distribution: kubesolo`",
        .distribution.title()
    )]
    Memory {
        /// The distribution chosen.
        distribution: cluster::Distribution,
        /// What it needs.
        min: u64,
        /// What the host has.
        have: u64,
    },
    /// `storage.longhorn` needs packages, and the host has no package manager zeatend uses.
    #[error(
        "storage.longhorn: this host has neither apt-get nor dnf. Install {packages} with its \
         package manager, and run apply again"
    )]
    NoPackageManager {
        /// Each missing program and its package names.
        packages: String,
    },
    /// A host file that cannot be serialised.
    #[error("{path}: {reason}")]
    HostFile {
        /// The file.
        path: String,
        /// Why.
        reason: String,
    },
    /// An admin kubeconfig that cannot be used.
    #[error("kubeconfig: {0}")]
    Kubeconfig(String),
    /// A bootstrap token was given to a node that joins as a server.
    #[error(
        "configuration: join token: a bootstrap token (<id>.<secret>) joins workers only; a server \
         needs the cluster's server token"
    )]
    BootstrapTokenForServer,
    /// A second server was asked of K3s, which runs one.
    #[error(
        "configuration: join.role: server: K3s runs one server with SQLite and takes workers only. \
         For a control plane that survives losing a server, install RKE2 (`distribution: rke2`) \
         on three servers"
    )]
    SecondServer,
    /// An `apply` would install an older release than the host runs.
    #[error(
        "this host runs {title} {installed}, and the plan installs the older {planned}. zeatend \
         does not downgrade: use a zeatend whose release is {installed} or newer, or remove the \
         version from the configuration"
    )]
    Downgrade {
        /// The distribution's name.
        title: &'static str,
        /// The recorded release.
        installed: String,
        /// The planned release.
        planned: String,
    },
    /// An `apply` would move a server more than one Kubernetes minor version.
    #[error(
        "the cluster runs Kubernetes {running}, and the plan installs Kubernetes {planned}. \
         Kubernetes upgrades one minor version at a time \
         (https://kubernetes.io/releases/version-skew-policy/): {how}"
    )]
    SkipsMinor {
        /// What the API server runs.
        running: String,
        /// The planned minor.
        planned: String,
        /// The minor to apply first.
        next: String,
        /// How to apply it.
        how: String,
    },
    /// A worker would run a newer Kubernetes minor than the cluster's API server.
    #[error(
        "the cluster's API server runs Kubernetes {running}, and this worker would run \
         Kubernetes {planned}; a kubelet must not be newer than the API server. Upgrade the \
         servers first: run zeatend apply on each server, one at a time, then on the workers"
    )]
    WorkerAhead {
        /// What the API server runs.
        running: String,
        /// The planned minor.
        planned: String,
    },
}
