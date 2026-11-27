//! Facts about the host the plan depends on.

use std::fs;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use zeatend_plan::cluster::{Distribution, IpFamily, Rancher};
use zeatend_plan::plan::Facts;
use zeatend_plan::{installer, k3s, kubesolo};

use crate::HostError;

const MAX_USER_WATCHES: &str = "/proc/sys/fs/inotify/max_user_watches";
const RESTRICT_UNCONFINED: &str = "/proc/sys/kernel/apparmor_restrict_unprivileged_unconfined";

/// Reads the architecture, host name, memory and the node's addresses: those of `interface`
/// (`network.interface`), or of the interfaces with default routes.
///
/// # Errors
///
/// When `/proc` cannot be read.
pub async fn read(interface: Option<&str>) -> Result<Facts, HostError> {
    let hostname = fs::read_to_string("/proc/sys/kernel/hostname")
        .map_err(HostError::io("/proc/sys/kernel/hostname"))?;
    let meminfo = fs::read_to_string("/proc/meminfo").map_err(HostError::io("/proc/meminfo"))?;
    let NodeAddresses { internal, external } = node_addresses(interface).await;
    Ok(Facts {
        arch: arch(std::env::consts::ARCH).to_owned(),
        hostname: hostname.trim().to_owned(),
        node_name: installed_node_name(Path::new("/")),
        host_hex: host_hex()?,
        memory_mib: memory_mib(&meminfo).ok_or_else(|| HostError::Parse {
            what: "/proc/meminfo".to_owned(),
            reason: "no MemTotal".to_owned(),
        })?,
        node_ips: internal,
        external_ips: external,
        record: None,
        kubernetes: None,
        apparmor_restricts_unconfined: fs::read_to_string(RESTRICT_UNCONFINED)
            .is_ok_and(|v| v.trim() == "1"),
        inotify_max_user_watches: fs::read_to_string(MAX_USER_WATCHES)
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0),
        service_family: installed_service_family(Path::new("/")),
        k3s_own_network: fs::read_to_string(k3s::CONFIG_FILE)
            .is_ok_and(|config| k3s::runs_own_network(&config)),
        installer_webhook: Path::new(installer::KUBESOLO_WEBHOOK_SCRIPT).exists(),
        longhorn: None,
        registry_files: Vec::new(),
        registry_passwords: std::collections::BTreeMap::new(),
    })
}

/// Six hex digits of the machine id's hash, so `plan` and a later `apply` pick the same name;
/// random without one.
fn host_hex() -> Result<String, HostError> {
    use sha2::{Digest as _, Sha256};
    match fs::read_to_string("/etc/machine-id") {
        Ok(id) if !id.trim().is_empty() => Ok(hex::encode(Sha256::digest(id.trim()))
            .chars()
            .take(6)
            .collect()),
        _ => crate::random::hex(6),
    }
}

/// The name K3s, RKE2 or `KubeSolo` already gives this node: the `node-name` of its
/// configuration, else the subject of the kubelet's certificate. `root` prefixes every path.
#[must_use]
pub fn installed_node_name(root: &Path) -> Option<String> {
    let at = |path: &str| root.join(path.trim_start_matches('/'));
    let configured = Rancher::ALL.into_iter().find_map(|d| {
        rancher_config_files(root, d)
            .iter()
            .filter_map(|f| rancher_node_name(f))
            .next_back()
    });
    configured
        .or_else(|| kubesolo_node_name(&at(kubesolo::CONFIG_FILE)))
        .or_else(|| {
            [
                Distribution::K3s,
                Distribution::Rke2,
                Distribution::KubeSolo,
            ]
            .into_iter()
            .map(|d| d.layout().kubelet_cert)
            .find_map(|cert| cert_node_name(&fs::read_to_string(at(cert)).ok()?))
        })
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
}

/// The family of the primary service network when the K3s or RKE2 configuration under `root` sets
/// `service-cidr`; the last file that sets it wins, as the distributions read them.
#[must_use]
pub fn installed_service_family(root: &Path) -> Option<IpFamily> {
    Rancher::ALL.into_iter().find_map(|d| {
        rancher_config_files(root, d)
            .iter()
            .filter_map(|f| Rancher::service_family(&fs::read_to_string(f).ok()?))
            .next_back()
    })
}

