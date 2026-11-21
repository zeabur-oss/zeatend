//! `plan` and `apply` on the first server of a cluster: K3s, RKE2 or `KubeSolo`. Each step checks
//! what is already there, so `apply` runs again safely.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use k8s_openapi::apimachinery::pkg::version::Info;
use zeatend_host::kubectl::{self, Kubectl};
use zeatend_host::{HostError, archive, cmd, download, facts, files, longhorn, preflight, systemd};
use zeatend_plan::cluster::{
    self, APPARMOR_PROFILE, Addon, Distribution, HelmChart, HostFile, ObjectList, Rancher,
};
use zeatend_plan::config::{Component, Config, Role};
use zeatend_plan::finding::{Finding, Status};
use zeatend_plan::longhorn::Prep;
use zeatend_plan::plan::{self, ClusterPlan, Plan, Release};
use zeatend_plan::precondition::{Change, Preconditions, RecordState};
use zeatend_plan::record::{RECORD_FILE, Record};
use zeatend_plan::rke2::{ETCD_USER, PREFIX};
use zeatend_plan::telemetry::{Action, Command, Run, Step};
use zeatend_plan::upgrade::{Kubernetes, release_tag};
use zeatend_plan::{k3s, ports};

use crate::{InstallError, i18n, output, render, telemetry};

/// The file in a state directory that says a restart is owed.
const RESTART_OWED: &str = "restart-owed";

/// Whether an earlier `apply` changed the host and stopped before the service restarted.
pub(crate) fn restart_owed(state_dir: &str) -> bool {
    Path::new(state_dir).join(RESTART_OWED).exists()
}

/// Records that a restart is owed. Call it before the files or the binary change, so an `apply`
/// that stops before the restart restarts the service on the next run.
pub(crate) fn owe_restart(state_dir: &str) -> Result<(), InstallError> {
    files::write(&Path::new(state_dir).join(RESTART_OWED), 0o600, b"")?;
    Ok(())
}

/// Records that the service runs with the files and the binary on disk.
pub(crate) fn restarted(state_dir: &str) -> Result<(), InstallError> {
    files::remove(&Path::new(state_dir).join(RESTART_OWED))?;
    Ok(())
}

/// The admin `kubectl` of a first server.
pub(crate) fn admin_kubectl(distribution: Distribution) -> Kubectl {
    if distribution.rancher().is_some() {
        Kubectl::admin(distribution)
    } else {
        crate::kubesolo::kubectl()
    }
}

pub(crate) async fn show_plan(config: &Config, json: bool) -> Result<(), InstallError> {
    let run = telemetry::run(config.distribution, Role::Server, Action::Install);
    let plan = install_plan(config).await;
    telemetry::planned(run.clone(), &plan);
    let plan = plan?;
    if json {
        return crate::print_json(&plan);
    }
    output::line(render::plan(&plan).trim_end());
    let failed = show_checks(
        config.distribution,
        Role::Server,
        config.host.require_encrypted_disk,
    )
    .await;
    if !failed.is_empty() {
        telemetry::checks_failed(run, Command::Plan, &failed);
        output::line(i18n::messages().apply_stops);
    }
    Ok(())
}

/// Installs the first node: K3s, RKE2 or `KubeSolo`. With `reviewed`, the preconditions of a
/// reviewed plan, it stops unless the plan made now still meets them.
pub(crate) async fn run(
    config: &Config,
    reviewed: Option<&Preconditions>,
) -> Result<(), InstallError> {
    let plan = install_plan(config).await?;
    output::line(render::plan(&plan).trim_end());
    check_reviewed(reviewed, &plan.preconditions)?;
    let run = telemetry::run(config.distribution, Role::Server, Action::Install);
    check_host(
        config.distribution,
        Role::Server,
        config.host.require_encrypted_disk,
        &run,
    )
    .await?;
    telemetry::started(run);
    prepare_longhorn(plan.longhorn.as_ref()).await?;
    match &plan.release {
        Release::Cluster(cluster) => run_cluster(config, &plan, cluster).await,
        Release::KubeSolo(solo) => crate::kubesolo::run(config, &plan, solo).await,
    }
}

