//! `uninstall`: removes K3s, RKE2 or `KubeSolo` from this host with the distribution's own
//! uninstall, then the files zeatend wrote. Volumes stay unless `--delete-volumes` is given.
//!
//! K3s and RKE2 bring `k3s-uninstall.sh`, `k3s-agent-uninstall.sh` and `rke2-uninstall.sh`.
//! zeatend installs only the `kubesolo` binary and its unit, without `kubesoloctl`, so it removes
//! `KubeSolo` itself, as `kubesoloctl uninstall --purge` does.
//!
//! zeatend makes no Kubernetes API writes, so a node that joined a cluster stays in the cluster's
//! node list. The output names the `kubectl delete node` to run on a server.

use std::fs;
use std::io::{BufRead as _, IsTerminal as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use zeatend_host::{HostError, cmd, facts, files, systemd};
use zeatend_plan::cluster::{APPARMOR_PROFILE, Distribution, Options, Rancher};
use zeatend_plan::config::Role;
use zeatend_plan::tuning::{HostTuning, MemoryTier};
use zeatend_plan::{PlanError, installer, k3s, kubesolo};

use crate::{InstallError, load, output};

/// The CNI configuration `KubeSolo` writes on every start.
const KUBESOLO_CNI_CONFIG: &str = "/etc/cni/net.d/10-bridge.conflist";
/// The volumes of Zeabur's local storage.
const ZEABUR_STORAGE: &str = "/var/lib/zeabur/storage";
/// Longhorn's replicas.
const LONGHORN: &str = "/var/lib/longhorn";

/// A cluster node on this host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Node {
    pub(crate) distribution: Distribution,
    /// `KubeSolo` is a server.
    pub(crate) role: Role,
    /// It joined a cluster that another host runs.
    joined: bool,
}

impl Node {
    pub(crate) const fn new(distribution: Distribution, role: Role, joined: bool) -> Self {
        Self {
            distribution,
            role,
            joined,
        }
    }

    fn title(self) -> String {
        match (self.distribution, self.role) {
            (Distribution::KubeSolo, _) => "KubeSolo".to_owned(),
            (d, Role::Server) => format!("{} (server)", d.title()),
            (d, Role::Worker) => format!("{} (worker)", d.title()),
        }
    }

    /// The distribution's own uninstall, if it has one.
    const fn script(self) -> Option<&'static str> {
        self.distribution.layout().uninstall(self.role)
    }

    /// What the distribution's uninstall removes, in words.
    fn uninstall_text(self) -> String {
        let (layout, title) = (self.distribution.layout(), self.distribution.title());
        self.script().map_or_else(
            || {
                format!(
                    "zeatend stops {title} and its pods and removes its service, its binary, {}, \
                     {KUBESOLO_CNI_CONFIG} and {}: the cluster's data, certificates and images.",
                    kubesolo::CONFIG_FILE,
                    layout.data_dir,
                )
            },
            |script| {
                format!(
                    "{script} stops {title} and its pods and removes its service, its binary, {} \
                     and {}: the cluster's data, certificates and images on this host.",
                    layout.config_dir, layout.data_dir,
                )
            },
        )
    }

    /// The paths zeatend left on the host for this node, removed after the distribution's
    /// uninstall. Many are in directories that uninstall removes; zeatend removes them all the
    /// same, so a failed or partial uninstall leaves none.
    fn paths(self) -> Result<Vec<Remove>, PlanError> {
        let layout = self.distribution.layout();
        let Some(rancher) = self.distribution.rancher() else {
            return Ok(vec![
                Remove::Tree(kubesolo::UNIT_FILE.to_owned()),
                Remove::Tree(layout.binary.to_owned()),
                Remove::Tree(kubesolo::CONFIG_FILE.to_owned()),
                Remove::Tree(KUBESOLO_CNI_CONFIG.to_owned()),
                Remove::Tree(layout.data_dir.to_owned()),
                Remove::Tree(layout.state_dir.to_owned()),
                Remove::Tree(kubesolo::KUBECONFIG.to_owned()),
                Remove::Tree(APPARMOR_PROFILE.to_owned()),
                Remove::Tree(installer::KUBESOLO_STATE_DIR.to_owned()),
                Remove::Tree(installer::KUBESOLO_KUBECONFIG.to_owned()),
                Remove::Tree(installer::KUBESOLO_WEBHOOK_SCRIPT.to_owned()),
                Remove::EmptyDir(parent(kubesolo::CONFIG_FILE)),
                Remove::EmptyDir(parent(kubesolo::KUBECONFIG)),
                Remove::EmptyDir(parent(installer::KUBESOLO_KUBECONFIG)),
            ]);
        };
        let mut paths = vec![
            Remove::Tree(layout.state_dir.to_owned()),
            Remove::Tree(installer::RANCHER_STATE_DIR.to_owned()),
        ];
        for tier in [MemoryTier::Small, MemoryTier::Standard] {
            let tuning = HostTuning::new(rancher.into(), tier);
            let written = match self.role {
                Role::Server => rancher.server_files(&Options {
                    tls_san: Vec::new(),
                    tuning,
                    own_network: false,
                    disable: Vec::new(),
                })?,
                Role::Worker => rancher.agent_files(&tuning)?,
            };
            push_all(
                &mut paths,
                written.into_iter().map(|f| Remove::Tree(f.path)),
            );
        }
        let mut more = vec![rancher.node_name_path(), rancher.node_ip_path()];
        if self.joined {
            more.extend([
                rancher.join_path(),
                rancher.paths().join_token_file.to_owned(),
                installer::join_token_file(rancher),
            ]);
        }
        more.push(APPARMOR_PROFILE.to_owned());
        if (rancher, self.role) == (Rancher::K3s, Role::Server) {
            more.push(k3s::AUDIT_LOG_DIR.to_owned());
        }
        push_all(&mut paths, more.into_iter().map(Remove::Tree));
        if (rancher, self.role) == (Rancher::K3s, Role::Server) {
            paths.push(Remove::EmptyDir(parent(k3s::MEMORY_DROP_IN)));
        }
        paths.push(Remove::EmptyDir(parent(layout.state_dir)));
        Ok(paths)
    }

    /// Where volumes may be on this node.
    fn volumes(self) -> Vec<Volume> {
        // The uninstall removes the data directory around the local-path volumes.
        let stash = match self.distribution {
            Distribution::K3s => Some("/var/lib/rancher/.zeatend-kept-k3s-storage"),
            Distribution::KubeSolo => Some("/var/lib/.zeatend-kept-kubesolo-local-path-storage"),
            Distribution::Rke2 => None,
        };
        let local = self
            .distribution
            .layout()
            .local_path
            .zip(stash)
            .map(|(path, stash)| Volume {
                path,
                stash: Some(stash),
            });
        local
            .into_iter()
            .chain([ZEABUR_STORAGE, LONGHORN].map(|path| Volume { path, stash: None }))
            .collect()
    }
}

