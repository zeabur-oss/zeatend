//! The steps of an install, in the order `apply` and `join` run them. `apply` announces each
//! step from these lists, and the manual install guide (`docs/get-started/manual-install.mdx`)
//! has one section per step.

use crate::cluster::Distribution;

/// One step of an install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The preflight checks; a failed one stops the install.
    CheckHost,
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

/// The steps of `KubeSolo`.
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
    #[must_use]
    pub const fn install(distribution: Distribution) -> &'static [Self] {
        match distribution {
            Distribution::K3s | Distribution::Rke2 => RANCHER_INSTALL,
            Distribution::KubeSolo => KUBESOLO_INSTALL,
        }
    }

    /// The steps of `apply` on a node that joins a cluster, in order.
    #[must_use]
    pub const fn join() -> &'static [Self] {
        JOIN
    }

    /// The step's name in the markers of the manual install guide.
    #[must_use]
    pub const fn marker(self) -> &'static str {
        match self {
            Self::CheckHost => "check-host",
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

    /// The manual install guide in each language.
    const GUIDES: [(&str, &str); 3] = [
        (
            "en",
            include_str!("../../../docs/get-started/manual-install.mdx"),
        ),
        (
            "zh-TW",
            include_str!("../../../docs/zh-TW/get-started/manual-install.mdx"),
        ),
        (
            "zh-CN",
            include_str!("../../../docs/zh-CN/get-started/manual-install.mdx"),
        ),
    ];

    /// The name of `distribution` in a marker, as in `distribution:` of the configuration.
    const fn name(distribution: Distribution) -> &'static str {
        match distribution {
            Distribution::K3s => "k3s",
            Distribution::Rke2 => "rke2",
            Distribution::KubeSolo => "kubesolo",
        }
    }

    /// The steps the guide has for `distribution`, from its markers in order. A marker is a line
    /// `{/* step: <marker> */}`, or `{/* step: <marker> <distribution>... */}` for a step that
    /// only those distributions have.
    fn guide_steps(guide: &str, distribution: Distribution) -> Vec<&str> {
        guide
            .lines()
            .filter_map(|line| line.trim().strip_prefix("{/* step: ")?.strip_suffix(" */}"))
            .filter_map(|marker| {
                let mut words = marker.split_whitespace();
                let step = words.next()?;
                let mut only = words.peekable();
                (only.peek().is_none() || only.any(|d| d == name(distribution))).then_some(step)
            })
            .collect()
    }

    #[test]
    fn the_manual_install_guide_has_a_section_per_step_in_order() {
        for (lang, guide) in GUIDES {
            for distribution in [
                Distribution::K3s,
                Distribution::Rke2,
                Distribution::KubeSolo,
            ] {
                let steps: Vec<&str> = Step::install(distribution)
                    .iter()
                    .map(|s| s.marker())
                    .collect();
                assert_eq!(
                    guide_steps(guide, distribution),
                    steps,
                    "{lang} guide, {distribution:?}"
                );
            }
        }
    }

    #[test]
    fn a_marker_with_distributions_counts_only_for_them() {
        let guide = "{/* step: check-host */}\n  {/* step: wait-removed k3s rke2 */}\n\
                     {/* step: wait-addons kubesolo */}\ntext {/* step: download */}\n";
        assert_eq!(
            guide_steps(guide, Distribution::Rke2),
            ["check-host", "wait-removed"]
        );
        assert_eq!(
            guide_steps(guide, Distribution::KubeSolo),
            ["check-host", "wait-addons"]
        );
    }
}
