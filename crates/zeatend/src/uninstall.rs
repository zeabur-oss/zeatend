//! `uninstall`: removes K3s, RKE2 or `KubeSolo` from this host with the distribution's own
//! uninstall, then the files zeatend wrote. Volumes stay unless `--delete-volumes` is given.
//!
//! K3s and RKE2 bring `k3s-uninstall.sh`, `k3s-agent-uninstall.sh` and `rke2-uninstall.sh`.
//! zeatend installs only the `kubesolo` binary and its unit, without `kubesoloctl`, so it removes
//! `KubeSolo` itself, as `kubesoloctl uninstall --purge` does. What to remove is planned in
//! [`zeatend_plan::uninstall`].
//!
//! zeatend makes no Kubernetes API writes, so a node that joined a cluster stays in the cluster's
//! node list. The output names the `kubectl delete node` to run on a server.

mod mounts;
mod network;
mod stop;
mod volumes;

use std::fs;
use std::io::{BufRead as _, IsTerminal as _};
use std::path::{Path, PathBuf};

use zeatend_host::{HostError, cmd, facts, files, systemd};
use zeatend_plan::installer;
use zeatend_plan::uninstall::{self, Node, POD_CIDR, Remove};

use crate::{InstallError, load, output};

/// Whether there is something to remove under `root`.
fn pending(root: &Path, remove: &Remove) -> bool {
    let path = at(root, remove.path());
    match remove {
        Remove::Tree(_) => path.symlink_metadata().is_ok(),
        Remove::EmptyDir(_) => fs::read_dir(&path).is_ok_and(|mut d| d.next().is_none()),
    }
}

/// `path` under `root`.
fn at(root: &Path, path: &str) -> PathBuf {
    root.join(path.trim_start_matches('/'))
}

pub(crate) fn exists(root: &Path, path: &str) -> bool {
    at(root, path).symlink_metadata().is_ok()
}

pub(crate) async fn run(yes: bool, delete_volumes: bool) -> Result<(), InstallError> {
    if systemd::booted() {
        let installs =
            systemd::running_services(&format!("{}*.service", crate::unit::PREFIX)).await?;
        if let Some(unit) = installs.first() {
            return Err(InstallError::InstallRuns(unit.clone()));
        }
    }
    let root = Path::new("/");
    let plan = uninstall::plan(uninstall::detect(|path| exists(root, path)));
    let network = network::PodState::read(root, &plan.nodes);
    let remove: Vec<&str> = plan
        .remove
        .iter()
        .filter(|r| pending(root, r))
        .map(Remove::path)
        .collect();
    let volumes: Vec<&str> = plan
        .volumes
        .iter()
        .filter(|v| volumes::present(root, **v))
        .map(|v| v.path)
        .collect();
    let stashed = plan
        .volumes
        .iter()
        .any(|v| v.stash.is_some_and(|s| exists(root, s)));
    let kept_configs: Vec<&str> = [load::DEFAULT_CONFIG, installer::CONFIG_FILE]
        .into_iter()
        .filter(|path| exists(root, path))
        .collect();
    let volumes_to_delete = delete_volumes && !volumes.is_empty();
    if plan.nodes.is_empty() && remove.is_empty() && !stashed && !volumes_to_delete {
        output::line(
            "Nothing to remove: K3s, RKE2 and KubeSolo are not installed on this host, and zeatend left no files.",
        );
        let kept = uninstall::kept(&plan.nodes, &volumes, false, &kept_configs);
        if !kept.is_empty() {
            output::line(&format!("These stay:\n  {}", kept.join("\n  ")));
        }
        return Ok(());
    }
    let mounts = fs::read_to_string("/proc/mounts").map_err(io("/proc/mounts"))?;
    if let Some((mount, path)) =
        mounts::volume_mounts(&mounts, &plan.volumes, delete_volumes).first()
    {
        return Err(InstallError::VolumesOnMount {
            path,
            mount: mount.clone(),
        });
    }
    output::line(
        uninstall::describe(&plan, &remove, &volumes, delete_volumes, &kept_configs).trim_end(),
    );
    if let Some(text) = network.describe() {
        output::line(&format!("\n{text}"));
    }
    if !yes {
        confirm()?;
    }

    let name = facts::installed_node_name(root);
    for node in &plan.nodes {
        output::step(&format!("Stopping {}", node.title()));
        stop::stop(*node).await?;
    }
    if !delete_volumes {
        volumes::stash(root, &plan.volumes)?;
    }
    uninstall_nodes(root, &plan.nodes).await?;
    if matches!(network, network::PodState::Remove(_)) {
        network::remove(root, &plan.nodes).await?;
    }
    output::step("Removing zeatend's files");
    remove_trees(root, &plan.remove)?;
    if delete_volumes {
        output::step("Removing the volumes");
        volumes::delete(root, &plan.volumes)?;
    } else {
        volumes::restore(root, &plan.volumes)?;
    }
    remove_empty_dirs(root, &plan.remove)?;
    if !plan.nodes.is_empty() {
        systemd::daemon_reload().await?;
    }

    output::step("Done");
    if plan.nodes.is_empty() {
        output::line("zeatend's files are removed from this host.");
    } else {
        let titles: Vec<String> = plan.nodes.iter().map(|n| n.title()).collect();
        output::line(&format!(
            "{} is removed from this host.",
            titles.join(" and ")
        ));
    }
    if plan.nodes.iter().any(|n| n.joined) {
        output::line(&uninstall::leave_text(name.as_deref()));
    }
    Ok(())
}

