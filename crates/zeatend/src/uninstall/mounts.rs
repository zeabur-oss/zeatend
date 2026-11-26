//! The mounts that the uninstall unmounts or must not move.

use std::path::Path;

use zeatend_plan::cluster::Distribution;
use zeatend_plan::uninstall::Volume;

/// The mounts of `KubeSolo`'s containers and pods under its data directory, deepest first: not
/// the data directory itself and not its local-path volumes, whose data the uninstall keeps or
/// removes first.
pub(super) fn runtime_mounts(mounts: &str) -> Vec<String> {
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
pub(super) fn volume_mounts(
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
pub(super) fn mounts_under(mounts: &str, dir: &str) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use zeatend_plan::config::Role;
    use zeatend_plan::uninstall::{LONGHORN, Node};

    use super::*;

    const KUBESOLO_DATA_DIR: &str = "/var/lib/kubesolo";
    const K3S_LOCAL_PATH: &str = "/var/lib/rancher/k3s/storage";
    const KUBESOLO_LOCAL_PATH: &str = "/var/lib/kubesolo/local-path-storage";

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
}
