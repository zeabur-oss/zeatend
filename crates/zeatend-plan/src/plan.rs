//! The install plan: everything `apply` will do, computed before the host is touched.
//!
//! The same configuration and host facts give the same plan, byte for byte, so `plan` can be
//! reviewed, diffed and tested.

use serde::{Deserialize, Serialize};

use crate::cluster::{self, Distribution, HostFile, Rancher};
use crate::config::{Config, Role};
use crate::finding::{Check, Finding};
use crate::pins::{self, K3S, K3s, KUBECTL, KUBESOLO, RKE2};
use crate::precondition::Preconditions;
use crate::record::{Changes, Record};
use crate::tuning::{HostTuning, MemoryTier};
use crate::upgrade::{self, Kubernetes};
use crate::{PlanError, kubesolo, node_name};

/// The label and field manager on everything zeatend creates.
pub const MANAGER: &str = "zeatend";

/// Facts about the host, read by `apply` and passed in so planning stays free of I/O.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Facts {
    /// `amd64` or `arm64`.
    pub arch: String,
    /// The host name; the default node name.
    pub hostname: String,
    /// The name an earlier install gave this node in Kubernetes; a re-run keeps it.
    pub node_name: Option<String>,
    /// Six hexadecimal digits from the machine id, for a node name when the host name will not do.
    pub host_hex: String,
    /// Memory in MiB.
    pub memory_mib: u64,
    /// The addresses the node registers with (`node-ip`); empty leaves the choice to the
    /// distribution.
    pub node_ips: Vec<String>,
    /// What an earlier `apply` or `join` recorded on the host.
    #[serde(skip)]
    pub record: Option<Record>,
    /// The Kubernetes version the cluster's API server reports, when the cluster answers.
    #[serde(skip)]
    pub kubernetes: Option<Kubernetes>,
    /// `kernel.apparmor_restrict_unprivileged_unconfined` is 1; see
    /// [`cluster::apparmor_profile`].
    #[serde(skip)]
    pub apparmor_restricts_unconfined: bool,
    /// The K3s configuration on the host turns flannel off: an earlier install gave the cluster a
    /// network plugin of its own, which zeatend keeps. See [`crate::k3s::runs_own_network`].
    #[serde(default)]
    pub k3s_own_network: bool,
    /// The script that zeabur-installer's `KubeSolo` unit runs after each start is on the host.
    /// See [`crate::kubesolo::Options::installer_webhook`].
    #[serde(default)]
    pub installer_webhook: bool,
}

impl Facts {
    /// The `AppArmor` profile containerd must find loaded on this host, if it needs one.
    #[must_use]
    pub fn apparmor_profile(&self) -> Option<HostFile> {
        self.apparmor_restricts_unconfined
            .then(cluster::apparmor_profile)
    }

    /// Stops when a newer zeatend applied, as the record says.
    ///
    /// # Errors
    ///
    /// [`PlanError::NewerZeatend`].
    pub fn check_zeatend_version(&self) -> Result<(), PlanError> {
        self.record
            .as_ref()
            .map_or(Ok(()), Record::check_zeatend_version)
    }
}

/// The whole plan of the first node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// Facts it was computed from.
    pub facts: Facts,
    /// The distribution.
    pub distribution: Distribution,
    /// The node's name in Kubernetes, written into the distribution's configuration.
    pub node_name: String,
    /// The release and its downloads.
    pub release: Release,
    /// The host's memory class.
    pub memory_tier: MemoryTier,
    /// What the host's memory class sets.
    pub tuning: HostTuning,
    /// Files written before the distribution starts.
    #[serde(skip)]
    pub host_files: Vec<HostFile>,
    /// Their paths, for the JSON form.
    pub host_file_paths: Vec<String>,
    /// What `apply` changes against the install record; `None` on a first install.
    pub changes: Option<Changes>,
    /// What to read before applying.
    pub findings: Vec<Check>,
    /// What the plan found on the host and what it writes; `apply` fills it.
    pub preconditions: Preconditions,
}

/// The release a plan installs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Release {
    /// K3s or RKE2.
    Cluster(ClusterPlan),
    /// `KubeSolo`.
    KubeSolo(KubeSoloPlan),
}