/// The directory that holds `path`.
fn parent(path: &str) -> String {
    Path::new(path)
        .parent()
        .map_or_else(String::new, |p| p.display().to_string())
}

fn push_all<T: PartialEq>(list: &mut Vec<T>, items: impl IntoIterator<Item = T>) {
    for item in items {
        if !list.contains(&item) {
            list.push(item);
        }
    }
}

/// A path zeatend removes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Remove {
    /// A file, or a directory with everything in it.
    Tree(String),
    /// A directory, only when it is empty: something else may keep files in it.
    EmptyDir(String),
}

impl Remove {
    fn path(&self) -> &str {
        match self {
            Self::Tree(path) | Self::EmptyDir(path) => path,
        }
    }

    /// Whether there is something to remove under `root`.
    fn pending(&self, root: &Path) -> bool {
        let path = at(root, self.path());
        match self {
            Self::Tree(_) => path.symlink_metadata().is_ok(),
            Self::EmptyDir(_) => fs::read_dir(&path).is_ok_and(|mut d| d.next().is_none()),
        }
    }
}

/// A directory that may hold volumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Volume {
    path: &'static str,
    /// Where it waits, on the same file system, while the uninstall removes the directory around
    /// it. `None` when the uninstall does not touch it.
    stash: Option<&'static str>,
}

impl Volume {
    /// Whether it exists under `root`, in its place or stashed by an uninstall that stopped.
    fn present(self, root: &Path) -> bool {
        exists(root, self.path) || self.stash.is_some_and(|stash| exists(root, stash))
    }
}

/// What `uninstall` does on a host.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Uninstall {
    nodes: Vec<Node>,
    /// zeatend's paths, in the order to remove them.
    remove: Vec<Remove>,
    volumes: Vec<Volume>,
}

/// The nodes on a host where `exists` says which paths exist.
pub(crate) fn detect(exists: impl Fn(&str) -> bool) -> Vec<Node> {
    let mut nodes = Vec::new();
    // zeatend's join token, or zeabur-installer's on a node it joined.
    let joined = |rancher: Rancher| {
        exists(rancher.paths().join_token_file) || exists(&installer::join_token_file(rancher))
    };
    let k3s = Distribution::K3s.layout();
    let rke2 = Distribution::Rke2.layout();
    let kubesolo = Distribution::KubeSolo.layout();
    if k3s.server_uninstall.is_some_and(&exists) {
        // `join` takes no K3s server today; a server with a join token joined all the same.
        let joined = joined(Rancher::K3s);
        nodes.push(Node::new(Distribution::K3s, Role::Server, joined));
    }
    if k3s.agent_uninstall.is_some_and(&exists) {
        nodes.push(Node::new(Distribution::K3s, Role::Worker, true));
    }
    if rke2.server_uninstall.is_some_and(&exists) {
        // Only a server has its own token.
        let role = if exists(Rancher::Rke2.paths().server_token_file) {
            Role::Server
        } else {
            Role::Worker
        };
        let joined = role == Role::Worker || joined(Rancher::Rke2);
        nodes.push(Node::new(Distribution::Rke2, role, joined));
    }
    if exists(kubesolo.binary) || exists(kubesolo::UNIT_FILE) {
        nodes.push(Node::new(Distribution::KubeSolo, Role::Server, false));
    }
    nodes
}