/// `config.yaml` of `distribution` under `root`, then its `config.yaml.d` drop-ins in the order
/// the distribution reads them.
fn rancher_config_files(root: &Path, distribution: Rancher) -> Vec<std::path::PathBuf> {
    let dir = root.join(
        distribution
            .distribution()
            .layout()
            .config_dir
            .trim_start_matches('/'),
    );
    let mut drop_ins: Vec<_> = fs::read_dir(dir.join("config.yaml.d"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
        .collect();
    drop_ins.sort();
    std::iter::once(dir.join("config.yaml"))
        .chain(drop_ins)
        .collect()
}

fn rancher_node_name(path: &Path) -> Option<String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "kebab-case")]
    struct NodeName {
        node_name: Option<String>,
    }
    let config: NodeName = serde_saphyr::from_str(&fs::read_to_string(path).ok()?).ok()?;
    config.node_name
}

fn kubesolo_node_name(path: &Path) -> Option<String> {
    let config: kubesolo::Config = serde_saphyr::from_str(&fs::read_to_string(path).ok()?).ok()?;
    config.kubernetes.node_name
}

/// The node in a kubelet certificate's `system:node:<name>` subject.
fn cert_node_name(pem: &str) -> Option<String> {
    const PREFIX: &[u8] = b"system:node:";
    let body: String = pem
        .lines()
        .skip_while(|l| !l.starts_with("-----BEGIN CERTIFICATE-----"))
        .skip(1)
        .take_while(|l| !l.starts_with("-----END"))
        .collect();
    let der = STANDARD.decode(body).ok()?;
    let at = der.windows(PREFIX.len()).position(|w| w == PREFIX)?;
    // The string's DER length byte comes just before it; node names fit its short form.
    let len = usize::from(*der.get(at.checked_sub(1)?)?);
    let name = der.get(at + PREFIX.len()..at + len)?;
    String::from_utf8(name.to_vec()).ok()
}

/// A global address of the host, as `ip -o addr` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostAddress {
    /// The interface.
    pub(crate) interface: String,
    /// The address, without its prefix length.
    pub(crate) address: String,
    /// An IPv6 privacy address, which the kernel replaces about once a day (RFC 8981).
    temporary: bool,
    /// Past its preferred lifetime, or not yet usable (tentative, failed duplicate detection).
    unusable: bool,
    /// Configured by the system, not learnt from router advertisements or DHCP.
    permanent: bool,
}

impl HostAddress {
    /// An IPv6 address.
    fn is_v6(&self) -> bool {
        self.address.contains(':')
    }
}

/// The host's global addresses with their flags.
pub(crate) async fn host_addresses() -> Vec<HostAddress> {
    parse_addresses(
        &crate::cmd::run("ip", &["-o", "addr", "show", "scope", "global"], None)
            .await
            .unwrap_or_default(),
    )
}

pub(crate) fn parse_addresses(text: &str) -> Vec<HostAddress> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let interface = words.nth(1)?.trim_end_matches(':').to_owned();
            if !matches!(words.next()?, "inet" | "inet6") {
                return None;
            }
            let (address, _prefix) = words.next()?.split_once('/')?;
            let address = address.to_owned();
            let flags: Vec<&str> = words
                .take_while(|w| *w != "\\" && *w != "valid_lft")
                .collect();
            let has = |f: &str| flags.contains(&f);
            Some(HostAddress {
                interface,
                address,
                temporary: has("temporary"),
                unusable: has("deprecated") || has("tentative") || has("dadfailed"),
                permanent: !has("dynamic"),
            })
        })
        .collect()
}

/// The interface of the first default route, `-4` or `-6`.
async fn default_interface(family: &str) -> Option<String> {
    let routes = crate::cmd::run("ip", &[family, "-o", "route", "show", "default"], None)
        .await
        .ok()?;
    route_interface(&routes)
}

fn route_interface(routes: &str) -> Option<String> {
    let line = routes.lines().next()?;
    let mut words = line.split_whitespace();
    words.find(|w| *w == "dev")?;
    words.next().map(str::to_owned)
}

/// The addresses a node registers with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NodeAddresses {
    /// `node-ip`.
    internal: Vec<String>,
    /// `node-external-ip`: the default-route addresses, when the node uses another interface.
    external: Vec<String>,
}

/// The addresses the node registers with, never a temporary IPv6 address.
///
/// Without `interface`, these are the IPv4 and the IPv6 address of the interfaces with default
/// routes, as K3s and RKE2 pick them themselves. They take the first IPv6 address they find,
/// which is often the temporary one; it changes within a day, and the node's address in the
/// network policies goes stale. A configured address comes first, then one from SLAAC (EUI-64 or
/// stable-privacy), which keeps its value.
///
/// With `interface`, the same rule picks its addresses, and the default-route addresses that are
/// not on it become the external ones, so Ingress and `LoadBalancer` addresses stay public.
async fn node_addresses(interface: Option<&str>) -> NodeAddresses {
    choose_addresses(
        &host_addresses().await,
        interface,
        default_interface("-4").await.as_deref(),
        default_interface("-6").await.as_deref(),
    )
}