impl Release {
    /// The release tag.
    #[must_use]
    pub fn version(&self) -> &str {
        match self {
            Self::Cluster(cluster) => &cluster.version,
            Self::KubeSolo(solo) => &solo.version,
        }
    }

    /// The Kubernetes version it runs: from a K3s or RKE2 tag, or the minor `KubeSolo` ships.
    #[must_use]
    pub fn kubernetes(&self) -> Option<Kubernetes> {
        match self {
            Self::Cluster(cluster) => Kubernetes::parse(&cluster.version),
            Self::KubeSolo(solo) => Kubernetes::parse(&solo.kubernetes),
        }
    }
}

/// The K3s or RKE2 release and its downloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterPlan {
    /// K3s or RKE2.
    pub distribution: Rancher,
    /// Release tag.
    pub version: String,
    /// Whether the digests come from zeatend (`true`) or the release's own list.
    pub pinned: bool,
    /// The release's checksum list, read first when the release is not pinned.
    pub checksums: Option<Download>,
    /// K3s's binary or RKE2's tarball.
    pub release: Download,
    /// The image archives it imports on start.
    pub images: Vec<Download>,
    /// K3s's `install.sh`, always the pinned one: it writes the systemd unit and the uninstall
    /// script for any release.
    pub install_script: Option<Download>,
}

impl ClusterPlan {
    /// Every download, the checksum list first.
    pub fn downloads(&self) -> impl Iterator<Item = &Download> {
        self.checksums
            .iter()
            .chain(std::iter::once(&self.release))
            .chain(&self.images)
            .chain(&self.install_script)
    }
}

/// The `KubeSolo` release and its downloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KubeSoloPlan {
    /// Release tag.
    pub version: String,
    /// The Kubernetes minor it ships.
    pub kubernetes: String,
    /// The offline archive.
    pub download: Download,
    /// The kubectl zeatend uses; `KubeSolo` ships none.
    pub kubectl: Download,
}

/// One download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Download {
    /// File name.
    pub file: String,
    /// URL.
    pub url: String,
    /// Expected SHA-256, when pinned.
    pub sha256: Option<String>,
}

impl From<pins::Asset> for Download {
    fn from(asset: pins::Asset) -> Self {
        Self {
            file: asset.file,
            url: asset.url,
            sha256: Some(asset.sha256.to_owned()),
        }
    }
}

/// Computes the plan of the first node.
///
/// # Errors
///
/// When a newer zeatend applied, the architecture has no release, the node name cannot be used,
/// a first install of K3s or RKE2 does not fit in the host's memory, or the plan would downgrade
/// the release or skip a Kubernetes minor version (see [`upgrade`]).
pub fn plan(config: &Config, facts: &Facts) -> Result<Plan, PlanError> {
    facts.check_zeatend_version()?;
    let configured = config.host.node_name.as_deref();
    let node_name = if config.distribution == Distribution::KubeSolo {
        node_name::kubesolo(configured, facts)?
    } else {
        node_name::choose(configured, facts, &[])?
    };
    let mut findings = Vec::new();
    let memory_tier = MemoryTier::of(facts.memory_mib);
    let tuning = HostTuning::new(config.distribution, memory_tier);
    let (release, mut host_files) = if let Some(rancher) = config.distribution.rancher() {
        memory(rancher, facts, &mut findings)?;
        let cluster = cluster_plan(
            rancher,
            config.version.as_deref(),
            &facts.arch,
            &mut findings,
        )?;
        let mut files = rancher.server_files(&cluster::Options {
            tls_san: config.host.tls_san.clone(),
            tuning,
            own_network: rancher == Rancher::K3s && facts.k3s_own_network,
            disable: config.disable.clone(),
            api_server: config.api_server.clone(),
        })?;
        files.push(rancher.node_name_file(&node_name)?);
        files.extend(rancher.node_ip_file(&facts.node_ips)?);
        (Release::Cluster(cluster), files)
    } else {
        let arch = facts.arch.as_str();
        let no_release = || PlanError::Arch(arch.to_owned());
        if facts.memory_mib < kubesolo::MIN_MEMORY_MIB {
            findings.push(Finding::MemoryLow {
                have_mib: facts.memory_mib,
                min_mib: kubesolo::MIN_MEMORY_MIB,
                distribution: Distribution::KubeSolo,
            });
        }
        let solo = KubeSoloPlan {
            version: KUBESOLO.version.to_owned(),
            kubernetes: KUBESOLO.kubernetes.to_owned(),
            download: KUBESOLO.archive(arch).ok_or_else(no_release)?.into(),
            kubectl: KUBECTL.binary(arch).ok_or_else(no_release)?.into(),
        };
        let files = kubesolo::host_files(&kubesolo::Options {
            node_name: node_name.clone(),
            node_ip: facts.node_ips.first().cloned(),
            tls_san: config.host.tls_san.clone(),
            api_server: config.api_server.clone(),
            tuning,
            installer_webhook: facts.installer_webhook,
        })?;
        (Release::KubeSolo(solo), files)
    };
    host_files.extend(facts.apparmor_profile());
    upgrade::check_release(
        facts.record.as_ref(),
        config.distribution,
        release.version(),
    )?;
    if let Some(planned) = release.kubernetes() {
        upgrade::check_kubernetes(facts.kubernetes, planned, Role::Server, config.distribution)?;
    }
    let changes = facts
        .record
        .as_ref()
        .map(|r| Changes::new(r, release.version()));
    Ok(Plan {
        facts: facts.clone(),
        distribution: config.distribution,
        node_name,
        release,
        memory_tier,
        tuning,
        host_file_paths: host_files.iter().map(|f| f.path.clone()).collect(),
        host_files,
        changes,
        findings: findings.into_iter().map(Check::from).collect(),
        preconditions: Preconditions::default(),
    })
}