/// Installs the first server of K3s or RKE2 and waits until it is ready.
async fn run_cluster(
    config: &Config,
    plan: &Plan,
    cluster: &ClusterPlan,
) -> Result<(), InstallError> {
    let rancher = cluster.distribution;
    let layout = config.distribution.layout();
    let kubectl = Kubectl::admin(config.distribution);
    let install = ClusterInstall {
        cluster,
        files: &plan.host_files,
        server: true,
        changed: false,
    };
    let restarted = install_cluster(&install).await?;
    let name = plan.node_name.clone();
    let messages = i18n::messages();
    telemetry::reached(Step::Node);
    output::step(&(messages.waiting_for_node)(&name));
    kubectl.wait_ready(Duration::from_mins(10)).await?;
    wait_removed(&kubectl, rancher, &config.disable).await?;
    files::set_modes(Path::new(rancher.paths().tls_dir), ".crt", 0o600)?;
    if restarted {
        crate::join::wait_node_ready(&kubectl, &name, Some(install.unit())).await?;
    }

    write_record(
        Path::new(layout.state_dir),
        &Record::new(plan.distribution, plan.release.version()),
    )?;

    output::step(messages.done);
    let host = reachable_host(&config.host.tls_san).await;
    // The `server:` URL in the admin kubeconfig that K3s and RKE2 write.
    let loopback = ports::api_server_url("127.0.0.1", ports::API_SERVER);
    let url = host
        .as_deref()
        .map(|host| ports::api_server_url(host, ports::API_SERVER));
    output::line(&(messages.ready)(&i18n::Ready {
        cluster: &format!("{} {}", config.distribution.title(), plan.release.version()),
        node: &name,
        kubectl: &format!(
            "sudo {} --kubeconfig {}",
            layout.kubectl, layout.admin_kubeconfig
        ),
        kubeconfig: layout.admin_kubeconfig,
        host: host.as_deref(),
        rewrite: url.as_deref().map(|url| (loopback.as_str(), url)),
        add_nodes: true,
    }));
    Ok(())
}

/// The address a person's computer reaches this server at: the first of `host.tlsSan`, else the
/// host's first address.
pub(crate) async fn reachable_host(tls_san: &[String]) -> Option<String> {
    match tls_san.first() {
        Some(name) => Some(name.clone()),
        None => facts::addresses().await.into_iter().next(),
    }
}

/// Waits until the server removes the components in `disable`, so that a later `apply` that
/// turns a component on again does not restart the server before the removal completes (see
/// `cluster::left_to_remove`).
async fn wait_removed(
    kubectl: &Kubectl,
    rancher: Rancher,
    disable: &[Component],
) -> Result<(), InstallError> {
    if disable.is_empty() {
        return Ok(());
    }
    kubectl::poll(
        Duration::from_mins(10),
        &format!(
            "{} to remove the components that disable turns off",
            rancher.distribution().title()
        ),
        async || {
            let addons: ObjectList<Addon> = kubectl
                .raw("/apis/k3s.cattle.io/v1/namespaces/kube-system/addons")
                .await?;
            let charts: ObjectList<HelmChart> = kubectl
                .raw("/apis/helm.cattle.io/v1/namespaces/kube-system/helmcharts")
                .await?;
            Ok::<_, InstallError>(
                cluster::left_to_remove(rancher, disable, &addons.items, &charts.items, |file| {
                    Path::new(file).exists()
                })
                .is_empty(),
            )
        },
    )
    .await
}

/// The Kubernetes version the API server behind `kubectl` reports; `None` when it does not answer.
pub(crate) async fn running_kubernetes(kubectl: &Kubectl) -> Option<Kubernetes> {
    let version: Info = kubectl.raw("/version").await.ok()?;
    Kubernetes::parse(&version.git_version)
}