/// Runs each node's own uninstall, then deletes the routes into the pod network that K3s's or
/// RKE2's network plugin left.
async fn uninstall_nodes(root: &Path, nodes: &[Node]) -> Result<(), InstallError> {
    for node in nodes {
        if let Some(script) = node.script().filter(|s| exists(root, s)) {
            output::step(&format!("Running {script}"));
            cmd::run(script, &[], None).await?;
        }
    }
    if nodes.iter().any(|n| n.distribution.rancher().is_some()) {
        output::step(&format!(
            "Removing the routes into the pod network {POD_CIDR}"
        ));
        remove_pod_routes().await?;
    }
    Ok(())
}

/// Asks before anything is removed.
fn confirm() -> Result<(), InstallError> {
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        return Err(InstallError::NoTerminal);
    }
    output::err("Remove? Type yes to go on:");
    let mut answer = String::new();
    stdin
        .lock()
        .read_line(&mut answer)
        .map_err(InstallError::Stdin)?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(InstallError::NotConfirmed)
    }
}

/// Deletes the routes into the pod network that the network plugin added and the distribution's
/// uninstall left, so a later install on this host can reach its own pods.
async fn remove_pod_routes() -> Result<(), InstallError> {
    let routes = cmd::run("ip", &["-4", "route", "show"], None).await?;
    for route in uninstall::pod_routes(&routes) {
        let args: Vec<&str> = route.iter().map(String::as_str).collect();
        cmd::run("ip", &args, None).await?;
    }
    Ok(())
}

fn remove_trees(root: &Path, remove: &[Remove]) -> Result<(), InstallError> {
    for item in remove {
        if let Remove::Tree(path) = item {
            remove_tree(&at(root, path))?;
        }
    }
    Ok(())
}

fn remove_empty_dirs(root: &Path, remove: &[Remove]) -> Result<(), InstallError> {
    for item in remove {
        if let Remove::EmptyDir(_) = item
            && pending(root, item)
        {
            let path = at(root, item.path());
            fs::remove_dir(&path).map_err(io(&path))?;
        }
    }
    Ok(())
}

fn io(path: impl AsRef<Path>) -> impl FnOnce(std::io::Error) -> HostError {
    let path = path.as_ref().to_path_buf();
    move |source| HostError::Io { path, source }
}

