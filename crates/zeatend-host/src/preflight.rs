//! Checks of the host: before `apply` changes anything, and by `doctor` on a host with a cluster.
//! The checks produce data; an interface words each [`Finding`].

use std::fs;
use std::net::{Ipv4Addr, TcpListener};
use std::path::Path;

use zeatend_plan::cluster::{Distribution, PortUse};
use zeatend_plan::config::Role;
use zeatend_plan::finding::{BusyPort, Check, Finding, Leftover};
use zeatend_plan::kubesolo;

use crate::facts::{self, HostAddress};
use crate::{cmd, systemd};

/// Whether a disk is encrypted, as far as the block devices tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encryption {
    /// A dm-crypt device is under it.
    Encrypted,
    /// No dm-crypt device is under it.
    Plain,
    /// The devices could not be read.
    Unknown,
}

/// What the checks need to know.
#[derive(Debug, Clone, Copy)]
pub struct Options<'a> {
    /// The cluster from an earlier `apply` is running, so its ports are expected to be in use.
    pub cluster_running: bool,
    /// Fail on a disk that is not encrypted.
    pub require_encrypted_disk: bool,
    /// What this host runs.
    pub distribution: Distribution,
    /// A server also listens on the API and join ports. `KubeSolo` is a server.
    pub role: Role,
    /// The interface the node uses, `network.interface`.
    pub interface: Option<&'a str>,
}

/// Where a node of `distribution` keeps secrets or data: the root file system, the data
/// directory, and the configuration with its tokens and kubeconfigs.
fn secret_paths(distribution: Distribution) -> Vec<&'static str> {
    let layout = distribution.layout();
    let mut paths = vec!["/", layout.data_dir, layout.config_dir];
    paths.extend(
        layout
            .kubeconfig
            .and_then(|f| Path::new(f).parent()?.to_str()),
    );
    paths
}

/// Files left by Kubernetes distributions, and the id of the distribution that owns each, which
/// an install of the same distribution does not report.
const OTHERS: [(&str, &str, Option<&str>); 9] = [
    ("/usr/local/bin/k3s", "K3s", Some("k3s")),
    ("/etc/rancher/k3s", "K3s", Some("k3s")),
    ("/etc/kubernetes/admin.conf", "kubeadm", None),
    ("/var/lib/k0s", "k0s", None),
    ("/snap/bin/microk8s", "MicroK8s", None),
    ("/usr/local/bin/rke2", "RKE2", Some("rke2")),
    ("/etc/rancher/rke2", "RKE2", Some("rke2")),
    (
        Distribution::KubeSolo.layout().binary,
        "KubeSolo",
        Some("kubesolo"),
    ),
    (
        Distribution::KubeSolo.layout().data_dir,
        "KubeSolo",
        Some("kubesolo"),
    ),
];
const DISK_NEEDED_GIB: u64 = 10;
const DISK_ROOMY_GIB: u64 = 30;

/// Runs every check.
#[must_use]
pub async fn run(options: Options<'_>) -> Vec<Check> {
    let mut checks = vec![
        root(),
        systemd(),
        time_sync().await,
        swap(),
        crash_kernel(),
        other_distributions(options.distribution),
    ];
    if options.distribution == Distribution::KubeSolo {
        checks.push(docker().await);
    }
    if !options.cluster_running {
        checks.push(ports(options.distribution, options.role));
    }
    if let Some(interface) = options.interface {
        checks.push(network_interface(interface).await);
    }
    checks.extend(iptables().await);
    checks.push(disk_space(options.distribution.layout().data_dir).await);
    checks.push(encryption(options.distribution, options.require_encrypted_disk).await);
    checks
}

/// Whether this process runs as root.
#[must_use]
pub fn is_root() -> bool {
    effective_uid(&fs::read_to_string("/proc/self/status").unwrap_or_default()) == Some(0)
}

fn root() -> Check {
    if is_root() {
        Finding::RootOk
    } else {
        Finding::RootNo
    }
    .into()
}

fn effective_uid(status: &str) -> Option<u32> {
    let line = status.lines().find(|l| l.starts_with("Uid:"))?;
    line.split_whitespace().nth(2)?.parse().ok()
}