/// The plan of the first node, K3s, RKE2 or `KubeSolo`, with the install record and the running
/// Kubernetes version read from the host.
pub(crate) async fn install_plan(config: &Config) -> Result<Plan, InstallError> {
    let layout = config.distribution.layout();
    let mut facts = facts::read().await?;
    if config.storage.longhorn {
        facts.longhorn = Some(longhorn::read().await);
    }
    facts.record = crate::installer::read_record(config.distribution).await?;
    if facts.record.is_some() {
        facts.kubernetes = running_kubernetes(&admin_kubectl(config.distribution)).await;
    }
    let mut plan = plan::plan(config, &facts)?;
    if let Some(changes) = &plan.changes {
        let (pending, outdated) = match &plan.release {
            Release::Cluster(cluster) => {
                let mut pending = pending(&ClusterInstall {
                    cluster,
                    files: &plan.host_files,
                    server: true,
                    changed: false,
                })
                .await?;
                // A new release is shown as such, not as its binary.
                if changes.version.is_some() {
                    pending.retain(|path| path != layout.binary);
                }
                (pending, false)
            }
            Release::KubeSolo(solo) => (
                changed_files(plan.host_files.iter()),
                binary_outdated(Path::new(layout.binary), &solo.version).await,
            ),
        };
        if let Some(changes) = &mut plan.changes {
            changes.set_host_files(pending);
            changes.restart |= outdated || restart_owed(layout.state_dir);
        }
    }
    plan.preconditions = preconditions(
        &written_files(&plan.host_files, plan.longhorn.as_ref()),
        Path::new(layout.binary),
        plan.facts.record.as_ref(),
        &Record::new(plan.distribution, plan.release.version()),
    )
    .await?;
    if let Release::KubeSolo(_) = plan.release {
        crate::kubesolo::add_kubeconfig(&mut plan)?;
    }
    Ok(plan)
}

/// The files a plan writes: the distribution's, then Longhorn's.
pub(crate) fn written_files(host_files: &[HostFile], longhorn: Option<&Prep>) -> Vec<HostFile> {
    host_files
        .iter()
        .chain(longhorn.into_iter().flat_map(|prep| &prep.files))
        .cloned()
        .collect()
}

/// Makes the changes Longhorn needs on this node, before the distribution starts, so that
/// Longhorn finds them on every node it schedules to.
pub(crate) async fn prepare_longhorn(prep: Option<&Prep>) -> Result<(), InstallError> {
    let Some(prep) = prep.filter(|prep| !prep.steps.is_empty()) else {
        return Ok(());
    };
    telemetry::reached(Step::Longhorn);
    output::step(i18n::messages().preparing_longhorn);
    longhorn::apply(prep, |step| output::note(&render::longhorn_step(step))).await?;
    Ok(())
}

/// What a plan that writes `files`, installs the release over `binary` and writes `record` finds
/// on the host now and leaves there.
pub(crate) async fn preconditions(
    files: &[HostFile],
    binary: &Path,
    found_record: Option<&Record>,
    record: &Record,
) -> Result<Preconditions, InstallError> {
    let mut found = std::collections::BTreeMap::new();
    for file in files {
        let change = Change {
            found: file_sha256(Path::new(&file.path))?,
            leaves: Some(download::sha256(file.content.as_bytes())),
        };
        found.insert(file.path.clone(), change);
    }
    Ok(Preconditions {
        files: found,
        release: Change {
            found: installed_release(binary).await,
            leaves: record.version.clone(),
        },
        record: Change {
            found: found_record.map(RecordState::from),
            leaves: Some(record.into()),
        },
        generated: std::collections::BTreeMap::new(),
    })
}

/// The SHA-256 of the file at `path`; `None` when it is missing.
pub(crate) fn file_sha256(path: &Path) -> Result<Option<String>, InstallError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(download::sha256(&bytes))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(HostError::Io {
            path: path.to_owned(),
            source,
        }
        .into()),
    }
}

