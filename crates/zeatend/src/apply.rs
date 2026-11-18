//! `plan` and `apply` on the first server of a K3s or RKE2 cluster. Each step checks what is
//! already there, so `apply` runs again safely.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zeatend_host::kubectl::{self, Kubectl};
use zeatend_host::preflight::{self, Status};
use zeatend_host::{HostError, archive, cmd, download, facts, files, systemd};
use zeatend_plan::cluster::{APPARMOR_PROFILE, HostFile, Rancher};
use zeatend_plan::config::Config;
use zeatend_plan::k3s;
use zeatend_plan::plan::{self, ClusterPlan, Download, Plan, Release};
use zeatend_plan::record::{RECORD_FILE, Record};
use zeatend_plan::rke2::{ETCD_USER, PREFIX};

use crate::{InstallError, output, render};

/// Downloads and the install record of K3s and RKE2.
pub(crate) const STATE_DIR: &str = "/var/lib/rancher/zeatend";
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

/// The K3s or RKE2 of a plan; a `KubeSolo` plan is an error.
fn cluster(plan: &Plan) -> Result<&ClusterPlan, InstallError> {
    match &plan.release {
        Release::Cluster(cluster) => Ok(cluster),
        Release::KubeSolo(_) => Err(InstallError::Message(
            "the plan installs KubeSolo, not K3s or RKE2".to_owned(),
        )),
    }
}

fn rancher(config: &Config) -> Result<Rancher, InstallError> {
    config
        .distribution
        .rancher()
        .ok_or_else(|| InstallError::Message("the configuration installs KubeSolo".to_owned()))
}

pub(crate) async fn show_plan(config: &Config, json: bool) -> Result<(), InstallError> {
    let rancher = rancher(config)?;
    let plan = make_plan(config, rancher).await?;
    if json {
        return crate::print_json(&plan);
    }
    output::line(render::plan(&plan).trim_end());
    if show_checks(
        preflight::Node::Server(rancher),
        config.host.require_encrypted_disk,
    )
    .await
    {
        output::line("apply stops until what failed is fixed.");
    }
    Ok(())
}

pub(crate) async fn run(config: &Config) -> Result<(), InstallError> {
    let rancher = rancher(config)?;
    let plan = make_plan(config, rancher).await?;
    output::line(render::plan(&plan).trim_end());
    check_host(
        preflight::Node::Server(rancher),
        config.host.require_encrypted_disk,
    )
    .await?;

    let kubectl = Kubectl::admin(rancher);
    let started = install_cluster(&ClusterInstall {
        cluster: cluster(&plan)?,
        calico: plan.calico.as_ref(),
        files: &plan.host_files,
        server: true,
        changed: false,
    })
    .await?;
    kubectl.wait_ready(Duration::from_mins(10)).await?;
    files::set_modes(Path::new(rancher.layout().tls_dir), ".crt", 0o600)?;
    let name = plan.node_name.clone();
    if started.is_some() {
        kubectl::poll(
            Duration::from_mins(10),
            &format!("node {name} to be Ready"),
            async || {
                crate::join::node_ready(&kubectl, &name, started)
                    .await
                    .unwrap_or(false)
            },
        )
        .await?;
    }

    write_record(
        Path::new(STATE_DIR),
        &Record::new(plan.distribution, plan.release.version()),
    )?;

    output::step("Done");
    let layout = rancher.layout();
    output::line(&format!(
        "{} {} runs on {name}. See the nodes with:\n  sudo {} --kubeconfig {} get nodes -o wide",
        rancher.title(),
        plan.release.version(),
        layout.kubectl,
        layout.admin_kubeconfig
    ));
    Ok(())
}