fn systemd() -> Check {
    if Path::new("/run/systemd/system").is_dir() {
        Finding::SystemdOk
    } else {
        Finding::SystemdNo
    }
    .into()
}

async fn time_sync() -> Check {
    match cmd::run(
        "timedatectl",
        &["show", "--property=NTPSynchronized", "--value"],
        None,
    )
    .await
    {
        Ok(out) if out.trim() == "yes" => Finding::TimeOk,
        Ok(_) => Finding::TimeDrift,
        Err(_) => Finding::TimeUnknown,
    }
    .into()
}

fn swap() -> Check {
    match fs::read_to_string("/proc/swaps") {
        Ok(swaps) if swaps.lines().skip(1).any(|l| !l.trim().is_empty()) => Finding::SwapOn,
        Ok(_) => Finding::SwapOff,
        Err(_) => Finding::SwapUnknown,
    }
    .into()
}

fn crash_kernel() -> Check {
    crash_reserve(
        fs::read_to_string("/sys/kernel/kexec_crash_size")
            .ok()
            .as_deref(),
    )
    .into()
}

fn crash_reserve(raw: Option<&str>) -> Finding {
    let mib = raw
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .map_or(0, |bytes| bytes >> 20);
    if mib == 0 {
        Finding::CrashNone
    } else {
        Finding::CrashKept { mib }
    }
}

fn other_distributions(distribution: Distribution) -> Check {
    let found: Vec<Leftover> = OTHERS
        .iter()
        .filter(|(_, _, own)| own.is_none_or(|own| own != distribution.id()))
        .filter(|(path, ..)| Path::new(path).exists())
        .map(|&(path, distribution, _)| Leftover { distribution, path })
        .collect();
    if found.is_empty() {
        Finding::OthersNone
    } else {
        Finding::OthersFound { found }
    }
    .into()
}

/// Where Docker's binary and socket are. Docker sets the iptables FORWARD policy to DROP, so
/// Pods of `KubeSolo` cannot reach the internet.
const DOCKER_PATHS: [&str; 5] = [
    "/usr/bin/docker",
    "/usr/local/bin/docker",
    "/snap/bin/docker",
    "/run/docker.sock",
    "/var/run/docker.sock",
];
const DOCKER_UNITS: [&str; 2] = ["docker.service", "docker.socket"];

async fn docker() -> Check {
    let mut found: Vec<String> = DOCKER_PATHS
        .iter()
        .filter(|path| Path::new(path).exists())
        .map(|&path| path.to_owned())
        .collect();
    for unit in DOCKER_UNITS {
        if systemd::is_active(unit).await {
            found.push(unit.to_owned());
        }
    }
    let iptables = cmd::run("iptables", &["-S", "FORWARD"], None)
        .await
        .unwrap_or_default();
    docker_check(found, &iptables).into()
}

fn docker_check(found: Vec<String>, iptables: &str) -> Finding {
    if !found.is_empty() {
        Finding::DockerFound { found }
    } else if iptables.lines().any(|l| l.trim() == "-P FORWARD DROP") {
        Finding::ForwardDrop
    } else {
        Finding::DockerNone
    }
}

/// The programs that print the host's IPv4 and IPv6 rules.
const IPTABLES_SAVE: [&str; 2] = ["iptables-save", "ip6tables-save"];

/// Whether `iptables-save` and `ip6tables-save` read the host's rules; `None` on a host without
/// them. Rules that another program writes with nftables into the tables iptables-nft owns, such
/// as Tailscale in its nftables mode, can make them fail, and then Calico and kube-proxy fail too.
async fn iptables() -> Option<Check> {
    iptables_in(&std::env::var_os("PATH").unwrap_or_default()).await
}

/// [`iptables`] with the programs found in `path`, a `PATH` value.
async fn iptables_in(path: &std::ffi::OsStr) -> Option<Check> {
    let programs: Vec<std::path::PathBuf> = IPTABLES_SAVE
        .iter()
        .filter_map(|program| {
            std::env::split_paths(path)
                .map(|dir| dir.join(program))
                .find(|file| file.is_file())
        })
        .collect();
    if programs.is_empty() {
        return None;
    }
    for program in programs {
        if let Err(e) = cmd::run(&program, &[], None).await {
            let command = program
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let message = e.to_string();
            let stderr: Vec<&str> = message.lines().skip(1).collect();
            let error = if stderr.is_empty() {
                message.clone()
            } else {
                stderr.join(" ")
            };
            return Some(Finding::IptablesUnreadable { command, error }.into());
        }
    }
    Some(Finding::IptablesOk.into())
}

