#![doc = include_str!("../README.md")]

pub mod cluster;
pub mod config;
pub mod join;
pub mod k3s;
pub mod kubesolo;
pub mod node_name;
pub mod pins;
pub mod plan;
pub mod record;
pub mod rke2;
pub mod tuning;

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
}
