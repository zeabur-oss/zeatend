//! `KubeSolo`'s pod network: the namespaces, bridge and CNI state its plugins leave.

use std::fs;
use std::path::Path;

use zeatend_host::cmd;
use zeatend_plan::cluster::Distribution;
use zeatend_plan::cni::{KUBELET_DIR, PodNetwork, RESULTS_DIR, Removal};
use zeatend_plan::kubesolo;
use zeatend_plan::uninstall::Node;

use super::{at, exists, mounts, remove_tree};
use crate::{InstallError, output};

/// What the pod network holds on the host, read before the footprint removes
/// [`kubesolo::CNI_CONFIG`].
#[derive(Debug)]
pub(super) enum PodState {
    /// No `KubeSolo` and no configuration: nothing to do.
    None,
    /// `KubeSolo` is on the host, but its configuration is missing or unreadable.
    Unread,
    /// What to remove.
    Remove(Removal),
}

impl PodState {
    /// Reads the configuration and the results under `root`.
    pub(super) fn read(root: &Path, nodes: &[Node]) -> Self {
        let kubesolo_node = nodes
            .iter()
            .any(|n| n.distribution == Distribution::KubeSolo);
        let config = fs::read_to_string(at(root, kubesolo::CNI_CONFIG)).ok();
        let Some(network) = config.as_deref().and_then(PodNetwork::parse) else {
            return if kubesolo_node || config.is_some() {
                Self::Unread
            } else {
                Self::None
            };
        };
        let namespaces = results(root, &network)
            .iter()
            .filter_map(|file| fs::read_to_string(file).ok())
            .filter_map(|text| network.result_netns(&text))
            .collect();
        let alone = nodes
            .iter()
            .all(|n| n.distribution == Distribution::KubeSolo);
        Self::Remove(Removal::new(network, namespaces, alone))
    }

    /// The text `uninstall` shows before it asks.
    pub(super) fn describe(&self) -> Option<String> {
        match self {
            Self::None => None,
            Self::Unread => Some(zeatend_plan::cni::unread_text()),
            Self::Remove(removal) => Some(removal.describe()),
        }
    }
}

/// The result files of `network` under `root`.
fn results(root: &Path, network: &PodNetwork) -> Vec<std::path::PathBuf> {
    fs::read_dir(at(root, RESULTS_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| network.owns_result(n))
        })
        .map(|e| e.path())
        .collect()
}

/// Removes the pod network after `KubeSolo` stopped. A namespace or bridge that is already gone
/// is no error.
pub(super) async fn remove(root: &Path, removal: &Removal) -> Result<(), InstallError> {
    output::step(&format!(
        "Removing KubeSolo's pod network {}",
        removal.network.name
    ));
    for namespace in &removal.namespaces {
        if exists(root, &format!("/run/netns/{namespace}")) {
            cmd::run("ip", &["netns", "delete", namespace], None).await?;
        }
    }
    if let Some(bridge) = &removal.bridge
        && exists(root, &format!("/sys/class/net/{bridge}"))
    {
        cmd::run("ip", &["link", "delete", bridge], None).await?;
    }
    for file in results(root, &removal.network) {
        remove_tree(&file)?;
    }
    remove_tree(&at(root, &removal.network.addresses_dir()))?;
    for dir in [RESULTS_DIR, zeatend_plan::cni::NETWORKS_DIR, "/var/lib/cni"] {
        // Only when empty: another network may keep files there.
        drop(fs::remove_dir(at(root, dir)));
    }
    if removal.kubelet {
        remove_kubelet(root)?;
    }
    Ok(())
}

/// Removes [`KUBELET_DIR`], unless something is still mounted in it.
fn remove_kubelet(root: &Path) -> Result<(), InstallError> {
    let mounts = fs::read_to_string("/proc/mounts").map_err(super::io("/proc/mounts"))?;
    if let Some(mount) = mounts::mounts_under(&mounts, KUBELET_DIR).first() {
        output::err(&format!(
            "    warning: {mount} is still mounted, so {KUBELET_DIR} stays"
        ));
        return Ok(());
    }
    remove_tree(&at(root, KUBELET_DIR))
}

#[cfg(test)]
mod tests {
    use zeatend_plan::config::Role;

    use super::*;

    const KUBESOLO: Node = Node::new(Distribution::KubeSolo, Role::Server, false);
    const K3S: Node = Node::new(Distribution::K3s, Role::Server, false);

    fn write(root: &Path, path: &str, text: &str) {
        let path = at(root, path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn conflist(root: &Path) {
        write(
            root,
            kubesolo::CNI_CONFIG,
            r#"{"name":"kubesolo-net","plugins":[{"type":"bridge","bridge":"cni0"}]}"#,
        );
    }

    #[test]
    fn the_namespaces_come_from_the_networks_own_results() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        conflist(root);
        let result = |ns: &str, net: &str| {
            format!(r#"{{"networkName":"{net}","netns":"/var/run/netns/{ns}"}}"#)
        };
        write(
            root,
            &format!("{RESULTS_DIR}/kubesolo-net-a-eth0"),
            &result("cni-a", "kubesolo-net"),
        );
        write(
            root,
            &format!("{RESULTS_DIR}/kubesolo-net-b-eth0"),
            &result("cni-b", "kubesolo-net"),
        );
        write(
            root,
            &format!("{RESULTS_DIR}/cbr0-c-eth0"),
            &result("cni-c", "cbr0"),
        );
        let PodState::Remove(removal) = PodState::read(root, &[KUBESOLO]) else {
            panic!("no removal");
        };
        assert_eq!(removal.namespaces, ["cni-a", "cni-b"]);
        assert_eq!(removal.bridge.as_deref(), Some("cni0"));
        assert!(removal.kubelet);

        let PodState::Remove(shared) = PodState::read(root, &[KUBESOLO, K3S]) else {
            panic!("no removal");
        };
        assert_eq!(shared.bridge, None);
        assert!(!shared.kubelet);
    }

    #[test]
    fn a_missing_configuration_removes_nothing_of_the_network() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(matches!(
            PodState::read(root, &[KUBESOLO]),
            PodState::Unread
        ));
        assert!(matches!(PodState::read(root, &[K3S]), PodState::None));
        write(root, kubesolo::CNI_CONFIG, "not json");
        assert!(matches!(PodState::read(root, &[]), PodState::Unread));
    }
}