/// Removes a file, or a directory with everything in it. A missing path is not an error.
fn remove_tree(path: &Path) -> Result<(), InstallError> {
    match path.symlink_metadata() {
        Ok(meta) if meta.is_dir() => {
            fs::remove_dir_all(path).map_err(io(path))?;
        }
        Ok(_) => files::remove(path)?,
        Err(_) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use zeatend_plan::cluster::Distribution;
    use zeatend_plan::config::Role;
    use zeatend_plan::record::RECORD_FILE;
    use zeatend_plan::uninstall::{LONGHORN, Uninstall, ZEABUR_STORAGE};
    use zeatend_plan::{k3s, kubesolo};

    use super::*;

    const K3S_LOCAL_PATH: &str = "/var/lib/rancher/k3s/storage";
    const RANCHER_STATE_DIR: &str = "/var/lib/rancher/zeatend";
    const KUBESOLO_BINARY: &str = "/usr/local/bin/kubesolo";
    const KUBESOLO_DATA_DIR: &str = "/var/lib/kubesolo";
    const KUBESOLO_STATE_DIR: &str = "/var/lib/zeatend";
    const KUBESOLO_KUBECTL: &str = "/var/lib/zeatend/bin/kubectl";
    const KUBESOLO_LOCAL_PATH: &str = "/var/lib/kubesolo/local-path-storage";

    const K3S_SERVER: Node = Node::new(Distribution::K3s, Role::Server, false);
    const KUBESOLO: Node = Node::new(Distribution::KubeSolo, Role::Server, false);

    fn touch(root: &Path, path: &str) {
        let path = at(root, path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
    }

    /// What `run` does after the nodes stopped, with the uninstall script's removal in between.
    fn clean(root: &Path, plan: &Uninstall, delete_volumes: bool, script_removes: &str) {
        if !delete_volumes {
            volumes::stash(root, &plan.volumes).unwrap();
        }
        remove_tree(&at(root, script_removes)).unwrap();
        remove_trees(root, &plan.remove).unwrap();
        if delete_volumes {
            volumes::delete(root, &plan.volumes).unwrap();
        } else {
            volumes::restore(root, &plan.volumes).unwrap();
        }
        remove_empty_dirs(root, &plan.remove).unwrap();
    }

    #[test]
    fn k3s_volumes_stay_where_they_were_and_the_rest_goes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let volume = format!("{K3S_LOCAL_PATH}/pvc-1/data");
        touch(root, &volume);
        touch(root, &format!("{LONGHORN}/replicas/r"));
        touch(root, &format!("{RANCHER_STATE_DIR}/{RECORD_FILE}"));
        touch(root, &format!("{RANCHER_STATE_DIR}/downloads/v1/k3s"));
        touch(root, "/etc/sysctl.d/60-k3s-cis.conf");
        touch(root, k3s::MEMORY_DROP_IN);
        touch(root, "/var/lib/rancher/k3s/server/db/state.db");
        touch(root, load::DEFAULT_CONFIG);
        let k3s = uninstall::plan(vec![K3S_SERVER]);
        clean(root, &k3s, false, "/var/lib/rancher/k3s");

        assert!(exists(root, &volume));
        assert!(exists(root, &format!("{LONGHORN}/replicas/r")));
        assert!(exists(root, load::DEFAULT_CONFIG));
        for gone in [
            RANCHER_STATE_DIR,
            "/etc/sysctl.d/60-k3s-cis.conf",
            "/etc/systemd/system/k3s.service.d",
            "/var/lib/rancher/k3s/server",
            "/var/lib/rancher/.zeatend-kept-k3s-storage",
        ] {
            assert!(!exists(root, gone), "{gone}");
        }
        // A second run finds nothing to remove.
        let again = uninstall::plan(uninstall::detect(|p| exists(root, p)));
        assert_eq!(again.nodes, Vec::new());
        assert!(!again.remove.iter().any(|r| pending(root, r)), "{again:?}");
    }

    #[test]
    fn delete_volumes_removes_them_too() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(root, &format!("{KUBESOLO_LOCAL_PATH}/pvc-1/data"));
        touch(root, &format!("{ZEABUR_STORAGE}/a"));
        touch(root, "/var/lib/kubesolo/kine/state.db");
        touch(root, KUBESOLO_KUBECTL);
        touch(root, kubesolo::KUBECONFIG);
        touch(root, KUBESOLO_BINARY);
        let solo = uninstall::plan(vec![KUBESOLO]);
        clean(root, &solo, true, KUBESOLO_DATA_DIR);
        for gone in [
            KUBESOLO_DATA_DIR,
            ZEABUR_STORAGE,
            KUBESOLO_STATE_DIR,
            "/etc/zeatend",
            KUBESOLO_BINARY,
        ] {
            assert!(!exists(root, gone), "{gone}");
        }
    }

    #[test]
    fn kubesolo_volumes_survive_the_removal_of_its_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let volume = format!("{KUBESOLO_LOCAL_PATH}/pvc-1/data");
        touch(root, &volume);
        touch(root, "/var/lib/kubesolo/kine/state.db");
        clean(root, &uninstall::plan(vec![KUBESOLO]), false, "/nothing");
        assert!(exists(root, &volume));
        assert!(!exists(root, "/var/lib/kubesolo/kine"));
    }

    #[test]
    fn volumes_from_an_uninstall_that_stopped_are_put_back() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(
            root,
            "/var/lib/rancher/.zeatend-kept-k3s-storage/pvc-1/data",
        );
        let leftovers = uninstall::plan(Vec::new());
        assert!(leftovers.volumes.iter().any(|v| volumes::present(root, *v)));
        clean(root, &leftovers, false, "/nothing");
        assert!(exists(root, &format!("{K3S_LOCAL_PATH}/pvc-1/data")));
    }
}
