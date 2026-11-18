//! The install plan: everything `apply` will do, computed before the host is touched.
//!
//! The same configuration and host facts give the same plan, byte for byte, so `plan` can be
//! reviewed, diffed and tested.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cluster::{self, Distribution, HostFile, Rancher};
use crate::config::Config;
use crate::pins::{self, CALICO, K3S, K3s, KUBECTL, KUBESOLO, RKE2};
use crate::record::{Changes, Record};
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
    /// What an earlier `apply` recorded in the cluster.
    #[serde(skip)]
    pub record: Option<Record>,
    /// The version of the zeatend that last applied, as kept on the host.
    #[serde(skip)]
    pub zeatend_version: Option<String>,
    /// `kernel.apparmor_restrict_unprivileged_unconfined` is 1; see
    /// [`cluster::apparmor_profile`].
    #[serde(skip)]
    pub apparmor_restricts_unconfined: bool,
}

impl Facts {
    /// The `AppArmor` profile containerd must find loaded on this host, if it needs one.
    #[must_use]
    pub fn apparmor_profile(&self) -> Option<HostFile> {
        self.apparmor_restricts_unconfined
            .then(cluster::apparmor_profile)
    }

    /// The name this host has as a node: [`Self::node_name`], else the host name in lower case,
    /// as K3s, RKE2 and `KubeSolo` name a node they are not told the name of.
    #[must_use]
    pub fn own_node_name(&self) -> String {
        self.node_name
            .clone()
            .unwrap_or_else(|| self.hostname.to_ascii_lowercase())
    }

    /// Stops when a newer zeatend applied, as the record or the host says.
    ///
    /// # Errors
    ///
    /// [`PlanError::NewerZeatend`].
    pub fn check_zeatend_version(&self) -> Result<(), PlanError> {
        if let Some(record) = &self.record {
            record.check_zeatend_version()?;
        }
        self.zeatend_version
            .as_deref()
            .map_or(Ok(()), crate::record::check_zeatend_version)
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
    /// Calico's manifest, applied once K3s's API answers; `None` on RKE2, which brings Canal, and
    /// on `KubeSolo`.
    pub calico: Option<Download>,
    /// Files written before the distribution starts.
    #[serde(skip)]
    pub host_files: Vec<HostFile>,
    /// Their paths, for the JSON form.
    pub host_file_paths: Vec<String>,
    /// What `apply` changes against the install record; `None` on a first install.
    pub changes: Option<Changes>,
    /// Things to read before applying.
    pub warnings: Vec<Warning>,
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
}

/// Something to read before applying.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    /// A stable key, such as `memory.low`, for an interface that words it itself.
    pub code: &'static str,
    /// The values the text names, in order.
    pub values: Vec<String>,
    /// The warning in English.
    pub text: String,
}

