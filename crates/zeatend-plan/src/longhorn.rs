//! What a node needs before Longhorn runs on it. The Zeabur catalog installs Longhorn; zeatend
//! prepares the host only.
//!
//! Longhorn attaches each volume over iSCSI from the node itself: every node needs `open-iscsi`
//! with `iscsid` running, and `iscsi_tcp` loaded before `iscsid` starts. `ReadWriteMany` volumes
//! need an `NFSv4` client
//! (<https://longhorn.io/docs/1.13.0/deploy/install/#installation-requirements>). Encrypted
//! volumes use `dm_crypt`. multipathd claims Longhorn's disks unless a blacklist keeps it off
//! them (<https://longhorn.io/kb/troubleshooting-volume-with-multipath/>).

use serde::{Deserialize, Serialize};

use crate::PlanError;
use crate::cluster::HostFile;

/// Loads the modules at boot (`modules-load.d(5)`).
pub const MODULES_FILE: &str = "/etc/modules-load.d/zeatend-longhorn.conf";
/// Keeps multipathd off Longhorn's disks; multipathd reads `/etc/multipath/conf.d` by default
/// (`multipath.conf(5)`, `config_dir`).
pub const MULTIPATH_FILE: &str = "/etc/multipath/conf.d/zeatend-longhorn.conf";
/// The kernel modules Longhorn needs.
pub const MODULES: [&str; 2] = ["iscsi_tcp", "dm_crypt"];
/// The iSCSI daemon.
pub const ISCSID: &str = "iscsid";
/// The multipath daemon.
pub const MULTIPATHD: &str = "multipathd";

/// Longhorn's disks report this SCSI vendor and product. Blacklisting them alone leaves every
/// other disk to multipathd, unlike the `devnode "^sd[a-z0-9]+"` of Longhorn's knowledge base.
const MULTIPATH_BLACKLIST: &str = "\
# Written by zeatend: Longhorn's iSCSI disks are not multipath devices.
blacklist {
    device {
        vendor \"IET\"
        product \"VIRTUAL-DISK\"
    }
}
";

/// The `storage` section of the configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Storage {
    /// Prepare the node for Longhorn.
    #[serde(default)]
    pub longhorn: bool,
}

impl Storage {
    /// Nothing is set.
    #[must_use]
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    pub const fn is_empty(&self) -> bool {
        !self.longhorn
    }
}

/// A package manager zeatend installs packages with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PackageManager {
    /// Debian and Ubuntu.
    Apt,
    /// RHEL and its rebuilds, and Fedora.
    Dnf,
}

impl PackageManager {
    /// Every one, in the order zeatend looks for them.
    pub const ALL: [Self; 2] = [Self::Apt, Self::Dnf];

    /// The program, which tells that the host has it.
    #[must_use]
    pub const fn program(self) -> &'static str {
        match self {
            Self::Apt => "apt-get",
            Self::Dnf => "dnf",
        }
    }
}

/// A package a node needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    /// A program it installs; when it is there, so is the package.
    pub program: &'static str,
    /// Its name for apt.
    pub apt: &'static str,
    /// Its name for dnf.
    pub dnf: &'static str,
}

impl Package {
    /// Its name for `manager`.
    #[must_use]
    pub const fn name(&self, manager: PackageManager) -> &'static str {
        match manager {
            PackageManager::Apt => self.apt,
            PackageManager::Dnf => self.dnf,
        }
    }
}

/// The iSCSI initiator and the NFS client.
pub const PACKAGES: [Package; 2] = [
    Package {
        program: "iscsiadm",
        apt: "open-iscsi",
        dnf: "iscsi-initiator-utils",
    },
    Package {
        program: "mount.nfs",
        apt: "nfs-common",
        dnf: "nfs-utils",
    },
];

/// What the host has, read by the caller so planning stays free of I/O.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostState {
    /// The first of [`PackageManager::ALL`] on the host.
    pub package_manager: Option<PackageManager>,
    /// The packages whose program is missing.
    pub missing_packages: Vec<Package>,
    /// The modules of [`MODULES`] that are not loaded.
    pub unloaded_modules: Vec<&'static str>,
    /// `iscsid` is enabled and active.
    pub iscsid_running: bool,
    /// `multipathd` is active.
    pub multipathd_running: bool,
    /// The content of [`MODULES_FILE`], when there is one.
    pub modules_file: Option<String>,
    /// The content of [`MULTIPATH_FILE`], when there is one.
    pub multipath_file: Option<String>,
}