/// The part of a plan file that `apply --plan` reads.
#[derive(serde::Deserialize)]
struct Reviewed {
    preconditions: Preconditions,
}

/// The preconditions in a file that `plan --json` printed.
///
/// # Errors
///
/// When the file cannot be read or is not such a plan.
pub(crate) fn read_reviewed(path: &Path) -> Result<Preconditions, InstallError> {
    let text = fs::read(path).map_err(InstallError::io(path))?;
    parse_reviewed(path, &text)
}

/// The preconditions in `text`, the content of the plan file at `path`.
///
/// # Errors
///
/// When it is not a plan that `plan --json` printed.
pub(crate) fn parse_reviewed(path: &Path, text: &[u8]) -> Result<Preconditions, InstallError> {
    serde_json::from_slice::<Reviewed>(text)
        .map(|reviewed| reviewed.preconditions)
        .map_err(|e| InstallError::NotPlan {
            path: path.to_owned(),
            reason: e.to_string(),
        })
}

/// Stops when `reviewed` is given and `now`, from the plan made now, does not meet it.
pub(crate) fn check_reviewed(
    reviewed: Option<&Preconditions>,
    now: &Preconditions,
) -> Result<(), InstallError> {
    reviewed.map_or(Ok(()), |reviewed| {
        reviewed.check(now).map_err(InstallError::Stale)
    })
}

/// The install record in `state_dir`, when there is one.
///
/// # Errors
///
/// When the file cannot be read or is not a record.
pub(crate) fn read_record(state_dir: &Path) -> Result<Option<Record>, InstallError> {
    let path = state_dir.join(RECORD_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(HostError::Io { path, source }.into()),
    };
    Record::parse(&text)
        .map(Some)
        .ok_or(InstallError::NotRecord(path))
}

/// Records what was installed in `state_dir`, where the next `plan` reads it.
pub(crate) fn write_record(state_dir: &Path, record: &Record) -> Result<(), InstallError> {
    let json = serde_json::to_string_pretty(record).map_err(|e| InstallError::Serialize {
        what: "install record",
        reason: e.to_string(),
    })?;
    files::write(
        &state_dir.join(RECORD_FILE),
        0o644,
        format!("{json}\n").as_bytes(),
    )?;
    Ok(())
}

/// The paths of `files` whose content on this host differs.
pub(crate) fn changed_files<'a>(files: impl Iterator<Item = &'a HostFile>) -> Vec<String> {
    files
        .filter(|f| fs::read(&f.path).ok().as_deref() != Some(f.content.as_bytes()))
        .map(|f| f.path.clone())
        .collect()
}

/// The preflight checks; a failed one stops `apply` of `run`.
pub(crate) async fn check_host(
    distribution: Distribution,
    role: Role,
    require_encrypted_disk: bool,
    run: &Run,
) -> Result<(), InstallError> {
    let failed = show_checks(distribution, role, require_encrypted_disk).await;
    if !failed.is_empty() {
        telemetry::checks_failed(run.clone(), Command::Apply, &failed);
        return Err(InstallError::HostNotReady);
    }
    Ok(())
}

/// Runs and prints the preflight checks, which only read the host. Returns what failed.
pub(crate) async fn show_checks(
    distribution: Distribution,
    role: Role,
    require_encrypted_disk: bool,
) -> Vec<Finding> {
    let messages = i18n::messages();
    output::step(messages.checking_host);
    let unit = distribution.layout().unit(role);
    let checks = preflight::run(preflight::Options {
        cluster_running: systemd::is_active(unit).await,
        require_encrypted_disk,
        distribution,
        role,
    })
    .await;
    for check in &checks {
        output::note(&messages.check_line(&check.finding));
    }
    checks
        .into_iter()
        .filter(|c| c.status == Status::Fail)
        .map(|c| c.finding)
        .collect()
}