/// The plan, with the install record read from the host.
async fn make_plan(config: &Config, rancher: Rancher) -> Result<Plan, InstallError> {
    let mut facts = facts::read().await?;
    facts.record = read_record(Path::new(STATE_DIR))?;
    let mut plan = plan::plan(config, &facts)?;
    if plan.changes.is_some() {
        let mut pending = pending(&ClusterInstall {
            cluster: cluster(&plan)?,
            calico: plan.calico.as_ref(),
            files: &plan.host_files,
            server: true,
            changed: false,
        })
        .await?;
        if let Some(changes) = &mut plan.changes {
            // A new release is shown as such, not as its binary.
            if changes.version.is_some() {
                let binary = release_binary(rancher).display().to_string();
                pending.retain(|path| *path != binary);
            }
            changes.set_host_files(pending);
            changes.restart |= restart_owed(STATE_DIR);
        }
    }
    Ok(plan)
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
    serde_json::from_str(&text)
        .ok()
        .and_then(|data| Record::from_data(&data))
        .map(Some)
        .ok_or_else(|| InstallError::Message(format!("{}: not an install record", path.display())))
}

/// Records what was installed in `state_dir`, where the next `plan` reads it.
pub(crate) fn write_record(state_dir: &Path, record: &Record) -> Result<(), InstallError> {
    let json = serde_json::to_string_pretty(record)
        .map_err(|e| InstallError::Message(format!("install record: {e}")))?;
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

/// The preflight checks; a failed one stops `apply`.
pub(crate) async fn check_host(
    node: preflight::Node,
    require_encrypted_disk: bool,
) -> Result<(), InstallError> {
    if show_checks(node, require_encrypted_disk).await {
        return Err(InstallError::Message(
            "the host is not ready; fix what failed and run apply again".to_owned(),
        ));
    }
    Ok(())
}

/// Runs and prints the preflight checks, which only read the host. Returns whether one failed.
pub(crate) async fn show_checks(node: preflight::Node, require_encrypted_disk: bool) -> bool {
    output::step("Checking the host");
    let unit = match node {
        preflight::Node::Server(d) => d.server_unit(),
        preflight::Node::Agent(d) => d.agent_unit(),
        preflight::Node::KubeSolo => zeatend_plan::kubesolo::UNIT,
    };
    let checks = preflight::run(preflight::Options {
        cluster_running: systemd::is_active(unit).await,
        require_encrypted_disk,
        node,
    })
    .await;
    for check in &checks {
        let mark = match check.status {
            Status::Pass => "ok  ",
            Status::Warn => "warn",
            Status::Fail => "FAIL",
        };
        output::err(&format!(
            "    {mark} {:<18} {}",
            check.name.label(),
            check.detail
        ));
    }
    checks.iter().any(|c| c.status == Status::Fail)
}

/// The binary of K3s or RKE2.
fn release_binary(distribution: Rancher) -> PathBuf {
    match distribution {
        Rancher::K3s => PathBuf::from(k3s::BINARY),
        Rancher::Rke2 => Path::new(PREFIX).join("bin/rke2"),
    }
}

/// What `install_release` would install: the binary when it is not the planned release, or the
/// K3s unit when it is missing.
async fn release_pending(cluster: &ClusterPlan, unit: &str) -> Option<String> {
    let binary = release_binary(cluster.distribution);
    let installed = cmd::run(&binary, &["--version"], None)
        .await
        .unwrap_or_default();
    // `install.sh` writes the K3s unit; RKE2's comes in its tarball.
    let unit_file = format!("/etc/systemd/system/{unit}.service");
    if !installed.contains(&cluster.version) {
        Some(binary.display().to_string())
    } else if cluster.distribution == Rancher::K3s && !Path::new(&unit_file).exists() {
        Some(unit_file)
    } else {
        None
    }
}

/// Installs the release when another version or no unit is there.
async fn install_release(
    cluster: &ClusterPlan,
    server: bool,
    unit: &str,
) -> Result<(), InstallError> {
    if release_pending(cluster, unit).await.is_none() {
        return Ok(());
    }
    let distribution = cluster.distribution;
    let binary = release_binary(distribution);
    let dir = PathBuf::from(STATE_DIR)
        .join("downloads")
        .join(&cluster.version);
    fetch_release(cluster, &dir).await?;
    let release = dir.join(&cluster.release.file);
    match distribution {
        Rancher::K3s => {
            files::install(&release, &binary, 0o755)?;
            let script = cluster.install_script.as_ref().ok_or_else(|| {
                InstallError::Message("no install.sh for K3s in the plan".to_owned())
            })?;
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
    /// Calico's manifest, which a K3s server applies.
    pub(crate) calico: Option<&'a Download>,
    pub(crate) files: &'a [HostFile],
    /// A server runs the control plane; an RKE2 server runs etcd as the `etcd` user.
    pub(crate) server: bool,
    /// Something the caller wrote already needs a restart.
    pub(crate) changed: bool,
}

impl ClusterInstall<'_> {
    const fn unit(&self) -> &'static str {
        if self.server {
            self.cluster.distribution.server_unit()
        } else {
            self.cluster.distribution.agent_unit()
        }
    }
}

/// The paths `install_cluster` would write; a running cluster restarts when there is one.
async fn pending(install: &ClusterInstall<'_>) -> Result<Vec<String>, InstallError> {
    let mut paths = changed_files(install.files.iter());
    if let Some(calico) = install.calico {
        let path = calico_path();
        if calico_differs(&pinned_path(calico), calico.sha256.as_deref(), &path)? {
            paths.push(path.display().to_string());
        }
    }
    paths.extend(release_pending(install.cluster, install.unit()).await);
    Ok(paths)
}

/// Installs K3s or RKE2 and starts it, or restarts it when `pending` finds a change. Returns when
/// it did, in seconds since the Unix epoch, taken just before; a Ready condition older than that is
/// the old kubelet's.
pub(crate) async fn install_cluster(
    install: &ClusterInstall<'_>,
) -> Result<Option<u64>, InstallError> {
    let cluster = install.cluster;
    let distribution = cluster.distribution;
    let layout = distribution.layout();
    let unit = install.unit();
    output::step(&format!(
        "{} {} ({unit})",
        distribution.title(),
        cluster.version
    ));
    let changed = install.changed || restart_owed(STATE_DIR) || !pending(install).await?.is_empty();
    if changed {
        owe_restart(STATE_DIR)?;
    }
    for file in install.files {
        files::write(Path::new(&file.path), file.mode, file.content.as_bytes())?;
    }
    cmd::run(
        "sysctl",
        &["--quiet", &format!("--load={}", layout.sysctl_file)],
        None,
    )
    .await?;
    load_apparmor_profile(install.files.iter()).await?;
    match distribution {
        Rancher::Rke2 if install.server && !cmd::succeeds("id", &["etcd"]).await => {
            let (program, args) = ETCD_USER
                .split_first()
                .ok_or_else(|| InstallError::Message("ETCD_USER".to_owned()))?;
            cmd::run(program, args, None).await?;
        }
        Rancher::K3s if install.server => {
            files::make_dir(Path::new(k3s::AUDIT_LOG_DIR), 0o700)?;
        }
        Rancher::K3s | Rancher::Rke2 => {}
    }
    if let Some(calico) = install.calico {
        calico_manifest(calico).await?;
    }
    install_release(cluster, install.server, unit).await?;

    let now = || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    };
    if !systemd::is_active(unit).await {
        output::err(&format!(
            "    starting {unit}; the first start takes a few minutes"
        ));
        let started = now();
        systemd::enable_now(unit).await?;
        restarted(STATE_DIR)?;
        Ok(Some(started))
    } else if changed {
        output::err(&format!("    restarting {unit} for the new configuration"));
        let started = now();
        systemd::restart(unit).await?;
        restarted(STATE_DIR)?;
        Ok(Some(started))
    } else {
        Ok(None)
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
    let images = cluster.distribution.layout().images_dir;
    let mut list = None;
    for d in cluster.downloads() {
        let dest = if cluster.images.contains(d) {
            Path::new(images).join(&d.file)
        } else {
            dir.join(&d.file)
        };
        output::err(&format!("    {}", d.url));
        let actual = download::fetch(&agent, &d.url, &dest, d.sha256.as_deref()).await?;
        if cluster.checksums.as_ref() == Some(d) {
            list = Some(
                fs::read_to_string(&dest)
                    .map_err(|e| InstallError::Message(format!("{}: {e}", dest.display())))?,
            );
        } else if let (None, Some(list)) = (&d.sha256, &list) {
            download::check_against_list(list, &dest, &actual)?;
        }
    }
    Ok(())
}

/// Where a pinned download is kept.
fn pinned_path(download: &Download) -> PathBuf {
    Path::new(STATE_DIR).join("downloads").join(&download.file)
}

/// Where K3s reads Calico's manifest, which it applies on start.
fn calico_path() -> PathBuf {
    Path::new(k3s::MANIFESTS_DIR).join("zeatend-calico.yaml")
}

/// Whether `source` is the download with the digest `sha256`. A source from another pin is stale;
/// one with no digest to check against is never taken as current.
fn is_current(source: &Path, sha256: Option<&str>) -> bool {
    sha256.is_some_and(|expected| download::is_cached(source, expected))
}

/// Whether the manifest at `path` is not the one `calico_manifest` writes from the pinned
/// `source`, which has the digest `sha256`. Without a current copy of the source it cannot tell,
/// so it says it is.
fn calico_differs(source: &Path, sha256: Option<&str>, path: &Path) -> Result<bool, InstallError> {
    if !is_current(source, sha256) {
        return Ok(true);
    }
    let manifest = fs::read_to_string(source)
        .map_err(|e| InstallError::Message(format!("{}: {e}", source.display())))?;
    Ok(fs::read_to_string(path).ok() != Some(k3s::calico_manifest(&manifest)?))
}

/// Writes Calico's manifest, set for K3s, from the pinned source, which it downloads again when
/// the copy is missing or from another pin.
async fn calico_manifest(calico: &Download) -> Result<(), InstallError> {
    let path = calico_path();
    let source = pinned_path(calico);
    let sha256 = calico.sha256.as_deref();
    if !is_current(&source, sha256) {
        output::err(&format!("    {}", calico.url));
        download::fetch(&download::agent(), &calico.url, &source, sha256).await?;
    }
    let manifest = fs::read_to_string(&source)
        .map_err(|e| InstallError::Message(format!("{}: {e}", source.display())))?;
    files::write(&path, 0o600, k3s::calico_manifest(&manifest)?.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_edited_calico_manifest_is_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("calico.yaml");
        let path = dir.path().join("zeatend-calico.yaml");
        assert!(calico_differs(&source, None, &path).unwrap());

        let images: Vec<String> = zeatend_plan::pins::CALICO
            .images
            .iter()
            .map(|(tag, _)| format!("          image: {tag}\n"))
            .collect();
        let manifest = format!(
            "{}            # - name: CALICO_IPV4POOL_CIDR\n            #   value: \"192.168.0.0/16\"\n            - name: CALICO_IPV4POOL_VXLAN\n              value: \"CrossSubnet\"\n",
            images.concat()
        );
        fs::write(&source, &manifest).unwrap();
        let sha256 = download::sha256_file(&source).unwrap();
        let sha256 = Some(sha256.as_str());
        assert!(calico_differs(&source, sha256, &path).unwrap());

        fs::write(&path, k3s::calico_manifest(&manifest).unwrap()).unwrap();
        assert!(!calico_differs(&source, sha256, &path).unwrap());

        fs::write(&path, "edited").unwrap();
        assert!(calico_differs(&source, sha256, &path).unwrap());
    }

    #[test]
    fn a_source_cached_for_another_pin_is_a_change_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("calico.yaml");
        let path = dir.path().join("zeatend-calico.yaml");
        // An older pin's manifest names other images, so the new pin's edits do not apply.
        let stale = "          image: docker.io/calico/node:v0.0.1\n";
        assert!(k3s::calico_manifest(stale).is_err());
        fs::write(&source, stale).unwrap();
        fs::write(&path, "the older pin's manifest").unwrap();
        let pinned = "0".repeat(64);
        assert!(calico_differs(&source, Some(&pinned), &path).unwrap());
        assert!(!is_current(&source, Some(&pinned)));
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