/// [`node_addresses`] from what it reads.
fn choose_addresses(
    addresses: &[HostAddress],
    interface: Option<&str>,
    v4_interface: Option<&str>,
    v6_interface: Option<&str>,
) -> NodeAddresses {
    let default = choose_node_ips(addresses, v4_interface, v6_interface);
    let Some(interface) = interface else {
        return NodeAddresses {
            internal: default,
            external: Vec::new(),
        };
    };
    let internal = choose_node_ips(addresses, Some(interface), Some(interface));
    let external = default
        .into_iter()
        .filter(|a| !internal.contains(a))
        .collect();
    NodeAddresses { internal, external }
}

/// The IPv4 and the stable IPv6 address of the interfaces of the two default routes.
pub(crate) fn choose_node_ips(
    addresses: &[HostAddress],
    v4_interface: Option<&str>,
    v6_interface: Option<&str>,
) -> Vec<String> {
    let usable = |a: &&HostAddress, v6: bool, interface: &str| {
        a.is_v6() == v6 && a.interface == interface && !a.unusable && !a.temporary
    };
    let v4 = v4_interface.and_then(|i| addresses.iter().find(|a| usable(a, false, i)));
    let v6 = v6_interface.and_then(|i| {
        let stable: Vec<&HostAddress> = addresses.iter().filter(|a| usable(a, true, i)).collect();
        stable
            .iter()
            .find(|a| a.permanent)
            .or_else(|| stable.first())
            .copied()
    });
    v4.into_iter()
        .chain(v6)
        .map(|a| a.address.clone())
        .collect()
}

/// Rust's architecture name as RKE2 spells it.
#[must_use]
pub fn arch(rust: &str) -> &str {
    match rust {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    }
}

fn memory_mib(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib / 1024)
}

