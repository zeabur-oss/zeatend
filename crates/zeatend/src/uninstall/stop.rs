//! Stopping a node and its pods before its files go.

use std::fs;
use std::path::Path;
use std::time::Duration;

use zeatend_host::cmd;
use zeatend_plan::cluster::Distribution;
use zeatend_plan::kubesolo;
use zeatend_plan::uninstall::Node;

use super::mounts;
use crate::{InstallError, output};

/// Stops the node and its pods, so no process writes to a volume or holds a mount while the
/// uninstall runs.
pub(super) async fn stop(node: Node) -> Result<(), InstallError> {
    let Some(killall) = node.distribution.layout().killall else {
        return stop_kubesolo().await;
    };
    // The script's exit status is that of its last cleanup command, and the uninstall script
    // runs it again, so a failure is reported and the uninstall goes on.
    if Path::new(killall).exists()
        && let Err(error) = cmd::run(killall, &[], None).await
    {
        output::err(&format!("    warning: {error}"));
    }
    Ok(())
}

/// The cgroups of `KubeSolo`'s pods and of its unit. With `KillMode=process`, stopping the unit
/// leaves the containers and their shims running in them.
/// The first three are cgroup v2; the rest are the `pids` hierarchy of cgroup v1, which lists
/// every process.
const KUBESOLO_CGROUPS: [&str; 6] = [
    "/sys/fs/cgroup/kubepods.slice",
    "/sys/fs/cgroup/kubepods",
    "/sys/fs/cgroup/system.slice/kubesolo.service",
    "/sys/fs/cgroup/pids/kubepods.slice",
    "/sys/fs/cgroup/pids/kubepods",
    "/sys/fs/cgroup/pids/system.slice/kubesolo.service",
];

async fn stop_kubesolo() -> Result<(), InstallError> {
    if Path::new(kubesolo::UNIT_FILE).exists() {
        let unit = Distribution::KubeSolo.layout().server_unit;
        cmd::run("systemctl", &["disable", "--now", unit], None).await?;
    }
    for cgroup in KUBESOLO_CGROUPS {
        kill_cgroup(Path::new(cgroup)).await?;
    }
    let mounts = fs::read_to_string("/proc/mounts").map_err(super::io("/proc/mounts"))?;
    for mount in mounts::runtime_mounts(&mounts) {
        cmd::run("umount", &["--lazy", &mount], None).await?;
    }
    Ok(())
}

/// Kills every process in a cgroup tree and waits until it is empty. `cgroup.kill` (Linux 5.14)
/// kills the tree at once; without it, each process in its `cgroup.procs` files gets `SIGKILL`,
/// again on each round, so a process forked in between is killed too. A tree that still has
/// processes after 10 seconds stops the uninstall before anything is removed.
async fn kill_cgroup(cgroup: &Path) -> Result<(), InstallError> {
    if !cgroup.is_dir() {
        return Ok(());
    }
    let kill = cgroup.join("cgroup.kill");
    let has_kill = kill.exists();
    if has_kill {
        fs::write(&kill, "1").map_err(super::io(&kill))?;
    }
    for _ in 0..100 {
        let pids = cgroup_pids(cgroup);
        if pids.is_empty() {
            return Ok(());
        }
        if !has_kill {
            let mut args = vec!["-s", "KILL", "--"];
            args.extend(pids.iter().map(String::as_str));
            // A process may end between the read and the kill; the next round checks again.
            drop(cmd::run("kill", &args, None).await);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(InstallError::ProcessesRemain(cgroup.to_owned()))
}

/// The process ids in the `cgroup.procs` files of a cgroup and the cgroups under it.
fn cgroup_pids(cgroup: &Path) -> Vec<String> {
    let mut pids = Vec::new();
    let mut dirs = vec![cgroup.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        if let Ok(text) = fs::read_to_string(dir.join("cgroup.procs")) {
            for pid in text.split_whitespace() {
                if !pids.iter().any(|p| p == pid) {
                    pids.push(pid.to_owned());
                }
            }
        }
        dirs.extend(
            fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path()),
        );
    }
    pids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pids_of_a_cgroup_tree_are_read_from_every_level() {
        let dir = tempfile::tempdir().unwrap();
        let cgroup = dir.path().join("kubepods.slice");
        let pod = cgroup.join("pod-a/container-1");
        fs::create_dir_all(&pod).unwrap();
        fs::write(cgroup.join("cgroup.procs"), "").unwrap();
        fs::write(pod.join("cgroup.procs"), "101\n102\n").unwrap();
        fs::write(cgroup.join("pod-a/cgroup.procs"), "100\n101\n").unwrap();
        let mut pids = cgroup_pids(&cgroup);
        pids.sort();
        assert_eq!(pids, ["100", "101", "102"]);
        assert_eq!(
            cgroup_pids(&dir.path().join("missing")),
            Vec::<String>::new()
        );
    }

    #[tokio::test]
    async fn an_empty_or_missing_cgroup_needs_no_kill() {
        let dir = tempfile::tempdir().unwrap();
        kill_cgroup(&dir.path().join("missing")).await.unwrap();
        fs::write(dir.path().join("cgroup.procs"), "").unwrap();
        kill_cgroup(dir.path()).await.unwrap();
    }
}