/// Tailscale's addresses: the shared address space of RFC 6598, `100.64.0.0/10`
/// (<https://tailscale.com/kb/1015/100.x-addresses>).
const TAILSCALE: (Ipv4Addr, u32) = (Ipv4Addr::new(100, 64, 0, 0), 10);

/// The interface of `network.interface`: it exists, has an address the node registers with, and
/// is not Tailscale's.
async fn network_interface(interface: &str) -> Check {
    let exists = Path::new("/sys/class/net").join(interface).exists();
    interface_check(interface, exists, &facts::host_addresses().await).into()
}

/// [`network_interface`] from what it reads.
///
/// Tailscale offers every local address as a direct endpoint, `flannel.1`'s too, so flannel's
/// VXLAN on `tailscale0` sends its packets back into the tunnel. K3s's own Tailscale
/// integration, `vpn-auth`, is the way to run a cluster over Tailscale.
fn interface_check(interface: &str, exists: bool, addresses: &[HostAddress]) -> Finding {
    let name = interface.to_owned();
    if !exists {
        return Finding::InterfaceMissing { interface: name };
    }
    let (net, bits) = TAILSCALE;
    let mask = u32::MAX << (32 - bits);
    let tailscale = addresses
        .iter()
        .filter(|a| a.interface == interface)
        .find(|a| {
            a.address
                .parse::<Ipv4Addr>()
                .is_ok_and(|ip| u32::from(ip) & mask == u32::from(net))
        });
    if let Some(found) = tailscale {
        return Finding::InterfaceTailscale {
            interface: name,
            address: found.address.clone(),
        };
    }
    let chosen = facts::choose_node_ips(addresses, Some(interface), Some(interface));
    if chosen.is_empty() {
        Finding::InterfaceNoAddress { interface: name }
    } else {
        Finding::InterfaceOk {
            interface: name,
            addresses: chosen,
        }
    }
}

/// The TCP ports a node needs free.
#[must_use]
fn needed_ports(distribution: Distribution, role: Role) -> Vec<(u16, PortUse)> {
    distribution
        .rancher()
        .map_or_else(kubesolo::ports, |rancher| {
            rancher.node_ports(role == Role::Server)
        })
}

fn ports(distribution: Distribution, role: Role) -> Check {
    let ports = needed_ports(distribution, role);
    let busy: Vec<BusyPort> = ports
        .iter()
        .filter(|(port, _)| TcpListener::bind(("0.0.0.0", *port)).is_err())
        .map(|&(port, purpose)| BusyPort { port, purpose })
        .collect();
    if busy.is_empty() {
        Finding::PortsFree {
            ports: ports.iter().map(|(port, _)| *port).collect(),
        }
    } else {
        Finding::PortsBusy { ports: busy }
    }
    .into()
}

async fn disk_space(data_dir: &str) -> Check {
    let target = existing_ancestor(data_dir);
    let kib = cmd::run("df", &["-Pk", target], None)
        .await
        .ok()
        .and_then(|out| available_kib(&out));
    disk_finding(kib, target).into()
}

fn disk_finding(kib: Option<u64>, target: &str) -> Finding {
    let path = target.to_owned();
    let Some(kib) = kib else {
        return Finding::DiskUnknown { path };
    };
    let free_tenths_gib = tenths_gib(kib);
    if kib < DISK_NEEDED_GIB << 20 {
        Finding::DiskLow {
            free_tenths_gib,
            path,
            needed_gib: DISK_NEEDED_GIB,
        }
    } else if kib < DISK_ROOMY_GIB << 20 {
        Finding::DiskTight {
            free_tenths_gib,
            path,
            roomy_gib: DISK_ROOMY_GIB,
        }
    } else {
        Finding::DiskOk {
            free_tenths_gib,
            path,
        }
    }
}