/// Stops a first install of K3s or RKE2 on a host with too little memory. A cluster that runs
/// already is upgraded with a warning instead.
fn memory(rancher: Rancher, facts: &Facts, findings: &mut Vec<Finding>) -> Result<(), PlanError> {
    let (have, min) = (facts.memory_mib, rancher.min_memory_mib());
    if have >= min {
        return Ok(());
    }
    if facts.record.is_none() {
        return Err(PlanError::Memory {
            distribution: rancher.into(),
            min,
            have,
        });
    }
    findings.push(Finding::MemoryLow {
        have_mib: have,
        min_mib: min,
        distribution: rancher.into(),
    });
    Ok(())
}

/// The release of `distribution` and its downloads: the pinned one, or `version` checked against
/// its own checksum list, with a warning.
pub(crate) fn cluster_plan(
    distribution: Rancher,
    version: Option<&str>,
    arch: &str,
    findings: &mut Vec<Finding>,
) -> Result<ClusterPlan, PlanError> {
    let unknown_arch = || PlanError::Arch(arch.to_owned());
    let pinned_version = distribution.distribution().pinned_version();
    let (version, pinned) = match version {
        Some(v) if v != pinned_version => {
            findings.push(Finding::ReleaseUnpinned {
                distribution,
                version: v.to_owned(),
            });
            (v.to_owned(), false)
        }
        Some(_) | None => (pinned_version.to_owned(), true),
    };
    let url: fn(&str, &str) -> String = match distribution {
        Rancher::K3s => pins::k3s_url,
        Rancher::Rke2 => pins::rke2_url,
    };
    let download = |file: String, sha: &str| Download {
        url: url(&version, &file),
        sha256: pinned.then(|| sha.to_owned()),
        file,
    };
    let (release, images, install_script) = match distribution {
        Rancher::K3s => (
            download(
                K3s::binary_file(arch),
                K3S.binary.get(arch).ok_or_else(unknown_arch)?,
            ),
            vec![download(
                format!("k3s-airgap-images-{arch}.tar.zst"),
                K3S.images.get(arch).ok_or_else(unknown_arch)?,
            )],
            Some(Download {
                file: "install.sh".to_owned(),
                url: pins::k3s_install_script_url(K3S.version),
                sha256: Some(K3S.install_script.to_owned()),
            }),
        ),
        Rancher::Rke2 => {
            let files = RKE2.files(arch).ok_or_else(unknown_arch)?;
            (
                download(format!("rke2.linux-{arch}.tar.gz"), files.tarball),
                vec![
                    download(
                        format!("rke2-images-core.linux-{arch}.tar.zst"),
                        files.images_core,
                    ),
                    download(
                        format!("rke2-images-canal.linux-{arch}.tar.zst"),
                        files.images_canal,
                    ),
                ],
                None,
            )
        }
    };
    let checksums = (!pinned).then(|| {
        let file = format!("sha256sum-{arch}.txt");
        Download {
            url: url(&version, &file),
            sha256: None,
            file,
        }
    });
    Ok(ClusterPlan {
        distribution,
        version,
        pinned,
        checksums,
        release,
        images,
        install_script,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::Topic;
    use crate::record::{self, Writer};

    fn config(yaml: &str) -> Config {
        Config::parse(yaml).unwrap()
    }

    fn facts() -> Facts {
        Facts {
            arch: "arm64".to_owned(),
            hostname: "vps-1".to_owned(),
            node_name: None,
            host_hex: "a1b2c3".to_owned(),
            memory_mib: 8192,
            node_ips: vec!["10.0.0.5".to_owned()],
            record: None,
            kubernetes: None,
            apparmor_restricts_unconfined: false,
            k3s_own_network: false,
            installer_webhook: false,
        }
    }

    fn topics(plan: &Plan) -> Vec<Topic> {
        plan.findings.iter().map(|c| c.name).collect()
    }

    #[test]
    fn a_host_last_applied_by_a_newer_zeatend_is_left_alone() {
        let newer = Facts {
            record: Some(Record {
                writer: Writer::Zeatend("99.0.0".to_owned()),
                distribution: None,
                version: None,
            }),
            ..facts()
        };
        assert!(matches!(
            plan(&config("{}"), &newer),
            Err(PlanError::NewerZeatend { recorded, .. }) if recorded == "99.0.0"
        ));
        let own = Facts {
            record: Some(Record::new(Distribution::K3s, "v1.0.0+k3s1")),
            ..facts()
        };
        plan(&config("{}"), &own).unwrap();
    }

    #[test]
    fn k3s_is_the_default_with_flannel_and_the_pinned_release() {
        let plan = plan(&config("{}"), &facts()).unwrap();
        assert_eq!(plan.distribution, Distribution::K3s);
        let Release::Cluster(cluster) = &plan.release else {
            panic!("not a cluster");
        };
        assert!(cluster.pinned);
        assert_eq!(cluster.distribution, Rancher::K3s);
        assert_eq!(cluster.version, K3S.version);
        assert!(cluster.install_script.is_some());
        assert_eq!(plan.memory_tier, MemoryTier::Standard);
        assert_eq!(
            plan.host_file_paths,
            [
                "/etc/sysctl.d/60-k3s-cis.conf",
                "/etc/rancher/k3s/audit-policy.yaml",
                "/etc/rancher/k3s/admission.yaml",
                "/etc/rancher/k3s/config.yaml",
                "/var/lib/rancher/k3s/server/manifests/zeabur-cis.yaml",
                "/etc/systemd/system/k3s.service.d/zeatend-memory.conf",
                "/var/lib/rancher/k3s/server/manifests/zeatend-servicelb-policy.yaml",
                "/etc/rancher/k3s/config.yaml.d/15-node-name.yaml",
                "/etc/rancher/k3s/config.yaml.d/20-node-ip.yaml",
            ]
        );
        assert!(
            plan.host_files
                .iter()
                .all(|f| !f.content.contains("flannel-backend"))
        );
        assert!(plan.findings.is_empty(), "{:?}", plan.findings);
        assert_eq!(plan.changes, None);
    }

    #[test]
    fn rke2_brings_canal_and_another_release_is_checked_against_its_own_list() {
        let plan = plan(
            &config("distribution: rke2\nversion: v1.37.1+rke2r1\n"),
            &facts(),
        )
        .unwrap();
        let Release::Cluster(cluster) = &plan.release else {
            panic!("not a cluster");
        };
        assert!(!cluster.pinned);
        assert_eq!(
            cluster.checksums.as_ref().map(|c| c.file.as_str()),
            Some("sha256sum-arm64.txt")
        );
        assert!(cluster.downloads().skip(1).all(|d| d.sha256.is_none()));
        assert_eq!(topics(&plan), [Topic::Release]);
    }

    #[test]
    fn kubesolo_gets_its_archive_kubectl_and_configuration() {
        let plan = plan(&config("distribution: kubesolo\n"), &facts()).unwrap();
        let Release::KubeSolo(solo) = &plan.release else {
            panic!("not KubeSolo");
        };
        assert_eq!(solo.version, KUBESOLO.version);
        assert!(solo.download.file.ends_with("-linux-arm64-offline.tar.gz"));
        assert_eq!(solo.kubectl.file, "kubectl");
        assert_eq!(
            plan.host_file_paths,
            [kubesolo::CONFIG_FILE, kubesolo::UNIT_FILE]
        );
        let small = Facts {
            memory_mib: 1024,
            ..facts()
        };
        let low = super::plan(&config("distribution: kubesolo\n"), &small).unwrap();
        assert_eq!(
            low.findings[0].finding,
            Finding::MemoryLow {
                have_mib: 1024,
                min_mib: kubesolo::MIN_MEMORY_MIB,
                distribution: Distribution::KubeSolo,
            }
        );
    }

    #[test]
    fn a_first_install_must_fit_and_an_upgrade_is_warned() {
        let small = Facts {
            memory_mib: 1800,
            ..facts()
        };
        let err = plan(&config("{}"), &small).unwrap_err();
        assert!(matches!(err, PlanError::Memory { .. }), "{err}");
        assert!(err.to_string().contains("distribution: kubesolo"), "{err}");
        let installed = Facts {
            record: Some(Record {
                writer: Writer::Zeatend(record::ZEATEND_VERSION.to_owned()),
                distribution: Some("k3s".to_owned()),
                version: Some("v1.0.0+k3s1".to_owned()),
            }),
            ..small
        };
        let upgrade = plan(&config("{}"), &installed).unwrap();
        assert_eq!(topics(&upgrade), [Topic::Memory]);
        assert!(upgrade.changes.is_some_and(|c| c.restart));
    }

    #[test]
    fn the_node_keeps_its_name_and_a_restricted_host_gets_containerds_profile() {
        let named = Facts {
            node_name: Some("db-1".to_owned()),
            apparmor_restricts_unconfined: true,
            ..facts()
        };
        // KubeSolo keeps the host name (node_name::kubesolo).
        let solo = Facts {
            hostname: "db-1".to_owned(),
            ..named.clone()
        };
        for (distribution, facts) in [("k3s", &named), ("rke2", &named), ("kubesolo", &solo)] {
            let plan = plan(&config(&format!("distribution: {distribution}\n")), facts).unwrap();
            assert_eq!(plan.node_name, "db-1");
            assert!(
                plan.host_files.contains(&cluster::apparmor_profile()),
                "{distribution}"
            );
        }
    }

    #[test]
    fn k3s_fits_on_2_gib_with_the_small_tier() {
        let two_gib = Facts {
            memory_mib: 1950,
            ..facts()
        };
        let plan = plan(&config("{}"), &two_gib).unwrap();
        assert_eq!(plan.memory_tier, MemoryTier::Small);
        assert_eq!(plan.tuning.gomemlimit_mib, Some(600));
        assert!(plan.findings.is_empty(), "{:?}", plan.findings);
        assert!(
            plan.host_file_paths
                .contains(&crate::k3s::MEMORY_DROP_IN.to_owned())
        );
    }

    #[test]
    fn an_existing_k3s_network_is_kept_and_rke2_ignores_it() {
        let own = Facts {
            k3s_own_network: true,
            ..facts()
        };
        let config_file = |plan: &Plan, path: &str| {
            plan.host_files
                .iter()
                .find(|f| f.path == path)
                .unwrap()
                .content
                .clone()
        };
        let k3s = plan(&config("{}"), &own).unwrap();
        assert!(config_file(&k3s, crate::k3s::CONFIG_FILE).contains("flannel-backend: none\n"));
        let rke2 = plan(&config("distribution: rke2\n"), &own).unwrap();
        assert!(!config_file(&rke2, "/etc/rancher/rke2/config.yaml").contains("flannel-backend"));
    }
}
