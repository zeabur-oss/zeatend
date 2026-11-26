//! The install plan: everything `apply` will do, computed before the host is touched.
//!
//! The same configuration and host facts give the same plan, byte for byte, so `plan` can be
//! reviewed, diffed and tested.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::cluster::{self, Distribution, HostFile, Rancher};
use crate::config::{Config, Role};
use crate::finding::{Check, Finding};
use crate::longhorn::{self, Storage};
use crate::pins::{self, K3S, K3s, KUBECTL, KUBESOLO, RKE2};
use crate::precondition::Preconditions;
use crate::record::{Changes, Record};
use crate::registry::{self, Password, Proxy};
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
    /// `fs.inotify.max_user_watches`; 0 when it could not be read. See
    /// [`cluster::inotify_sysctl`].
    #[serde(default)]
    pub inotify_max_user_watches: u64,
    /// The K3s configuration on the host turns flannel off: an earlier install gave the cluster a
    /// network plugin of its own, which zeatend keeps. See [`crate::k3s::runs_own_network`].
    #[serde(default)]
    pub k3s_own_network: bool,
    /// The script that zeabur-installer's `KubeSolo` unit runs after each start is on the host.
    /// See [`crate::kubesolo::Options::installer_webhook`].
    #[serde(default)]
    pub installer_webhook: bool,
    /// What the host has of what Longhorn needs; read only when `storage.longhorn` asks for it.
    #[serde(skip)]
    pub longhorn: Option<longhorn::HostState>,
    /// The files of registry and proxy settings that zeatend wrote on the host before, among
    /// [`crate::registry::optional_files`] and the `hosts.toml` files of `KubeSolo`.
    #[serde(default)]
    pub registry_files: Vec<String>,
    /// The password of each registry host with a `passwordFile`, read on the host.
    #[serde(skip)]
    pub registry_passwords: BTreeMap<String, Password>,
}

impl Facts {
    /// The `AppArmor` profile containerd must find loaded on this host, if it needs one.
    #[must_use]
    pub fn apparmor_profile(&self) -> Option<HostFile> {
        self.apparmor_restricts_unconfined
            .then(cluster::apparmor_profile)
    }

    /// The inotify limits for this host.
    #[must_use]
    pub fn inotify_sysctl(&self) -> HostFile {
        cluster::inotify_sysctl(self.inotify_max_user_watches)
    }

