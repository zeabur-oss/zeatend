//! The steps of an install, in the order `apply` and `join` run them. `apply` announces each
//! step from these lists.

use crate::cluster::Distribution;
use crate::telemetry;

/// One step of an install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The preflight checks; a failed one stops the install.
    CheckHost,
    /// What `storage.longhorn` needs: its packages, kernel modules, `iscsid` and the multipath
    /// blacklist.
    PrepareLonghorn,
    /// The release, each file checked against its SHA-256: K3s's binary, `install.sh` and image
    /// archive, RKE2's tarball and image archives, or `KubeSolo`'s archive and kubectl.
    Download,
    /// The configuration files, then `systemctl daemon-reload` when one changed.
    WriteFiles,
    /// The `AppArmor` profile when the plan has one, and on K3s and RKE2 the kernel parameters,
    /// the audit log directory of a K3s server and the `etcd` user of an RKE2 server.
    PrepareHost,
    /// The binary: K3s's with its `install.sh`, which writes the unit, RKE2's tarball, or
    /// `KubeSolo`'s binary.
    InstallRelease,
    /// Enables and starts the unit, or restarts it when something changed.
    StartService,
    /// Waits until the API server answers and the node is Ready.
    WaitReady,
    /// Waits until the server removes the components that `disable` turns off.
    WaitRemoved,
    /// Makes the certificates in the server's `tls` directory readable by root only.
    ProtectCertificates,
    /// Waits until `KubeSolo`'s `CoreDNS` and local-path provisioner run.
    WaitAddons,
    /// Writes the install record that the next `plan` reads.
    WriteRecord,
    /// Writes the kubeconfig that names the node, from `KubeSolo`'s admin kubeconfig.
    WriteKubeconfig,
}

/// The steps of the first server of K3s or RKE2.
const RANCHER_INSTALL: &[Step] = &[
    Step::CheckHost,
    Step::PrepareLonghorn,
    Step::Download,
    Step::WriteFiles,
    Step::PrepareHost,
    Step::InstallRelease,
    Step::StartService,
    Step::WaitReady,
    Step::WaitRemoved,
    Step::ProtectCertificates,
    Step::WriteRecord,
];

/// The steps of `KubeSolo`, which has no `storage.longhorn`.
const KUBESOLO_INSTALL: &[Step] = &[
    Step::CheckHost,
    Step::Download,
    Step::WriteFiles,
    Step::PrepareHost,
    Step::InstallRelease,
    Step::StartService,
    Step::WaitReady,
    Step::WaitAddons,
    Step::WriteRecord,
    Step::WriteKubeconfig,
];

/// The steps of a node that joins a K3s or RKE2 cluster.
const JOIN: &[Step] = &[
    Step::CheckHost,
    Step::PrepareLonghorn,
    Step::Download,
    Step::WriteFiles,
    Step::PrepareHost,
    Step::InstallRelease,
    Step::StartService,
    Step::WaitReady,
    Step::WriteRecord,
];

impl Step {
    /// The steps of `apply` on the first node of `distribution`, in order.
    /// [`Self::PrepareLonghorn`] is there only with `longhorn`.
    #[must_use]
    pub fn install(distribution: Distribution, longhorn: bool) -> Vec<Self> {
        let list = match distribution {
            Distribution::K3s | Distribution::Rke2 => RANCHER_INSTALL,
            Distribution::KubeSolo => KUBESOLO_INSTALL,
        };
        Self::keep(list, longhorn)
    }

    /// The steps of `apply` on a node that joins a cluster, in order. [`Self::PrepareLonghorn`]
    /// is there only with `longhorn`.
    #[must_use]
    pub fn join(longhorn: bool) -> Vec<Self> {
        Self::keep(JOIN, longhorn)
    }

    /// `list` without [`Self::PrepareLonghorn`] unless `longhorn`.
    fn keep(list: &[Self], longhorn: bool) -> Vec<Self> {
        list.iter()
            .copied()
            .filter(|step| longhorn || *step != Self::PrepareLonghorn)
            .collect()
    }

    /// The step of the experience program that a failure in this step reports; `None` before
    /// `apply` starts to change the host.
    #[must_use]
    pub const fn telemetry(self) -> Option<telemetry::Step> {
        match self {
            Self::CheckHost => None,
            Self::PrepareLonghorn => Some(telemetry::Step::Longhorn),
            Self::Download
            | Self::WriteFiles
            | Self::PrepareHost
            | Self::InstallRelease
            | Self::StartService => Some(telemetry::Step::Install),
            Self::WaitReady
            | Self::WaitRemoved
            | Self::ProtectCertificates
            | Self::WaitAddons
            | Self::WriteRecord
            | Self::WriteKubeconfig => Some(telemetry::Step::Node),
        }
    }

    /// The step's name in error messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::CheckHost => "check-host",
            Self::PrepareLonghorn => "prepare-longhorn",
            Self::Download => "download",
            Self::WriteFiles => "write-files",
            Self::PrepareHost => "prepare-host",
            Self::InstallRelease => "install-release",
            Self::StartService => "start-service",
            Self::WaitReady => "wait-ready",
            Self::WaitRemoved => "wait-removed",
            Self::ProtectCertificates => "protect-certificates",
            Self::WaitAddons => "wait-addons",
            Self::WriteRecord => "write-record",
            Self::WriteKubeconfig => "write-kubeconfig",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_node_with_storage_longhorn_prepares_for_longhorn() {
        for distribution in [Distribution::K3s, Distribution::Rke2] {
            assert!(Step::install(distribution, true).contains(&Step::PrepareLonghorn));
            assert!(!Step::install(distribution, false).contains(&Step::PrepareLonghorn));
        }
        assert!(Step::join(true).contains(&Step::PrepareLonghorn));
        assert!(!Step::join(false).contains(&Step::PrepareLonghorn));
    }
}