pub(crate) fn warning(code: &'static str, values: &[&str], text: String) -> Warning {
    Warning {
        code,
        values: values.iter().map(|&v| v.to_owned()).collect(),
        text,
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
/// or a first install of K3s or RKE2 does not fit in the host's memory.
pub fn plan(config: &Config, facts: &Facts) -> Result<Plan, PlanError> {
    facts.check_zeatend_version()?;
    let node_name = node_name::choose(config.host.node_name.as_deref(), facts, &[])?;
    let mut warnings = Vec::new();
    let (release, calico, mut host_files) = if let Some(rancher) = config.distribution.rancher() {
        memory(rancher, facts, &mut warnings)?;
        let cluster = cluster_plan(rancher, config.version(), &facts.arch, &mut warnings)?;
        let mut files = rancher.server_files(&cluster::Options {
            tls_san: config.host.tls_san.clone(),
        });
        files.push(rancher.node_name_file(&node_name));
        files.extend(rancher.node_ip_file(&facts.node_ips));
        let calico = (rancher == Rancher::K3s).then(|| CALICO.manifest().into());
        (Release::Cluster(cluster), calico, files)
    } else {
        let arch = facts.arch.as_str();
        let no_release = || PlanError::Arch(arch.to_owned());
        if facts.memory_mib < kubesolo::MIN_MEMORY_MIB {
            warnings.push(warning(
                "memory.low",
                &[
                    &facts.memory_mib.to_string(),
                    &kubesolo::MIN_MEMORY_MIB.to_string(),
                    "KubeSolo",
                ],
                format!(
                    "This host has {} MiB of memory, and KubeSolo needs at least {} MiB.",
                    facts.memory_mib,
                    kubesolo::MIN_MEMORY_MIB
                ),
            ));
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
        });
        (Release::KubeSolo(solo), None, files)
    };
    host_files.extend(facts.apparmor_profile());
    let changes = facts
        .record
        .as_ref()
        .map(|r| Changes::new(r, release.version()));
    Ok(Plan {
        facts: facts.clone(),
        distribution: config.distribution,
        node_name,
        release,
        calico,
        host_file_paths: host_files.iter().map(|f| f.path.clone()).collect(),
        host_files,
        changes,
        warnings,
    })
}

/// Stops a first install of K3s or RKE2 on a host with too little memory. A cluster that runs
/// already is upgraded with a warning instead.
fn memory(rancher: Rancher, facts: &Facts, warnings: &mut Vec<Warning>) -> Result<(), PlanError> {
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
    let title = rancher.title();
    warnings.push(warning(
        "memory.low",
        &[&have.to_string(), &min.to_string(), title],
        format!(
            "This host has {have} MiB of memory, and {title} needs at least {min} MiB, so some \
             pods may not start."
        ),
    ));
    Ok(())
}

/// The release of `distribution` and its downloads: the pinned one, or `version` checked against
/// its own checksum list, with a warning.
pub(crate) fn cluster_plan(
    distribution: Rancher,
    version: Option<&str>,
    arch: &str,
    warnings: &mut Vec<Warning>,
) -> Result<ClusterPlan, PlanError> {
    let unknown_arch = || PlanError::Arch(arch.to_owned());
    let pinned_version = distribution.pinned_version();
    let (version, pinned) = match version {
        Some(v) if v != pinned_version => {
            let title = distribution.title();
            warnings.push(warning(
                "cluster.unsupported",
                &[title, v],
                format!(
                    "{title} {v} is not supported by this zeatend release: its downloads are \
                     checked only against the release's own checksum list."
                ),
            ));
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

/// The namespace where K3s and RKE2 run the `ServiceLB` pods.
pub const SERVICELB_NAMESPACE: &str = "kube-system";

/// The `NetworkPolicy` that lets outside traffic into the `ServiceLB` pods.
///
/// `kube-system/default-network-policy` lets in only pods of `kube-system`: RKE2's `cis` profile
/// adds it, and zeatend adds it on K3s. Without this policy, a `ServiceLB` pod on a worker gets no
/// outside requests. Each distribution labels the pods with its own prefix.
#[must_use]
pub fn servicelb_policy(distribution: Rancher) -> Value {
    json!({
        "apiVersion": "networking.k8s.io/v1",
        "kind": "NetworkPolicy",
        "metadata": {
            "name": "zeatend-servicelb",
            "namespace": SERVICELB_NAMESPACE,
            "labels": {"app.kubernetes.io/managed-by": MANAGER},
        },
        "spec": {
            "podSelector": {"matchExpressions": [
                {"key": distribution.servicelb_label(), "operator": "Exists"},
            ]},
            "policyTypes": ["Ingress"],
            "ingress": [{}],
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record;

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
            zeatend_version: None,
            apparmor_restricts_unconfined: false,
        }
    }

    fn codes(plan: &Plan) -> Vec<&str> {
        plan.warnings.iter().map(|w| w.code).collect()
    }

    #[test]
    fn a_stopped_cluster_last_applied_by_a_newer_zeatend_is_left_alone() {
        let stopped = Facts {
            zeatend_version: Some("99.0.0".to_owned()),
            ..facts()
        };
        assert!(matches!(
            plan(&config("{}"), &stopped),
            Err(PlanError::NewerZeatend { recorded, .. }) if recorded == "99.0.0"
        ));
        let own = Facts {
            zeatend_version: Some(record::ZEATEND_VERSION.to_owned()),
            ..facts()
        };
        plan(&config("{}"), &own).unwrap();
    }

    #[test]
    fn k3s_is_the_default_with_calico_and_the_pinned_release() {
        let plan = plan(&config("{}"), &facts()).unwrap();
        assert_eq!(plan.distribution, Distribution::K3s);
        let Release::Cluster(cluster) = &plan.release else {
            panic!("not a cluster");
        };
        assert!(cluster.pinned);
        assert_eq!(cluster.distribution, Rancher::K3s);
        assert_eq!(cluster.version, K3S.version);
        assert!(cluster.install_script.is_some());
        assert!(plan.calico.is_some());
        assert_eq!(
            plan.host_file_paths,
            [
                "/etc/sysctl.d/60-k3s-cis.conf",
                "/etc/rancher/k3s/audit-policy.yaml",
                "/etc/rancher/k3s/admission.yaml",
                "/etc/rancher/k3s/config.yaml",
                "/var/lib/rancher/k3s/server/manifests/zeabur-cis.yaml",
                "/etc/rancher/k3s/config.yaml.d/15-node-name.yaml",
                "/etc/rancher/k3s/config.yaml.d/20-node-ip.yaml",
            ]
        );
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
        assert_eq!(plan.changes, None);
    }

    #[test]
    fn rke2_brings_canal_and_another_release_is_checked_against_its_own_list() {
        let plan = plan(
            &config("distribution: rke2\nrke2Version: v1.37.1+rke2r1\n"),
            &facts(),
        )
        .unwrap();
        assert_eq!(plan.calico, None);
        let Release::Cluster(cluster) = &plan.release else {
            panic!("not a cluster");
        };
        assert!(!cluster.pinned);
        assert_eq!(
            cluster.checksums.as_ref().map(|c| c.file.as_str()),
            Some("sha256sum-arm64.txt")
        );
        assert!(cluster.downloads().skip(1).all(|d| d.sha256.is_none()));
        assert_eq!(codes(&plan), ["cluster.unsupported"]);
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
        assert_eq!(plan.calico, None);
        assert_eq!(
            plan.host_file_paths,
            [kubesolo::CONFIG_FILE, kubesolo::UNIT_FILE]
        );
        let small = Facts {
            memory_mib: 1024,
            ..facts()
        };
        let low = super::plan(&config("distribution: kubesolo\n"), &small).unwrap();
        assert_eq!(codes(&low), ["memory.low"]);
    }

    #[test]
    fn a_first_install_must_fit_and_an_upgrade_is_warned() {
        let small = Facts {
            memory_mib: 1024,
            ..facts()
        };
        let err = plan(&config("{}"), &small).unwrap_err();
        assert!(matches!(err, PlanError::Memory { .. }), "{err}");
        assert!(err.to_string().contains("distribution: kubesolo"), "{err}");
        let installed = Facts {
            record: Some(Record {
                zeatend_version: record::ZEATEND_VERSION.to_owned(),
                distribution: Some("k3s".to_owned()),
                version: Some("v1.0.0+k3s1".to_owned()),
            }),
            ..small
        };
        let upgrade = plan(&config("{}"), &installed).unwrap();
        assert_eq!(codes(&upgrade), ["memory.low"]);
        assert!(upgrade.changes.is_some_and(|c| c.restart));
    }

    #[test]
    fn the_node_keeps_its_name_and_a_restricted_host_gets_containerds_profile() {
        let named = Facts {
            node_name: Some("db-1".to_owned()),
            apparmor_restricts_unconfined: true,
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
    fn the_servicelb_policy_selects_the_distributions_pods() {
        for rancher in Rancher::ALL {
            let policy = servicelb_policy(rancher);
            assert_eq!(
                policy["spec"]["podSelector"]["matchExpressions"][0]["key"],
                rancher.servicelb_label()
            );
            assert_eq!(
                policy["metadata"]["labels"]["app.kubernetes.io/managed-by"],
                MANAGER
            );
        }
    }
}