/// Something the node lacks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "need",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Need {
    /// A package.
    Package {
        /// The program it installs.
        program: &'static str,
        /// Its name for apt.
        apt: &'static str,
        /// Its name for dnf.
        dnf: &'static str,
    },
    /// [`MODULES_FILE`] is missing or differs.
    ModulesFile,
    /// A module is not loaded.
    Module {
        /// The module.
        module: &'static str,
    },
    /// `iscsid` is not enabled and active.
    Iscsid,
    /// multipathd runs, and [`MULTIPATH_FILE`] is missing or differs.
    MultipathBlacklist,
}

/// One change `apply` makes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "step",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Step {
    /// Install packages.
    Install {
        /// The package manager.
        manager: PackageManager,
        /// The packages, by its names.
        packages: Vec<&'static str>,
    },
    /// Write [`MODULES_FILE`].
    WriteModulesFile,
    /// Load modules now.
    LoadModules {
        /// The modules.
        modules: Vec<&'static str>,
    },
    /// Enable and start `iscsid`.
    StartIscsid,
    /// Write [`MULTIPATH_FILE`] and restart multipathd.
    BlacklistMultipath,
}

/// What `apply` does on a node for Longhorn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prep {
    /// The changes, in the order `apply` makes them; empty when the node is ready.
    pub steps: Vec<Step>,
    /// The files it keeps on the host.
    #[serde(skip)]
    pub files: Vec<HostFile>,
}

/// [`MODULES_FILE`].
fn modules_file() -> HostFile {
    HostFile {
        path: MODULES_FILE.to_owned(),
        mode: 0o644,
        content: MODULES.iter().flat_map(|m| [*m, "\n"]).collect(),
    }
}

/// [`MULTIPATH_FILE`].
fn multipath_file() -> HostFile {
    HostFile {
        path: MULTIPATH_FILE.to_owned(),
        mode: 0o644,
        content: MULTIPATH_BLACKLIST.to_owned(),
    }
}

/// What the node lacks.
#[must_use]
pub fn needs(host: &HostState) -> Vec<Need> {
    let mut needs: Vec<Need> = host
        .missing_packages
        .iter()
        .map(|p| Need::Package {
            program: p.program,
            apt: p.apt,
            dnf: p.dnf,
        })
        .collect();
    if host.modules_file.as_deref() != Some(modules_file().content.as_str()) {
        needs.push(Need::ModulesFile);
    }
    needs.extend(
        host.unloaded_modules
            .iter()
            .map(|&module| Need::Module { module }),
    );
    if !host.iscsid_running {
        needs.push(Need::Iscsid);
    }
    if host.multipathd_running && host.multipath_file.as_deref() != Some(MULTIPATH_BLACKLIST) {
        needs.push(Need::MultipathBlacklist);
    }
    needs
}