/// Every kind of node, for a host where none is left: zeatend removes what any of them left.
const ALL_NODES: [Node; 5] = [
    Node::new(Distribution::K3s, Role::Server, false),
    Node::new(Distribution::K3s, Role::Worker, true),
    Node::new(Distribution::Rke2, Role::Server, true),
    Node::new(Distribution::Rke2, Role::Worker, true),
    Node::new(Distribution::KubeSolo, Role::Server, false),
];

/// What `uninstall` removes and keeps for `nodes`.
fn plan(nodes: Vec<Node>) -> Result<Uninstall, PlanError> {
    let sources = if nodes.is_empty() {
        ALL_NODES.to_vec()
    } else {
        nodes.clone()
    };
    let mut remove = Vec::new();
    let mut volumes = Vec::new();
    for node in &sources {
        push_all(&mut remove, node.paths()?);
        push_all(&mut volumes, node.volumes());
    }
    // Files and trees first, then directories that only they kept from being empty.
    remove.sort_by_key(|r| matches!(r, Remove::EmptyDir(_)));
    Ok(Uninstall {
        nodes,
        remove,
        volumes,
    })
}

/// What `uninstall` shows before it asks.
fn describe(
    plan: &Uninstall,
    remove: &[&str],
    volumes: &[&str],
    delete_volumes: bool,
    configs: &[&str],
) -> String {
    let mut out = String::new();
    let mut line = |text: &str| {
        out.push_str(text);
        out.push('\n');
    };
    if plan.nodes.is_empty() {
        line("K3s, RKE2 and KubeSolo are not installed on this host.");
    } else {
        let titles: Vec<String> = plan.nodes.iter().map(|n| n.title()).collect();
        line(&format!(
            "zeatend uninstall removes {} from this host.",
            titles.join(" and ")
        ));
        for node in &plan.nodes {
            line(&format!("\n{}", node.uninstall_text()));
        }
    }
    if !remove.is_empty() {
        line("\nzeatend removes the files it left:");
        for path in remove {
            line(&format!("  {path}"));
        }
    }
    if delete_volumes && !volumes.is_empty() {
        line("\n--delete-volumes removes the volumes, and the data in them:");
        for path in volumes {
            line(&format!("  {path}"));
        }
    }
    let kept = kept(&plan.nodes, volumes, delete_volumes, configs);
    if !kept.is_empty() {
        line("\nThese stay:");
        for item in &kept {
            line(&format!("  {item}"));
        }
    }
    out
}

/// What stays on the host.
fn kept(nodes: &[Node], volumes: &[&str], delete_volumes: bool, configs: &[&str]) -> Vec<String> {
    let mut kept = Vec::new();
    if !delete_volumes {
        kept.extend(
            volumes
                .iter()
                .map(|path| format!("{path}  (volumes; --delete-volumes removes them)")),
        );
    }
    kept.extend(configs.iter().map(|&path| {
        let whose = if path == installer::CONFIG_FILE {
            "zeabur-installer's"
        } else {
            "zeatend's"
        };
        format!("{path}  ({whose} configuration)")
    }));
    if nodes
        .iter()
        .any(|n| (n.distribution, n.role) == (Distribution::Rke2, Role::Server))
    {
        kept.push(
            "the etcd user  (apply adds it when it is missing, so zeatend cannot tell whether \
             something else uses it)"
                .to_owned(),
        );
    }
    kept
}