    /// What `apply` does for Longhorn, when `storage` asks for it and the host was read.
    ///
    /// # Errors
    ///
    /// [`PlanError::NoPackageManager`].
    pub fn longhorn(&self, storage: Storage) -> Result<Option<longhorn::Prep>, PlanError> {
        self.longhorn
            .as_ref()
            .filter(|_| storage.longhorn)
            .map(longhorn::prepare)
            .transpose()
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
    /// What `apply` does for Longhorn; `None` unless `storage.longhorn` is set.
    pub longhorn: Option<longhorn::Prep>,
    /// Files of registry and proxy settings that zeatend wrote before and removes now.
    pub removed_files: Vec<String>,
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
    /// The proxy the downloads go through; `None` leaves it to the environment.
    pub proxy: Option<Proxy>,
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
    /// The proxy the downloads go through; `None` leaves it to the environment.
    pub proxy: Option<Proxy>,
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
    let node_name = node_name::choose(configured, facts, &[])?;
    let mut findings: Vec<Finding> = node_name::localhost_note(&node_name, facts)
        .into_iter()
        .collect();
    let memory_tier = MemoryTier::of(facts.memory_mib);
    let tuning = HostTuning::new(config.distribution, memory_tier);
    let (release, mut host_files) = if let Some(rancher) = config.distribution.rancher() {
        memory(rancher, facts, &mut findings)?;
        let mut cluster = cluster_plan(
            rancher,
            config.version.as_deref(),
            &facts.arch,
            &mut findings,
        )?;
        cluster.proxy.clone_from(&config.proxy);
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
            proxy: config.proxy.clone(),
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
    host_files.extend(config.registry(&facts.registry_passwords).files(
        config.distribution,
        Role::Server,
        &facts.node_ips,
    )?);
    host_files.extend(facts.apparmor_profile());
    host_files.push(facts.inotify_sysctl());
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
    let longhorn = facts.longhorn(config.storage)?;
    Ok(Plan {
        facts: facts.clone(),
        distribution: config.distribution,
        node_name,
        release,
        memory_tier,
        tuning,
        host_file_paths: host_files.iter().map(|f| f.path.clone()).collect(),
        removed_files: registry::removed(&facts.registry_files, &host_files),
        host_files,
        longhorn,
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

/// The URLs of the release files `apply` downloads to install `distribution` on `arch`: the
/// pinned release, or `version` with its checksum list. None for an architecture zeatend does
/// not install.
#[must_use]
pub fn release_urls(distribution: Distribution, version: Option<&str>, arch: &str) -> Vec<String> {
    let Some(rancher) = distribution.rancher() else {
        return [KUBESOLO.archive(arch), KUBECTL.binary(arch)]
            .into_iter()
            .flatten()
            .map(|asset| asset.url)
            .collect();
    };
    cluster_plan(rancher, version, arch, &mut Vec::new()).map_or_else(
        |_| Vec::new(),
        |cluster| cluster.downloads().map(|d| d.url.clone()).collect(),
    )
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
        proxy: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::Topic;
    use crate::record::{self, Writer};

    #[test]
    fn the_release_urls_are_the_downloads_of_the_pinned_release() {
        let k3s = release_urls(Distribution::K3s, None, "arm64");
        assert!(k3s.iter().any(|url| url.ends_with("/k3s-arm64")), "{k3s:?}");
        assert!(
            k3s.iter().any(|url| url.ends_with("/install.sh")),
            "{k3s:?}"
        );
        let rke2 = release_urls(Distribution::Rke2, None, "amd64");
        assert!(
            rke2.iter().any(|url| url.contains("rke2-images-core")),
            "{rke2:?}"
        );
        let kubesolo = release_urls(Distribution::KubeSolo, None, "amd64");
        assert!(
            kubesolo
                .iter()
                .any(|url| url.starts_with("https://dl.k8s.io/")),
            "{kubesolo:?}"
        );
        for distribution in [
            Distribution::K3s,
            Distribution::Rke2,
            Distribution::KubeSolo,
        ] {
            assert!(
                release_urls(distribution, None, "riscv64").is_empty(),
                "{distribution:?}"
            );
        }
    }

    #[test]
    fn the_release_urls_follow_a_configured_version_and_its_checksum_list() {
        let k3s = release_urls(Distribution::K3s, Some("v1.36.0+k3s1"), "arm64");
        assert!(
            k3s.iter()
                .any(|url| url.contains("/v1.36.0%2Bk3s1/") && url.ends_with("/k3s-arm64")),
            "{k3s:?}"
        );
        assert!(k3s.iter().any(|url| url.contains("sha256sum")), "{k3s:?}");
    }

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
            inotify_max_user_watches: 0,
            k3s_own_network: false,
            installer_webhook: false,
            longhorn: None,
            registry_files: Vec::new(),
            registry_passwords: BTreeMap::new(),
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

    fn localhost_note(yaml: &str, facts: &Facts) -> Option<Finding> {
        plan(&config(yaml), facts)
            .unwrap()
            .findings
            .into_iter()
            .map(|c| c.finding)
            .find(|f| matches!(f, Finding::NodeNameLocalhost))
    }

    fn localhost() -> Facts {
        Facts {
            hostname: "localhost".to_owned(),
            ..facts()
        }
    }

    #[test]
    fn a_kubesolo_node_is_named_like_a_k3s_node() {
        let solo = config("distribution: kubesolo\n");
        let address = Facts {
            hostname: "ip-10-211-55-77".to_owned(),
            ..facts()
        };
        assert_eq!(plan(&solo, &address).unwrap().node_name, "node-a1b2c3");
        let named = config("distribution: kubesolo\nhost:\n  nodeName: db-1\n");
        assert_eq!(plan(&named, &facts()).unwrap().node_name, "db-1");
    }

    #[test]
    fn a_node_named_localhost_gets_a_note() {
        for distribution in ["k3s", "rke2", "kubesolo"] {
            let yaml = format!("distribution: {distribution}\n");
            assert_eq!(
                localhost_note(&yaml, &localhost()),
                Some(Finding::NodeNameLocalhost),
                "{distribution}"
            );
            let named = format!("{yaml}host:\n  nodeName: vps-1\n");
            assert_eq!(localhost_note(&named, &localhost()), None, "{distribution}");
            assert_eq!(localhost_note(&yaml, &facts()), None, "{distribution}");
        }
    }

    #[test]
    fn a_node_registered_as_localhost_gets_no_note() {
        let registered = Facts {
            node_name: Some("localhost".to_owned()),
            ..localhost()
        };
        assert_eq!(localhost_note("{}", &registered), None);
        assert_eq!(
            localhost_note("distribution: kubesolo\n", &registered),
            None
        );
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
                "/var/lib/rancher/k3s/agent/etc/kubelet.conf.d/50-zeatend.conf",
                "/etc/rancher/k3s/config.yaml.d/15-node-name.yaml",
                "/etc/rancher/k3s/config.yaml.d/20-node-ip.yaml",
                "/etc/sysctl.d/60-zeatend-inotify.conf",
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
    fn every_server_gets_the_inotify_limits() {
        for distribution in ["k3s", "rke2", "kubesolo"] {
            let plan = plan(
                &config(&format!("distribution: {distribution}\n")),
                &facts(),
            )
            .unwrap();
            let file = plan
                .host_files
                .iter()
                .find(|f| f.path == "/etc/sysctl.d/60-zeatend-inotify.conf")
                .unwrap_or_else(|| panic!("{distribution}"));
            assert!(
                file.content
                    .contains("fs.inotify.max_user_instances=1024\n"),
                "{distribution}"
            );
        }
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
            [
                kubesolo::CONFIG_FILE,
                kubesolo::UNIT_FILE,
                cluster::INOTIFY_SYSCTL_FILE
            ]
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

    fn bare_host() -> longhorn::HostState {
        longhorn::HostState {
            package_manager: Some(longhorn::PackageManager::Dnf),
            missing_packages: longhorn::PACKAGES.to_vec(),
            unloaded_modules: longhorn::MODULES.to_vec(),
            ..longhorn::HostState::default()
        }
    }

    #[test]
    fn storage_longhorn_lists_what_apply_installs_without_touching_the_cluster_files() {
        let read = Facts {
            longhorn: Some(bare_host()),
            ..facts()
        };
        let without = plan(&config("{}"), &read).unwrap();
        assert_eq!(without.longhorn, None);
        let with = plan(&config("storage: {longhorn: true}\n"), &read).unwrap();
        let prep = with.longhorn.as_ref().unwrap();
        assert_eq!(
            prep.steps[0],
            longhorn::Step::Install {
                manager: longhorn::PackageManager::Dnf,
                packages: vec!["iscsi-initiator-utils", "nfs-utils"],
            }
        );
        assert_eq!(with.host_files, without.host_files);
        let json = serde_json::to_value(&with).unwrap();
        assert_eq!(json["longhorn"]["steps"][0]["step"], "install");

        let no_manager = Facts {
            longhorn: Some(longhorn::HostState {
                package_manager: None,
                ..bare_host()
            }),
            ..facts()
        };
        assert!(matches!(
            plan(&config("storage: {longhorn: true}\n"), &no_manager),
            Err(PlanError::NoPackageManager { .. })
        ));
    }

    #[test]
    fn the_node_keeps_its_name_and_a_restricted_host_gets_containerds_profile() {
        let named = Facts {
            node_name: Some("db-1".to_owned()),
            apparmor_restricts_unconfined: true,
            inotify_max_user_watches: 0,
            ..facts()
        };
        for distribution in ["k3s", "rke2", "kubesolo"] {
            let plan = plan(&config(&format!("distribution: {distribution}\n")), &named).unwrap();
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