/// The changes that give the node what it lacks.
///
/// # Errors
///
/// [`PlanError::NoPackageManager`] when a package is missing and the host has neither apt nor
/// dnf.
pub fn prepare(host: &HostState) -> Result<Prep, PlanError> {
    let mut steps = Vec::new();
    if !host.missing_packages.is_empty() {
        let manager = host
            .package_manager
            .ok_or_else(|| PlanError::NoPackageManager {
                packages: host
                    .missing_packages
                    .iter()
                    .map(|p| format!("{} ({} or {})", p.program, p.apt, p.dnf))
                    .collect::<Vec<_>>()
                    .join(", "),
            })?;
        steps.push(Step::Install {
            manager,
            packages: host
                .missing_packages
                .iter()
                .map(|p| p.name(manager))
                .collect(),
        });
    }
    let needs = needs(host);
    if needs.contains(&Need::ModulesFile) {
        steps.push(Step::WriteModulesFile);
    }
    if !host.unloaded_modules.is_empty() {
        steps.push(Step::LoadModules {
            modules: host.unloaded_modules.clone(),
        });
    }
    if needs.contains(&Need::Iscsid) {
        steps.push(Step::StartIscsid);
    }
    if needs.contains(&Need::MultipathBlacklist) {
        steps.push(Step::BlacklistMultipath);
    }
    let mut files = vec![modules_file()];
    if host.multipathd_running {
        files.push(multipath_file());
    }
    Ok(Prep { steps, files })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh Ubuntu host: nothing installed, multipathd running as on Ubuntu Server.
    fn fresh() -> HostState {
        HostState {
            package_manager: Some(PackageManager::Apt),
            missing_packages: PACKAGES.to_vec(),
            unloaded_modules: MODULES.to_vec(),
            iscsid_running: false,
            multipathd_running: true,
            modules_file: None,
            multipath_file: None,
        }
    }

    /// The host after `apply` made the changes of `prep`.
    fn applied(prep: &Prep, host: &HostState) -> HostState {
        let file = |path: &str| {
            prep.files
                .iter()
                .find(|f| f.path == path)
                .map(|f| f.content.clone())
        };
        HostState {
            package_manager: host.package_manager,
            missing_packages: Vec::new(),
            unloaded_modules: Vec::new(),
            iscsid_running: true,
            multipathd_running: host.multipathd_running,
            modules_file: file(MODULES_FILE),
            multipath_file: file(MULTIPATH_FILE).or_else(|| host.multipath_file.clone()),
        }
    }

    #[test]
    fn a_fresh_host_gets_every_change_and_a_second_apply_makes_none() {
        let host = fresh();
        let prep = prepare(&host).unwrap();
        assert_eq!(
            prep.steps,
            [
                Step::Install {
                    manager: PackageManager::Apt,
                    packages: vec!["open-iscsi", "nfs-common"],
                },
                Step::WriteModulesFile,
                Step::LoadModules {
                    modules: vec!["iscsi_tcp", "dm_crypt"],
                },
                Step::StartIscsid,
                Step::BlacklistMultipath,
            ]
        );
        let after = applied(&prep, &host);
        assert_eq!(needs(&after), []);
        let again = prepare(&after).unwrap();
        assert_eq!(again.steps, []);
        assert_eq!(again.files, prep.files);
    }

    #[test]
    fn dnf_installs_the_rhel_names() {
        let host = HostState {
            package_manager: Some(PackageManager::Dnf),
            ..fresh()
        };
        assert_eq!(
            prepare(&host).unwrap().steps[0],
            Step::Install {
                manager: PackageManager::Dnf,
                packages: vec!["iscsi-initiator-utils", "nfs-utils"],
            }
        );
    }

    #[test]
    fn a_host_without_apt_or_dnf_stops_only_when_a_package_is_missing() {
        let host = HostState {
            package_manager: None,
            ..fresh()
        };
        let error = prepare(&host).unwrap_err().to_string();
        assert!(error.contains("open-iscsi"), "{error}");
        assert!(error.contains("iscsi-initiator-utils"), "{error}");
        let installed = HostState {
            missing_packages: Vec::new(),
            ..host
        };
        assert!(prepare(&installed).is_ok());
    }

    #[test]
    fn multipathd_is_left_alone_when_it_does_not_run() {
        let host = HostState {
            multipathd_running: false,
            ..fresh()
        };
        let prep = prepare(&host).unwrap();
        assert!(!prep.steps.contains(&Step::BlacklistMultipath));
        assert!(prep.files.iter().all(|f| f.path != MULTIPATH_FILE));
        assert_eq!(needs(&applied(&prep, &host)), []);
    }

    #[test]
    fn an_edited_file_is_written_again() {
        let prep = prepare(&fresh()).unwrap();
        let ready = applied(&prep, &fresh());
        let edited = HostState {
            multipath_file: Some("blacklist {}\n".to_owned()),
            ..ready.clone()
        };
        assert_eq!(needs(&edited), [Need::MultipathBlacklist]);
        assert_eq!(prepare(&edited).unwrap().steps, [Step::BlacklistMultipath]);
        let stopped = HostState {
            iscsid_running: false,
            unloaded_modules: vec!["iscsi_tcp"],
            ..ready
        };
        assert_eq!(
            prepare(&stopped).unwrap().steps,
            [
                Step::LoadModules {
                    modules: vec!["iscsi_tcp"],
                },
                Step::StartIscsid,
            ]
        );
    }
}