/// Whether the binary at `path` is missing or reports another release than `version`.
pub(crate) async fn binary_outdated(path: &Path, version: &str) -> bool {
    installed_release(path).await.as_deref() != Some(version)
}

/// The release tag that the binary at `path` reports, such as `v1.36.5+k3s1`; `None` when it is
/// missing or reports none.
pub(crate) async fn installed_release(path: &Path) -> Option<String> {
    release_tag(&cmd::version_output(path).await)
}

/// What `install_release` would install: the binary when it is not the planned release, or the
/// K3s unit when it is missing.
async fn release_pending(cluster: &ClusterPlan, unit: &str) -> Option<String> {
    let binary = Path::new(cluster.distribution.distribution().layout().binary);
    // `install.sh` writes the K3s unit; RKE2's comes in its tarball.
    let unit_file = format!("/etc/systemd/system/{unit}.service");
    if installed_release(binary).await.as_deref() != Some(cluster.version.as_str()) {
        Some(binary.display().to_string())
    } else if cluster.distribution == Rancher::K3s && !Path::new(&unit_file).exists() {
        Some(unit_file)
    } else {
        None
    }
}

/// Installs the release: the binary and, for K3s, the unit `install.sh` writes.
async fn install_release(cluster: &ClusterPlan, server: bool) -> Result<(), InstallError> {
    let distribution = cluster.distribution;
    let layout = distribution.distribution().layout();
    let binary = Path::new(layout.binary);
    let dir = PathBuf::from(layout.state_dir)
        .join("downloads")
        .join(&cluster.version);
    fetch_release(cluster, &dir).await?;
    let release = dir.join(&cluster.release.file);
    match distribution {
        Rancher::K3s => {
            files::install(&release, binary, 0o755)?;
            let script = cluster
                .install_script
                .as_ref()
                .ok_or(InstallError::NoInstallScript)?;
            let role = if server { "server" } else { "agent" };
            let script = dir.join(&script.file);
            cmd::run_with(
                "sh",
                &[&script.to_string_lossy(), role],
                &[
                    ("INSTALL_K3S_SKIP_DOWNLOAD", "true"),
                    ("INSTALL_K3S_SKIP_ENABLE", "true"),
                    ("INSTALL_K3S_SKIP_START", "true"),
                    ("INSTALL_K3S_SKIP_SELINUX_RPM", "true"),
                ],
                None,
            )
            .await?;
        }
        Rancher::Rke2 => {
            archive::unpack_tar_gz(&release, Path::new(PREFIX))?;
        }
    }
    systemd::daemon_reload().await?;
    Ok(())
}

/// What `install_cluster` installs: a first server, a joining server or an agent.
pub(crate) struct ClusterInstall<'a> {
    pub(crate) cluster: &'a ClusterPlan,
    pub(crate) files: &'a [HostFile],
    /// A server runs the control plane; an RKE2 server runs etcd as the `etcd` user.
    pub(crate) server: bool,
    /// Something the caller wrote already needs a restart.
    pub(crate) changed: bool,
}

impl ClusterInstall<'_> {
    pub(crate) const fn unit(&self) -> &'static str {
        let role = if self.server {
            Role::Server
        } else {
            Role::Worker
        };
        self.cluster.distribution.distribution().layout().unit(role)
    }
}

/// The paths `install_cluster` would write; a running cluster restarts when there is one.
async fn pending(install: &ClusterInstall<'_>) -> Result<Vec<String>, InstallError> {
    let mut paths = changed_files(install.files.iter());
    paths.extend(release_pending(install.cluster, install.unit()).await);
    Ok(paths)
}