/// KiB as tenths of a GiB, rounded down.
const fn tenths_gib(kib: u64) -> u64 {
    kib * 10 / (1 << 20)
}

fn available_kib(df: &str) -> Option<u64> {
    df.lines().nth(1)?.split_whitespace().nth(3)?.parse().ok()
}

/// The nearest of `path` and its parents that exists, so a directory the install has yet to make
/// is checked on the file system it will be on.
fn existing_ancestor(path: &str) -> &str {
    Path::new(path)
        .ancestors()
        .find(|p| p.exists())
        .and_then(Path::to_str)
        .unwrap_or("/")
}

/// The first path that is not encrypted, or else the first that could not be told, or else
/// `Encrypted`, naming the paths.
fn least_encrypted(found: &[(&'static str, Encryption)]) -> (Encryption, Vec<&'static str>) {
    for worst in [Encryption::Plain, Encryption::Unknown] {
        if let Some((path, _)) = found.iter().find(|(_, e)| *e == worst) {
            return (worst, vec![*path]);
        }
    }
    (
        Encryption::Encrypted,
        found.iter().map(|(path, _)| *path).collect(),
    )
}

/// Whether every file system a node of `distribution` keeps secrets or data on is encrypted; each
/// one is read once.
async fn encryption(distribution: Distribution, required: bool) -> Check {
    let mut sources: Vec<String> = Vec::new();
    let mut found = Vec::new();
    for path in secret_paths(distribution) {
        let Some(source) = mount_source(existing_ancestor(path)).await else {
            found.push((path, Encryption::Unknown));
            continue;
        };
        if !sources.contains(&source) {
            found.push((path, device_encryption(&source).await));
            sources.push(source);
        }
    }
    encryption_finding(&found, required).into()
}

fn encryption_finding(found: &[(&'static str, Encryption)], required: bool) -> Finding {
    let (found, paths) = least_encrypted(found);
    let path = paths.first().copied().unwrap_or("/");
    match (found, required) {
        (Encryption::Encrypted, _) => Finding::CryptYes { paths },
        (Encryption::Plain, true) => Finding::CryptRequiredPlain { path },
        (Encryption::Unknown, true) => Finding::CryptRequiredUnknown { path },
        (Encryption::Plain, false) => Finding::CryptNo { path },
        (Encryption::Unknown, false) => Finding::CryptUnknown { path },
    }
}

/// The device of the file system that holds `path`; `None` when it cannot be read.
async fn mount_source(path: &str) -> Option<String> {
    let source = cmd::run(
        "findmnt",
        &["--noheadings", "--output", "SOURCE", "--target", path],
        None,
    )
    .await
    .ok()?;
    let source = source.trim();
    // Btrfs subvolumes read as `/dev/sda2[/@]`.
    Some(source.split('[').next().unwrap_or(source).to_owned())
}

/// Whether `device` has a dm-crypt device beneath it.
async fn device_encryption(device: &str) -> Encryption {
    match cmd::run(
        "lsblk",
        &["--noheadings", "--inverse", "--output", "TYPE", device],
        None,
    )
    .await
    {
        Ok(types) if types.split_whitespace().any(|t| t == "crypt") => Encryption::Encrypted,
        Ok(_) => Encryption::Plain,
        Err(_) => Encryption::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use zeatend_plan::finding::Status;

    use super::*;

    fn program(dir: &Path, name: &str, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        let file = dir.join(name);
        fs::write(&file, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[tokio::test]
    async fn rules_that_iptables_save_cannot_read_stop_apply() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(iptables_in(dir.path().as_os_str()).await, None);
        program(dir.path(), "iptables-save", "echo '*filter'");
        program(dir.path(), "ip6tables-save", "echo '*filter'");
        let ok = iptables_in(dir.path().as_os_str()).await.unwrap();
        assert_eq!(ok.finding, Finding::IptablesOk);
        program(
            dir.path(),
            "ip6tables-save",
            "echo 'Error: meta sreg is not an immediate' >&2\n\
             echo 'ip6tables-nft-save v1.8.11 (nf_tables): Parsing nftables rule failed' >&2\n\
             exit 1",
        );
        let failed = iptables_in(dir.path().as_os_str()).await.unwrap();
        assert_eq!(failed.status, Status::Fail);
        let Finding::IptablesUnreadable { command, error } = failed.finding else {
            panic!("{failed:?}");
        };
        assert_eq!(command, "ip6tables-save");
        assert!(error.contains("Parsing nftables rule failed"), "{error}");
    }

    #[test]
    fn the_interface_must_exist_hold_an_address_and_not_be_tailscale() {
        let addresses = facts::parse_addresses(
            "3: eth1    inet 10.0.0.7/24 brd 10.0.0.255 scope global eth1\\       valid_lft forever preferred_lft forever\n\
             4: tailscale0    inet 100.101.102.103/32 scope global tailscale0\\       valid_lft forever preferred_lft forever\n\
             5: eth2    inet6 fd00::aa/64 scope global temporary dynamic \\       valid_lft 1sec preferred_lft 1sec\n",
        );
        let ok = interface_check("eth1", true, &addresses);
        assert_eq!(
            ok,
            Finding::InterfaceOk {
                interface: "eth1".to_owned(),
                addresses: vec!["10.0.0.7".to_owned()],
            }
        );
        assert_eq!(ok.status(), Status::Pass);
        for (interface, exists, refused) in [
            (
                "eth9",
                false,
                Finding::InterfaceMissing {
                    interface: "eth9".to_owned(),
                },
            ),
            // Only a temporary IPv6 address, which the node does not register with.
            (
                "eth2",
                true,
                Finding::InterfaceNoAddress {
                    interface: "eth2".to_owned(),
                },
            ),
            (
                "eth3",
                true,
                Finding::InterfaceNoAddress {
                    interface: "eth3".to_owned(),
                },
            ),
            (
                "tailscale0",
                true,
                Finding::InterfaceTailscale {
                    interface: "tailscale0".to_owned(),
                    address: "100.101.102.103".to_owned(),
                },
            ),
        ] {
            let found = interface_check(interface, exists, &addresses);
            assert_eq!(found, refused);
            assert_eq!(found.status(), Status::Fail, "{interface}");
        }
        // Just outside 100.64.0.0/10.
        let near = facts::parse_addresses(
            "3: wg0    inet 100.128.0.1/24 scope global wg0\\       valid_lft forever preferred_lft forever\n",
        );
        assert_eq!(interface_check("wg0", true, &near).status(), Status::Pass);
    }

    #[test]
    fn docker_or_a_dropping_forward_policy_stops_the_install() {
        let docker = || vec!["/usr/bin/docker".to_owned()];
        let found = Finding::DockerFound { found: docker() };
        let policy = "-P FORWARD ACCEPT\n";
        let drop = "-P FORWARD DROP\n-A FORWARD -j DOCKER-USER\n";
        assert_eq!(docker_check(vec![], policy), Finding::DockerNone);
        assert_eq!(docker_check(vec![], ""), Finding::DockerNone);
        assert_eq!(docker_check(docker(), policy), found);
        assert_eq!(docker_check(vec![], drop), Finding::ForwardDrop);
        assert_eq!(docker_check(docker(), drop), found);
        assert_eq!(found.status(), Status::Fail);
        assert_eq!(Finding::ForwardDrop.status(), Status::Fail);
        assert_eq!(Finding::DockerNone.status(), Status::Pass);
    }

    #[test]
    fn parsing() {
        assert_eq!(effective_uid("Name:\tx\nUid:\t1000\t0\t0\t0\n"), Some(0));
        assert_eq!(effective_uid("Uid:\t0\t1000\t1000\t1000\n"), Some(1000));
        let df = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda2 61255492 9032 49094708 16% /\n";
        assert_eq!(available_kib(df), Some(49_094_708));
        assert_eq!(tenths_gib(3_250_586), 31);
        assert_eq!(tenths_gib(10 << 20), 100);
        assert_eq!(crash_reserve(None), Finding::CrashNone);
        assert_eq!(crash_reserve(Some("0\n")), Finding::CrashNone);
        assert_eq!(
            crash_reserve(Some("536870912\n")),
            Finding::CrashKept { mib: 512 }
        );
        assert_eq!(Finding::CrashKept { mib: 512 }.status(), Status::Warn);
        let ports = |distribution, role| -> Vec<u16> {
            needed_ports(distribution, role)
                .iter()
                .map(|(p, _)| *p)
                .collect()
        };
        assert_eq!(ports(Distribution::Rke2, Role::Worker), [80, 443, 10250]);
        assert_eq!(
            ports(Distribution::Rke2, Role::Server),
            [80, 443, 6443, 9345, 10250]
        );
        assert_eq!(
            ports(Distribution::K3s, Role::Server),
            [80, 443, 6443, 10250]
        );
        assert_eq!(
            ports(Distribution::KubeSolo, Role::Server),
            [80, 443, 6443, 10250, 10443]
        );
    }

    #[test]
    fn free_space_sets_the_outcome() {
        let path = || "/var/lib".to_owned();
        assert_eq!(
            disk_finding(None, "/var/lib"),
            Finding::DiskUnknown { path: path() }
        );
        assert_eq!(
            disk_finding(Some(3_250_586), "/var/lib"),
            Finding::DiskLow {
                free_tenths_gib: 31,
                path: path(),
                needed_gib: 10
            }
        );
        assert_eq!(
            disk_finding(Some(10 << 20), "/var/lib"),
            Finding::DiskTight {
                free_tenths_gib: 100,
                path: path(),
                roomy_gib: 30
            }
        );
        assert_eq!(
            disk_finding(Some(30 << 20), "/var/lib"),
            Finding::DiskOk {
                free_tenths_gib: 300,
                path: path()
            }
        );
    }

    #[test]
    fn encryption_names_the_paths_or_the_worst_one() {
        use Encryption::{Encrypted, Plain, Unknown};
        let both = [("/", Encrypted), ("/var/lib/rancher", Encrypted)];
        assert_eq!(
            encryption_finding(&both, true),
            Finding::CryptYes {
                paths: vec!["/", "/var/lib/rancher"]
            }
        );
        let plain = [("/", Encrypted), ("/var/lib/rancher", Plain)];
        assert_eq!(
            encryption_finding(&plain, true),
            Finding::CryptRequiredPlain {
                path: "/var/lib/rancher"
            }
        );
        assert_eq!(
            encryption_finding(&plain, false),
            Finding::CryptNo {
                path: "/var/lib/rancher"
            }
        );
        let unknown = [("/etc/rancher/rke2", Unknown)];
        assert_eq!(
            encryption_finding(&unknown, true),
            Finding::CryptRequiredUnknown {
                path: "/etc/rancher/rke2"
            }
        );
        assert_eq!(
            encryption_finding(&unknown, false),
            Finding::CryptUnknown {
                path: "/etc/rancher/rke2"
            }
        );
    }

    #[test]
    fn encryption_covers_where_each_node_keeps_secrets_and_data() {
        assert_eq!(
            secret_paths(Distribution::K3s),
            ["/", "/var/lib/rancher/k3s", "/etc/rancher/k3s"]
        );
        assert_eq!(
            secret_paths(Distribution::Rke2),
            ["/", "/var/lib/rancher/rke2", "/etc/rancher/rke2"]
        );
        assert_eq!(
            secret_paths(Distribution::KubeSolo),
            ["/", "/var/lib/kubesolo", "/etc/kubesolo", "/etc/zeatend"]
        );
        assert_eq!(existing_ancestor("/no/such/dir/here"), "/");
    }

    #[test]
    fn one_plain_file_system_is_enough_to_fail() {
        use Encryption::{Encrypted, Plain, Unknown};
        assert_eq!(
            least_encrypted(&[("/", Encrypted), ("/var/lib/rancher", Encrypted)]),
            (Encrypted, vec!["/", "/var/lib/rancher"])
        );
        assert_eq!(
            least_encrypted(&[
                ("/", Encrypted),
                ("/etc/rancher/rke2", Unknown),
                ("/var/lib/rancher", Plain),
            ]),
            (Plain, vec!["/var/lib/rancher"])
        );
        assert_eq!(
            least_encrypted(&[("/", Encrypted), ("/etc/rancher/rke2", Unknown)]),
            (Unknown, vec!["/etc/rancher/rke2"])
        );
    }
}