/// What to run on a server after a node that joined a cluster left this host.
fn leave_text(name: Option<&str>) -> String {
    format!(
        "This node was in a cluster that other hosts run. zeatend makes no changes through the \
         Kubernetes API, so the cluster still lists the node. On a server, remove it with:\n  \
         kubectl delete node {}",
        name.unwrap_or("<node name>")
    )
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
    let plan = plan(detect(|path| exists(root, path)))?;
    let remove: Vec<&str> = plan
        .remove
        .iter()
        .filter(|r| r.pending(root))
        .map(Remove::path)
        .collect();
    let volumes: Vec<&str> = plan
        .volumes
        .iter()
        .filter(|v| v.present(root))
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
        let kept = kept(&plan.nodes, &volumes, false, &kept_configs);
        if !kept.is_empty() {
            output::line(&format!("These stay:\n  {}", kept.join("\n  ")));
        }
        return Ok(());
    }
    let mounts = fs::read_to_string("/proc/mounts").map_err(io("/proc/mounts"))?;
    if let Some((mount, path)) = volume_mounts(&mounts, &plan.volumes, delete_volumes).first() {
        return Err(InstallError::VolumesOnMount {
            path,
            mount: mount.clone(),
        });
    }
    output::line(describe(&plan, &remove, &volumes, delete_volumes, &kept_configs).trim_end());
    if !yes {
        confirm()?;
    }

    let name = facts::installed_node_name(root);
    for node in &plan.nodes {
        output::step(&format!("Stopping {}", node.title()));
        stop(*node).await?;
    }
    if !delete_volumes {
        stash(root, &plan.volumes)?;
    }
    for node in &plan.nodes {
        if let Some(script) = node.script().filter(|s| exists(root, s)) {
            output::step(&format!("Running {script}"));
            cmd::run(script, &[], None).await?;
        }
    }
    output::step("Removing zeatend's files");
    remove_trees(root, &plan.remove)?;
    if delete_volumes {
        output::step("Removing the volumes");
        delete(root, &plan.volumes)?;
    } else {
        restore(root, &plan.volumes)?;
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
        output::line(&leave_text(name.as_deref()));
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

/// Stops the node and its pods, so no process writes to a volume or holds a mount while the
/// uninstall runs.
async fn stop(node: Node) -> Result<(), InstallError> {
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
    let mounts = fs::read_to_string("/proc/mounts").map_err(io("/proc/mounts"))?;
    for mount in runtime_mounts(&mounts) {
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
        fs::write(&kill, "1").map_err(io(&kill))?;
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
            push_all(&mut pids, text.split_whitespace().map(str::to_owned));
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

/// The mounts of `KubeSolo`'s containers and pods under its data directory, deepest first: not
/// the data directory itself and not its local-path volumes, whose data the uninstall keeps or
/// removes first.
fn runtime_mounts(mounts: &str) -> Vec<String> {
    let layout = Distribution::KubeSolo.layout();
    let storage = layout
        .local_path
        .map_or_default(|path| mounts_under(mounts, path));
    mounts_under(mounts, layout.data_dir)
        .into_iter()
        .filter(|m| m != layout.data_dir && !storage.contains(m))
        .collect()
}

/// The mount points that hold volumes this uninstall moves or removes, each with its volume.
///
/// A rename does not move a mount point, so a stashed volume must not be, hold or sit under one.
fn volume_mounts(
    mounts: &str,
    volumes: &[Volume],
    delete_volumes: bool,
) -> Vec<(String, &'static str)> {
    let all = mount_points(mounts);
    volumes
        .iter()
        .filter(|volume| volume.stash.is_some() || delete_volumes)
        .flat_map(|volume| {
            let mut points = mounts_under(mounts, volume.path);
            if let Some(stash) = volume.stash {
                let top = Path::new(stash).parent();
                points.extend(
                    Path::new(volume.path)
                        .ancestors()
                        .skip(1)
                        .take_while(|dir| Some(*dir) != top)
                        .filter_map(|dir| dir.to_str())
                        .filter(|dir| all.iter().any(|m| m == dir))
                        .map(str::to_owned),
                );
            }
            points.into_iter().map(|m| (m, volume.path))
        })
        .collect()
}

/// The mount points in `mounts`, as in `/proc/mounts`.
fn mount_points(mounts: &str) -> Vec<String> {
    mounts
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(unescape_mount)
        .collect()
}

/// The mount points in `mounts` (as in `/proc/mounts`) at or under `dir`, deepest first.
fn mounts_under(mounts: &str, dir: &str) -> Vec<String> {
    let prefix = format!("{dir}/");
    let mut found: Vec<String> = mount_points(mounts)
        .into_iter()
        .filter(|m| m == dir || m.starts_with(&prefix))
        .collect();
    found.sort_by(|a, b| b.cmp(a));
    found.dedup();
    found
}

/// Decodes a `/proc/mounts` field. The kernel (`mangle` in `fs/proc_namespace.c`) writes a space,
/// tab, newline and backslash in a path as an octal escape, a backslash and three digits. This
/// decodes each such escape in one pass, as `unmangle` in util-linux does, so `\134040` is the
/// text `\040` and not a space.
fn unescape_mount(field: &str) -> String {
    let mut bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    while let Some((&byte, rest)) = bytes.split_first() {
        if let (b'\\', [a @ b'0'..=b'3', b @ b'0'..=b'7', c @ b'0'..=b'7', tail @ ..]) =
            (byte, rest)
        {
            out.push(((a - b'0') << 6) | ((b - b'0') << 3) | (c - b'0'));
            bytes = tail;
        } else {
            out.push(byte);
            bytes = rest;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Moves the volumes that the uninstall would remove out of its way.
fn stash(root: &Path, volumes: &[Volume]) -> Result<(), InstallError> {
    for volume in volumes {
        let Some(stash) = volume.stash else { continue };
        let (path, stash) = (at(root, volume.path), at(root, stash));
        if path.symlink_metadata().is_err() {
            continue;
        }
        if stash.symlink_metadata().is_ok() {
            return Err(InstallError::StashExists { path, stash });
        }
        if let Err(source) = fs::rename(&path, &stash) {
            return Err(InstallError::Stash {
                path,
                stash,
                source,
            });
        }
    }
    Ok(())
}

/// Moves stashed volumes back to their place.
fn restore(root: &Path, volumes: &[Volume]) -> Result<(), InstallError> {
    for volume in volumes {
        let Some(stash) = volume.stash else { continue };
        let (path, stash) = (at(root, volume.path), at(root, stash));
        if stash.symlink_metadata().is_err() {
            continue;
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io(dir))?;
        }
        if let Err(source) = fs::rename(&stash, &path) {
            return Err(InstallError::Restore {
                path,
                stash,
                source,
            });
        }
    }
    Ok(())
}

/// Removes the volumes, in their place and stashed.
fn delete(root: &Path, volumes: &[Volume]) -> Result<(), InstallError> {
    for volume in volumes {
        for path in [Some(volume.path), volume.stash].into_iter().flatten() {
            remove_tree(&at(root, path))?;
        }
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
            && item.pending(root)
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
    use zeatend_plan::record::RECORD_FILE;

    use super::*;

    const K3S_UNINSTALL: &str = "/usr/local/bin/k3s-uninstall.sh";
    const K3S_AGENT_UNINSTALL: &str = "/usr/local/bin/k3s-agent-uninstall.sh";
    const RKE2_UNINSTALL: &str = "/usr/local/bin/rke2-uninstall.sh";
    const RKE2_KILLALL: &str = "/usr/local/bin/rke2-killall.sh";
    const K3S_LOCAL_PATH: &str = "/var/lib/rancher/k3s/storage";
    const RANCHER_STATE_DIR: &str = "/var/lib/rancher/zeatend";
    const KUBESOLO_BINARY: &str = "/usr/local/bin/kubesolo";
    const KUBESOLO_DATA_DIR: &str = "/var/lib/kubesolo";
    const KUBESOLO_STATE_DIR: &str = "/var/lib/zeatend";
    const KUBESOLO_KUBECTL: &str = "/var/lib/zeatend/bin/kubectl";
    const KUBESOLO_LOCAL_PATH: &str = "/var/lib/kubesolo/local-path-storage";

    fn tree_paths(node: Node) -> Vec<String> {
        node.paths()
            .unwrap()
            .into_iter()
            .filter_map(|r| match r {
                Remove::Tree(path) => Some(path),
                Remove::EmptyDir(_) => None,
            })
            .collect()
    }

    const K3S_SERVER: Node = Node::new(Distribution::K3s, Role::Server, false);
    const K3S_WORKER: Node = Node::new(Distribution::K3s, Role::Worker, true);
    const RKE2_FIRST: Node = Node::new(Distribution::Rke2, Role::Server, false);
    const RKE2_JOINED: Node = Node::new(Distribution::Rke2, Role::Server, true);
    const RKE2_WORKER: Node = Node::new(Distribution::Rke2, Role::Worker, true);
    const KUBESOLO: Node = Node::new(Distribution::KubeSolo, Role::Server, false);

    #[test]
    fn the_node_is_found_by_its_uninstall_script() {
        let host = |paths: &'static [&'static str]| detect(move |p| paths.contains(&p));
        assert_eq!(host(&[]), Vec::new());
        assert_eq!(host(&[K3S_UNINSTALL]), vec![K3S_SERVER]);
        assert_eq!(
            host(&[K3S_UNINSTALL, "/etc/rancher/k3s/zeatend-join-token"]),
            vec![Node::new(Distribution::K3s, Role::Server, true)]
        );
        assert_eq!(host(&[K3S_AGENT_UNINSTALL]), vec![K3S_WORKER]);
        let paths = Rancher::Rke2.paths();
        assert_eq!(host(&[RKE2_UNINSTALL]), vec![RKE2_WORKER]);
        assert_eq!(
            host(&[RKE2_UNINSTALL, "/var/lib/rancher/rke2/server/token"]),
            vec![RKE2_FIRST]
        );
        assert_eq!(
            paths.server_token_file,
            "/var/lib/rancher/rke2/server/token"
        );
        assert_eq!(
            host(&[
                RKE2_UNINSTALL,
                "/var/lib/rancher/rke2/server/token",
                "/etc/rancher/rke2/zeatend-join-token"
            ]),
            vec![RKE2_JOINED]
        );
        assert_eq!(
            host(&[K3S_AGENT_UNINSTALL, "/etc/rancher/k3s/zeabur-join-token"]),
            vec![K3S_WORKER],
            "a node zeabur-installer joined"
        );
        assert_eq!(
            host(&[
                RKE2_UNINSTALL,
                "/var/lib/rancher/rke2/server/token",
                "/etc/rancher/rke2/zeabur-join-token"
            ]),
            vec![RKE2_JOINED]
        );
        assert_eq!(host(&[KUBESOLO_BINARY]), vec![KUBESOLO]);
        assert_eq!(host(&[kubesolo::UNIT_FILE]), vec![KUBESOLO]);
        // Volumes that an earlier uninstall kept are not an install.
        assert_eq!(host(&[KUBESOLO_DATA_DIR, K3S_LOCAL_PATH]), Vec::new());
    }

    #[test]
    fn the_scripts_are_where_the_releases_put_them() {
        let rke2 = Distribution::Rke2.layout();
        for script in [rke2.server_uninstall, rke2.agent_uninstall, rke2.killall] {
            assert!(script.is_some_and(|s| s.starts_with(zeatend_plan::rke2::PREFIX)));
        }
        assert_eq!(rke2.killall, Some(RKE2_KILLALL));
        assert_eq!(parent(K3S_UNINSTALL), parent(k3s::BINARY));
        assert_eq!(K3S_SERVER.script(), Some(K3S_UNINSTALL));
        assert_eq!(K3S_WORKER.script(), Some(K3S_AGENT_UNINSTALL));
        assert_eq!(RKE2_WORKER.script(), Some(RKE2_UNINSTALL));
        assert_eq!(KUBESOLO.script(), None);
        assert!(KUBESOLO_LOCAL_PATH.starts_with(KUBESOLO_DATA_DIR));
        assert_eq!(
            Distribution::KubeSolo.layout().local_path,
            Some(KUBESOLO_LOCAL_PATH)
        );
    }

    #[test]
    fn a_k3s_server_loses_zeatends_manifests_config_and_state() {
        let paths = tree_paths(K3S_SERVER);
        for path in [
            RANCHER_STATE_DIR,
            "/etc/sysctl.d/60-k3s-cis.conf",
            "/etc/rancher/k3s/config.yaml",
            "/etc/rancher/k3s/audit-policy.yaml",
            "/etc/rancher/k3s/admission.yaml",
            "/etc/rancher/k3s/config.yaml.d/15-node-name.yaml",
            "/etc/rancher/k3s/config.yaml.d/20-node-ip.yaml",
            "/var/lib/rancher/k3s/server/manifests/zeabur-cis.yaml",
            "/var/lib/rancher/k3s/server/manifests/zeatend-servicelb-policy.yaml",
            k3s::MEMORY_DROP_IN,
            k3s::AUDIT_LOG_DIR,
            APPARMOR_PROFILE,
        ] {
            assert!(paths.contains(&path.to_owned()), "{path}");
        }
        assert!(!paths.iter().any(|p| p.contains("join")), "{paths:?}");
        assert!(K3S_SERVER.paths().unwrap().contains(&Remove::EmptyDir(
            "/etc/systemd/system/k3s.service.d".to_owned()
        )));
    }

    #[test]
    fn a_k3s_worker_loses_its_join_token() {
        let paths = tree_paths(K3S_WORKER);
        for path in [
            RANCHER_STATE_DIR,
            "/etc/sysctl.d/60-k3s-cis.conf",
            "/etc/rancher/k3s/config.yaml",
            "/etc/rancher/k3s/zeatend-join-token",
            "/etc/rancher/k3s/config.yaml.d/10-join.yaml",
            "/etc/rancher/k3s/config.yaml.d/15-node-name.yaml",
        ] {
            assert!(paths.contains(&path.to_owned()), "{path}");
        }
        assert!(!paths.iter().any(|p| p.contains("manifests")), "{paths:?}");
        assert!(!paths.contains(&k3s::MEMORY_DROP_IN.to_owned()));
    }

    #[test]
    fn what_zeabur_installer_left_is_removed_too() {
        assert!(tree_paths(K3S_SERVER).contains(&"/var/lib/rancher/zeabur-installer".to_owned()));
        assert!(
            tree_paths(RKE2_JOINED).contains(&"/etc/rancher/rke2/zeabur-join-token".to_owned())
        );
        let solo = tree_paths(KUBESOLO);
        for path in [
            "/var/lib/zeabur-installer",
            "/etc/zeabur-installer/kubeconfig.yaml",
            "/usr/local/libexec/zeabur-kubesolo-webhook",
        ] {
            assert!(solo.contains(&path.to_owned()), "{path}");
        }
        let text = describe(
            &plan(vec![KUBESOLO]).unwrap(),
            &[],
            &[],
            false,
            &[installer::CONFIG_FILE],
        );
        assert!(
            text.contains("/etc/zeabur-installer/config.yaml  (zeabur-installer's configuration)"),
            "{text}"
        );
    }

    #[test]
    fn rke2_servers_and_workers_lose_their_own_files() {
        let first = tree_paths(RKE2_FIRST);
        for path in [
            RANCHER_STATE_DIR,
            "/etc/sysctl.d/60-rke2-cis.conf",
            "/etc/rancher/rke2/config.yaml",
            "/etc/rancher/rke2/audit-policy.yaml",
            "/var/lib/rancher/rke2/server/manifests/zeatend-servicelb-policy.yaml",
        ] {
            assert!(first.contains(&path.to_owned()), "{path}");
        }
        assert!(!first.iter().any(|p| p.contains("join")), "{first:?}");
        let joined = tree_paths(RKE2_JOINED);
        assert!(joined.contains(&"/etc/rancher/rke2/zeatend-join-token".to_owned()));
        assert!(joined.contains(&"/etc/rancher/rke2/config.yaml.d/10-join.yaml".to_owned()));
        let worker = tree_paths(RKE2_WORKER);
        assert!(worker.contains(&"/etc/rancher/rke2/zeatend-join-token".to_owned()));
        assert!(
            !worker.iter().any(|p| p.contains("manifests")),
            "{worker:?}"
        );
        assert!(!worker.iter().any(|p| p.contains("k3s")), "{worker:?}");
    }

    #[test]
    fn kubesolo_is_removed_by_zeatend_itself() {
        let paths = tree_paths(KUBESOLO);
        for path in [
            kubesolo::UNIT_FILE,
            KUBESOLO_BINARY,
            kubesolo::CONFIG_FILE,
            KUBESOLO_DATA_DIR,
            KUBESOLO_STATE_DIR,
            kubesolo::KUBECONFIG,
            KUBESOLO_CNI_CONFIG,
        ] {
            assert!(paths.contains(&path.to_owned()), "{path}");
        }
        assert!(!paths.contains(&RANCHER_STATE_DIR.to_owned()));
        // The person's configuration in /etc/zeatend stays; the directory goes only when empty.
        assert!(!paths.contains(&load::DEFAULT_CONFIG.to_owned()));
        assert!(
            KUBESOLO
                .paths()
                .unwrap()
                .contains(&Remove::EmptyDir("/etc/zeatend".to_owned()))
        );
    }

    #[test]
    fn volumes_are_named_for_each_distribution() {
        let paths = |node: Node| node.volumes().iter().map(|v| v.path).collect::<Vec<_>>();
        assert_eq!(
            paths(K3S_SERVER),
            [K3S_LOCAL_PATH, ZEABUR_STORAGE, LONGHORN]
        );
        assert_eq!(paths(RKE2_WORKER), [ZEABUR_STORAGE, LONGHORN]);
        assert_eq!(
            paths(KUBESOLO),
            [KUBESOLO_LOCAL_PATH, ZEABUR_STORAGE, LONGHORN]
        );
        // Only the volumes inside a directory the uninstall removes move out of its way.
        for node in [K3S_SERVER, K3S_WORKER, RKE2_FIRST, KUBESOLO] {
            for volume in node.volumes() {
                assert_eq!(
                    volume.stash.is_some(),
                    [K3S_LOCAL_PATH, KUBESOLO_LOCAL_PATH].contains(&volume.path)
                );
            }
        }
    }

    #[test]
    fn a_host_without_a_node_is_checked_for_what_any_node_left() {
        let leftovers = plan(Vec::new()).unwrap();
        assert_eq!(leftovers.nodes, Vec::new());
        for path in [RANCHER_STATE_DIR, KUBESOLO_STATE_DIR] {
            assert!(leftovers.remove.contains(&Remove::Tree(path.to_owned())));
        }
        let paths: Vec<&str> = leftovers.volumes.iter().map(|v| v.path).collect();
        for path in [
            K3S_LOCAL_PATH,
            KUBESOLO_LOCAL_PATH,
            ZEABUR_STORAGE,
            LONGHORN,
        ] {
            assert!(paths.contains(&path));
        }
        let first_empty_dir = leftovers
            .remove
            .iter()
            .position(|r| matches!(r, Remove::EmptyDir(_)))
            .unwrap();
        assert!(
            leftovers.remove[first_empty_dir..]
                .iter()
                .all(|r| matches!(r, Remove::EmptyDir(_)))
        );
    }

    #[test]
    fn the_description_names_what_goes_and_what_stays() {
        let k3s = plan(vec![K3S_SERVER]).unwrap();
        let text = describe(
            &k3s,
            &[RANCHER_STATE_DIR],
            &[K3S_LOCAL_PATH],
            false,
            &[load::DEFAULT_CONFIG],
        );
        assert!(
            text.contains("removes K3s (server) from this host"),
            "{text}"
        );
        assert!(text.contains(K3S_UNINSTALL), "{text}");
        assert!(text.contains(&format!("  {RANCHER_STATE_DIR}\n")), "{text}");
        assert!(
            text.contains(&format!(
                "These stay:\n  {K3S_LOCAL_PATH}  (volumes; --delete-volumes removes them)\n  {}",
                load::DEFAULT_CONFIG
            )),
            "{text}"
        );
        let deleting = describe(&k3s, &[], &[K3S_LOCAL_PATH, LONGHORN], true, &[]);
        assert!(
            deleting.contains(&format!(
                "--delete-volumes removes the volumes, and the data in them:\n  {K3S_LOCAL_PATH}\n  {LONGHORN}\n"
            )),
            "{deleting}"
        );
        assert!(!deleting.contains("These stay"), "{deleting}");
        let rke2 = describe(&plan(vec![RKE2_FIRST]).unwrap(), &[], &[], false, &[]);
        assert!(rke2.contains("the etcd user"), "{rke2}");
        let solo = describe(&plan(vec![KUBESOLO]).unwrap(), &[], &[], false, &[]);
        assert!(solo.contains("zeatend stops KubeSolo"), "{solo}");
    }

    #[test]
    fn a_node_that_joined_is_deleted_from_a_server() {
        assert!(leave_text(Some("worker-1")).ends_with("kubectl delete node worker-1"));
        assert!(leave_text(None).ends_with("kubectl delete node <node name>"));
    }

    #[test]
    fn mounts_under_the_data_dir_come_deepest_first() {
        let mounts = "overlay /var/lib/kubesolo/containerd/a/rootfs overlay rw 0 0\n\
                      tmpfs /var/lib/kubesolo/kubelet/pods/x/volumes/my\\040vol tmpfs rw 0 0\n\
                      /dev/sda1 /var/lib/kubesolo-other ext4 rw 0 0\n\
                      /dev/sda2 /var/lib/kubesolo ext4 rw 0 0\n";
        assert_eq!(
            mounts_under(mounts, KUBESOLO_DATA_DIR),
            [
                "/var/lib/kubesolo/kubelet/pods/x/volumes/my vol",
                "/var/lib/kubesolo/containerd/a/rootfs",
                "/var/lib/kubesolo",
            ]
        );
    }

    #[test]
    fn mount_escapes_are_decoded_once() {
        assert_eq!(unescape_mount("/a\\040b\\011c\\012d"), "/a b\tc\nd");
        assert_eq!(unescape_mount("/a\\134040"), "/a\\040");
        assert_eq!(unescape_mount("/a\\04"), "/a\\04");
        assert_eq!(unescape_mount("/a\\"), "/a\\");
        assert_eq!(unescape_mount("/é\\040"), "/é ");
    }

    #[test]
    fn kubesolo_unmounts_only_its_runtime_mounts() {
        let mounts = "overlay /var/lib/kubesolo/containerd/a/rootfs overlay rw 0 0\n\
                      /dev/sdb1 /var/lib/kubesolo/local-path-storage ext4 rw 0 0\n\
                      /dev/sdb2 /var/lib/kubesolo/local-path-storage/pvc-1 ext4 rw 0 0\n\
                      /dev/sda2 /var/lib/kubesolo ext4 rw 0 0\n";
        assert_eq!(
            runtime_mounts(mounts),
            ["/var/lib/kubesolo/containerd/a/rootfs"]
        );
    }

    #[test]
    fn volumes_on_their_own_file_system_stop_the_uninstall() {
        let k3s = Node::new(Distribution::K3s, Role::Server, false).volumes();
        let solo = Node::new(Distribution::KubeSolo, Role::Server, false).volumes();
        let runtime = "overlay /run/k3s/containerd/a/rootfs overlay rw 0 0\n\
                       /dev/sda1 /var/lib/rancher ext4 rw 0 0\n\
                       /dev/sdc1 /var/lib/longhorn ext4 rw 0 0\n";
        // A disk for all of /var/lib/rancher holds the stash too; Longhorn's disk stays mounted.
        assert_eq!(volume_mounts(runtime, &k3s, false), Vec::new());
        assert_eq!(
            volume_mounts(runtime, &k3s, true),
            [("/var/lib/longhorn".to_owned(), LONGHORN)]
        );
        for mount in [
            "/var/lib/rancher/k3s",
            "/var/lib/rancher/k3s/storage",
            "/var/lib/rancher/k3s/storage/pvc-1",
        ] {
            let mounts = format!("/dev/sdb1 {mount} ext4 rw 0 0\n");
            for delete in [false, true] {
                assert_eq!(
                    volume_mounts(&mounts, &k3s, delete),
                    [(mount.to_owned(), K3S_LOCAL_PATH)],
                    "{mount}"
                );
            }
        }
        for mount in ["/var/lib/kubesolo", "/var/lib/kubesolo/local-path-storage"] {
            let mounts = format!("/dev/sdb1 {mount} ext4 rw 0 0\n");
            assert_eq!(
                volume_mounts(&mounts, &solo, false),
                [(mount.to_owned(), KUBESOLO_LOCAL_PATH)],
                "{mount}"
            );
        }
    }

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

    fn touch(root: &Path, path: &str) {
        let path = at(root, path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
    }

    /// What `run` does after the nodes stopped, with the uninstall script's removal in between.
    fn clean(root: &Path, plan: &Uninstall, delete_volumes: bool, script_removes: &str) {
        if !delete_volumes {
            stash(root, &plan.volumes).unwrap();
        }
        remove_tree(&at(root, script_removes)).unwrap();
        remove_trees(root, &plan.remove).unwrap();
        if delete_volumes {
            delete(root, &plan.volumes).unwrap();
        } else {
            restore(root, &plan.volumes).unwrap();
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
        let k3s = plan(vec![K3S_SERVER]).unwrap();
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
        let again = plan(detect(|p| exists(root, p))).unwrap();
        assert_eq!(again.nodes, Vec::new());
        assert!(!again.remove.iter().any(|r| r.pending(root)), "{again:?}");
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
        let solo = plan(vec![KUBESOLO]).unwrap();
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
        clean(root, &plan(vec![KUBESOLO]).unwrap(), false, "/nothing");
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
        let leftovers = plan(Vec::new()).unwrap();
        assert!(leftovers.volumes.iter().any(|v| v.present(root)));
        clean(root, &leftovers, false, "/nothing");
        assert!(exists(root, &format!("{K3S_LOCAL_PATH}/pvc-1/data")));
    }
}