/// Installs K3s or RKE2 and starts it, or restarts it when `pending` finds a change. Returns
/// whether it started or restarted the unit.
pub(crate) async fn install_cluster(install: &ClusterInstall<'_>) -> Result<bool, InstallError> {
    let cluster = install.cluster;
    let distribution = cluster.distribution;
    let unit = install.unit();
    telemetry::reached(Step::Install);
    output::step(&format!(
        "{} {} ({unit})",
        distribution.distribution().title(),
        cluster.version
    ));
    let release = release_pending(cluster, unit).await.is_some();
    let converge = Converge {
        unit,
        state_dir: distribution.distribution().layout().state_dir,
        files: install.files,
        changed: install.changed || release,
    };
    converge
        .run(async || {
            cmd::run(
                "sysctl",
                &[
                    "--quiet",
                    &format!("--load={}", distribution.paths().sysctl_file),
                ],
                None,
            )
            .await?;
            match distribution {
                Rancher::Rke2 if install.server && !cmd::succeeds("id", &["etcd"]).await => {
                    let (program, args) = ETCD_USER;
                    cmd::run(program, args, None).await?;
                }
                Rancher::K3s if install.server => {
                    files::make_dir(Path::new(k3s::AUDIT_LOG_DIR), 0o700)?;
                }
                Rancher::K3s | Rancher::Rke2 => {}
            }
            if release {
                install_release(cluster, install.server).await?;
            }
            Ok(())
        })
        .await
}

/// What [`Converge::run`] writes and starts.
pub(crate) struct Converge<'a> {
    pub(crate) unit: &'static str,
    /// Where it records that a restart is owed.
    pub(crate) state_dir: &'static str,
    pub(crate) files: &'a [HostFile],
    /// The release or something the caller wrote already needs a restart.
    pub(crate) changed: bool,
}

impl Converge<'_> {
    /// Writes the files, runs `install`, the distribution's own steps and its release, then starts
    /// the unit, or restarts it when something changed. Returns whether it started or restarted
    /// the unit.
    pub(crate) async fn run(
        &self,
        install: impl AsyncFnOnce() -> Result<(), InstallError>,
    ) -> Result<bool, InstallError> {
        let changed = self.changed
            || restart_owed(self.state_dir)
            || !changed_files(self.files.iter()).is_empty();
        if changed {
            owe_restart(self.state_dir)?;
        }
        for file in self.files {
            files::write(Path::new(&file.path), file.mode, file.content.as_bytes())?;
        }
        if changed {
            // A unit or a drop-in, such as the one with `GOMEMLIMIT`, takes effect after a reload.
            systemd::daemon_reload().await?;
        }
        load_apparmor_profile(self.files.iter()).await?;
        install().await?;

        let unit = self.unit;
        if !systemd::is_active(unit).await {
            output::note(&(i18n::messages().starting_unit)(unit));
            systemd::enable_now(unit).await?;
        } else if changed {
            output::note(&(i18n::messages().restarting_unit)(unit));
            systemd::restart(unit).await?;
        } else {
            return Ok(false);
        }
        restarted(self.state_dir)?;
        Ok(true)
    }
}

/// Loads the `AppArmor` profile among `files`, so containerd finds it loaded and keeps it.
pub(crate) async fn load_apparmor_profile<'a>(
    mut files: impl Iterator<Item = &'a HostFile>,
) -> Result<(), InstallError> {
    if files.any(|f| f.path == APPARMOR_PROFILE) {
        cmd::run("apparmor_parser", &["--replace", APPARMOR_PROFILE], None).await?;
    }
    Ok(())
}