/// The host's global addresses, IPv4 first, from `ip`; empty when it cannot run.
#[must_use]
pub async fn addresses() -> Vec<String> {
    let mut found = host_addresses().await;
    found.sort_by_key(HostAddress::is_v6);
    found.into_iter().map(|a| a.address).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const IP_ADDR: &str = "\
2: enp0s5    inet 192.0.2.121/24 brd 192.0.2.255 scope global dynamic noprefixroute enp0s5\\       valid_lft 1107sec preferred_lft 1107sec
11: flannel.1    inet 10.42.5.0/32 scope global flannel.1\\       valid_lft forever preferred_lft forever
2: enp0s5    inet6 fdb2:2c26:f4e4:0:9b2d:5a24:581b:f432/64 scope global temporary dynamic \\       valid_lft 172109sec preferred_lft 85390sec
2: enp0s5    inet6 fdb2:2c26:f4e4:0:3ad8:c781:797:77a5/64 scope global dynamic mngtmpaddr noprefixroute \\       valid_lft 2591869sec preferred_lft 604669sec
";

    fn write(root: &Path, path: &str, content: &str) {
        let path = root.join(path.trim_start_matches('/'));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn cert(subject: &str) -> String {
        let mut der = vec![0x30, 0x00, 0x0C, u8::try_from(subject.len()).unwrap()];
        der.extend(subject.as_bytes());
        der.extend([0x30, 0x00]);
        format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            STANDARD.encode(der)
        )
    }

    #[test]
    fn a_node_keeps_the_name_its_configuration_gives_it() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        assert_eq!(installed_node_name(root), None);
        write(
            root,
            "/etc/rancher/k3s/config.yaml",
            "protect-kernel-defaults: true\n",
        );
        write(
            root,
            "/var/lib/rancher/k3s/agent/client-kubelet.crt",
            &cert("system:node:Old-Host"),
        );
        assert_eq!(installed_node_name(root).as_deref(), Some("old-host"));
        write(
            root,
            "/etc/rancher/k3s/config.yaml.d/15-node-name.yaml",
            "node-name: vps-1\n",
        );
        assert_eq!(installed_node_name(root).as_deref(), Some("vps-1"));
        write(
            root,
            "/etc/rancher/k3s/config.yaml.d/90-local.yaml",
            "node-name: db-1\n",
        );
        assert_eq!(installed_node_name(root).as_deref(), Some("db-1"));
    }

    #[test]
    fn the_last_service_cidr_on_the_host_gives_the_family() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        assert_eq!(installed_service_family(root), None);
        write(
            root,
            "/etc/rancher/rke2/config.yaml.d/90-local.yaml",
            "service-cidr: fd00:43::/112\n",
        );
        assert_eq!(installed_service_family(root), Some(IpFamily::V6));
        write(
            root,
            "/etc/rancher/rke2/config.yaml.d/95-later.yaml",
            "service-cidr: 10.43.0.0/16,fd00:43::/112\n",
        );
        assert_eq!(installed_service_family(root), Some(IpFamily::V4));
    }

    #[test]
    fn kubesolo_keeps_its_name_too() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        write(
            root,
            Distribution::KubeSolo.layout().kubelet_cert,
            &cert("system:node:solo"),
        );
        assert_eq!(installed_node_name(root).as_deref(), Some("solo"));
        write(
            root,
            kubesolo::CONFIG_FILE,
            "kind: Config\nkubernetes:\n  nodeName: \"solo-2\"\n",
        );
        assert_eq!(installed_node_name(root).as_deref(), Some("solo-2"));
    }

    #[test]
    fn node_ips_leave_temporary_addresses_out() {
        let addresses = parse_addresses(IP_ADDR);
        assert_eq!(addresses.len(), 4);
        assert_eq!(addresses[1].interface, "flannel.1");
        assert!(addresses[2].temporary && !addresses[3].temporary);
        assert_eq!(
            choose_node_ips(&addresses, Some("enp0s5"), Some("enp0s5")),
            ["192.0.2.121", "fdb2:2c26:f4e4:0:3ad8:c781:797:77a5"]
        );
        assert_eq!(
            choose_node_ips(&addresses, Some("enp0s5"), None),
            ["192.0.2.121"]
        );
        let only_temporary: Vec<HostAddress> = addresses
            .iter()
            .filter(|a| !a.address.ends_with("77a5"))
            .cloned()
            .collect();
        assert_eq!(
            choose_node_ips(&only_temporary, Some("enp0s5"), Some("enp0s5")),
            ["192.0.2.121"]
        );
    }

    #[test]
    fn the_configured_interface_gives_the_node_addresses_and_the_default_route_the_external_ones() {
        let addresses = parse_addresses(
            "2: eth0    inet 203.0.113.7/24 brd 203.0.113.255 scope global eth0\\       valid_lft forever preferred_lft forever\n\
             3: eth1    inet 10.0.0.7/24 brd 10.0.0.255 scope global eth1\\       valid_lft forever preferred_lft forever\n\
             3: eth1    inet6 fd00::aa/64 scope global temporary dynamic \\       valid_lft 1sec preferred_lft 1sec\n\
             3: eth1    inet6 fd00::7/64 scope global \\       valid_lft forever preferred_lft forever\n\
             2: eth0    inet6 2001:db8::7/64 scope global \\       valid_lft forever preferred_lft forever\n",
        );
        assert_eq!(
            choose_addresses(&addresses, Some("eth1"), Some("eth0"), Some("eth0")),
            NodeAddresses {
                internal: vec!["10.0.0.7".to_owned(), "fd00::7".to_owned()],
                external: vec!["203.0.113.7".to_owned(), "2001:db8::7".to_owned()],
            }
        );
        // The default route on the interface itself: nothing is external.
        assert_eq!(
            choose_addresses(&addresses, Some("eth1"), Some("eth1"), None),
            NodeAddresses {
                internal: vec!["10.0.0.7".to_owned(), "fd00::7".to_owned()],
                external: Vec::new(),
            }
        );
        assert_eq!(
            choose_addresses(&addresses, None, Some("eth0"), Some("eth0")),
            NodeAddresses {
                internal: vec!["203.0.113.7".to_owned(), "2001:db8::7".to_owned()],
                external: Vec::new(),
            }
        );
        assert_eq!(
            choose_addresses(&addresses, Some("eth9"), Some("eth0"), None).internal,
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_configured_ipv6_address_comes_first() {
        let addresses = parse_addresses(
            "2: eth0    inet6 2001:db8::aa/64 scope global dynamic mngtmpaddr \\       valid_lft 1sec preferred_lft 1sec\n\
             2: eth0    inet6 2001:db8::5/64 scope global \\       valid_lft forever preferred_lft forever\n\
             2: eth0    inet6 2001:db8::6/64 scope global deprecated \\       valid_lft forever preferred_lft 0sec\n",
        );
        assert_eq!(
            choose_node_ips(&addresses, None, Some("eth0")),
            ["2001:db8::5"]
        );
        assert_eq!(
            route_interface("default via fe80::1 dev eth0 proto ra metric 100 pref medium\n"),
            Some("eth0".to_owned())
        );
        assert_eq!(route_interface(""), None);
    }

    #[test]
    fn meminfo() {
        assert_eq!(
            memory_mib("MemTotal:        3995216 kB\nMemFree: 1 kB\n"),
            Some(3901)
        );
        assert_eq!(memory_mib("MemFree: 1 kB\n"), None);
        assert_eq!(arch("aarch64"), "arm64");
    }
}