/// Downloads the release into `dir` and its image archives into the image directory, and checks
/// each against its pinned digest or, for a release that is not pinned, the release's list.
async fn fetch_release(cluster: &ClusterPlan, dir: &Path) -> Result<(), InstallError> {
    let agent = download::agent();
    let images = cluster.distribution.paths().images_dir;
    let mut list = None;
    for d in cluster.downloads() {
        let dest = if cluster.images.contains(d) {
            Path::new(images).join(&d.file)
        } else {
            dir.join(&d.file)
        };
        output::note(&d.url);
        let actual = download::fetch(
            &agent,
            &d.url,
            &dest,
            d.sha256.as_deref(),
            &mut output::Meter,
        )
        .await?;
        if cluster.checksums.as_ref() == Some(d) {
            list = Some(fs::read_to_string(&dest).map_err(InstallError::io(&dest))?);
        } else if let (None, Some(list)) = (&d.sha256, &list) {
            download::check_against_list(list, &dest, &actual)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use zeatend_plan::precondition::Stale;

    use super::*;

    #[tokio::test]
    async fn a_missing_binary_is_outdated() {
        let dir = tempfile::tempdir().unwrap();
        assert!(binary_outdated(&dir.path().join("kubesolo"), "v1.0.0").await);
    }

    #[test]
    fn the_record_is_the_one_that_last_applied() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_record(dir.path()).unwrap(), None);
        let record = Record::new(Rancher::K3s.into(), "v1.0.0+k3s1");
        write_record(dir.path(), &record).unwrap();
        assert_eq!(read_record(dir.path()).unwrap(), Some(record));
        fs::write(dir.path().join(RECORD_FILE), "{}").unwrap();
        let error = read_record(dir.path()).unwrap_err().to_string();
        assert!(error.contains("not an install record"), "{error}");
    }

    #[test]
    fn a_restart_stays_owed_until_the_service_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let state = state.to_str().unwrap();
        assert!(!restart_owed(state));
        owe_restart(state).unwrap();
        // An apply that stops here leaves the marker for the next run.
        assert!(restart_owed(state));
        restarted(state).unwrap();
        assert!(!restart_owed(state));
        restarted(state).unwrap();
    }

    #[tokio::test]
    async fn a_reviewed_plan_applies_until_a_file_it_writes_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        fs::write(&path, "old").unwrap();
        let files = [HostFile {
            path: path.display().to_string(),
            mode: 0o600,
            content: "new".to_owned(),
        }];
        let binary = dir.path().join("k3s");
        let record = Record::new(Rancher::K3s.into(), "v1.37.2+k3s1");
        let facts = plan::Facts {
            arch: "amd64".to_owned(),
            hostname: "vps-1".to_owned(),
            node_name: None,
            host_hex: "a1b2c3".to_owned(),
            memory_mib: 8192,
            node_ips: Vec::new(),
            record: None,
            kubernetes: None,
            apparmor_restricts_unconfined: false,
            k3s_own_network: false,
            installer_webhook: false,
            longhorn: None,
        };
        let mut plan = plan::plan(&Config::parse("{}").unwrap(), &facts).unwrap();
        plan.preconditions = preconditions(&files, &binary, None, &record).await.unwrap();
        let plan_file = dir.path().join("plan.json");
        fs::write(&plan_file, serde_json::to_string_pretty(&plan).unwrap()).unwrap();
        let reviewed = read_reviewed(&plan_file).unwrap();
        assert_eq!(reviewed, plan.preconditions);

        let now = preconditions(&files, &binary, None, &record).await.unwrap();
        check_reviewed(Some(&reviewed), &now).unwrap();

        fs::write(&path, "edited").unwrap();
        let now = preconditions(&files, &binary, None, &record).await.unwrap();
        assert!(matches!(
            check_reviewed(Some(&reviewed), &now),
            Err(InstallError::Stale(Stale::File(changed))) if changed == files[0].path
        ));
        check_reviewed(None, &now).unwrap();

        fs::write(&plan_file, "{}").unwrap();
        assert!(matches!(
            read_reviewed(&plan_file),
            Err(InstallError::NotPlan { .. })
        ));
    }

    #[test]
    fn changed_files_are_the_ones_whose_content_differs() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str, content: &str| HostFile {
            path: dir.path().join(name).display().to_string(),
            mode: 0o600,
            content: content.to_owned(),
        };
        fs::write(dir.path().join("same"), "a").unwrap();
        fs::write(dir.path().join("other"), "old").unwrap();
        let planned = [
            file("same", "a"),
            file("other", "new"),
            file("missing", "x"),
        ];
        assert_eq!(
            changed_files(planned.iter()),
            vec![planned[1].path.clone(), planned[2].path.clone()]
        );
    }
}
