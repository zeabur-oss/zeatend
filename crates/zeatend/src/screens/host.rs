//! What the screens show about this host, read once before they open.

use std::fs;
use std::net::IpAddr;
use std::num::NonZero;

use zeatend_host::preflight::{self, Node};
use zeatend_host::{HostError, facts, systemd};
use zeatend_plan::cluster::{Distribution, Rancher};
use zeatend_plan::finding::Check;
use zeatend_plan::plan::Facts;

/// The distributions, in the order the Server screen lists them.
pub(crate) const DISTRIBUTIONS: [Distribution; 3] = [
    Distribution::KubeSolo,
    Distribution::K3s,
    Distribution::Rke2,
];

/// This host.
#[derive(Debug, Clone)]
pub(crate) struct Host {
    /// The host name.
    pub(crate) hostname: String,
    /// The operating system, from `PRETTY_NAME` in `/etc/os-release`.
    pub(crate) system: String,
    /// The CPUs this process can use.
    pub(crate) cpus: usize,
    /// Memory in MiB.
    pub(crate) memory_mib: u64,
    /// The global addresses, IPv4 first.
    pub(crate) addresses: Vec<String>,
    /// The host checks for each distribution.
    pub(crate) checks: Vec<(Distribution, Vec<Check>)>,
    /// What the plan needs.
    pub(crate) facts: Facts,
}

impl Host {
    /// The host checks for `distribution`.
    pub(crate) fn checks(&self, distribution: Distribution) -> &[Check] {
        self.checks
            .iter()
            .find(|(d, _)| *d == distribution)
            .map_or(&[], |(_, checks)| checks.as_slice())
    }
}

/// Reads this host and runs the host checks of each distribution.
///
/// # Errors
///
/// When `/proc` cannot be read.
pub(crate) async fn read(require_encrypted_disk: bool) -> Result<Host, HostError> {
    let facts = facts::read().await?;
    let addresses = facts::addresses().await;
    let os_release = fs::read_to_string("/etc/os-release").unwrap_or_default();
    let (kubesolo, k3s, rke2) = tokio::join!(
        checks(Node::KubeSolo, require_encrypted_disk),
        checks(Node::Server(Rancher::K3s), require_encrypted_disk),
        checks(Node::Server(Rancher::Rke2), require_encrypted_disk),
    );
    Ok(Host {
        hostname: facts.hostname.clone(),
        system: pretty_name(&os_release).unwrap_or_else(|| "Linux".to_owned()),
        cpus: std::thread::available_parallelism().map_or(1, NonZero::get),
        memory_mib: facts.memory_mib,
        addresses,
        checks: vec![
            (Distribution::KubeSolo, kubesolo),
            (Distribution::K3s, k3s),
            (Distribution::Rke2, rke2),
        ],
        facts,
    })
}

async fn checks(node: Node, require_encrypted_disk: bool) -> Vec<Check> {
    let unit = match node {
        Node::Server(rancher) => rancher.server_unit(),
        Node::Agent(rancher) => rancher.agent_unit(),
        Node::KubeSolo => zeatend_plan::kubesolo::UNIT,
    };
    preflight::run(preflight::Options {
        cluster_running: systemd::is_active(unit).await,
        require_encrypted_disk,
        node,
    })
    .await
}

/// `PRETTY_NAME` of an os-release file (os-release(5)).
fn pretty_name(os_release: &str) -> Option<String> {
    let value = os_release
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))?;
    let value = value.trim().trim_matches(['"', '\'']);
    (!value.is_empty()).then(|| value.to_owned())
}

/// Whether only a private network reaches `address`: RFC 1918, shared (RFC 6598), loopback or
/// link-local IPv4, or unique local (RFC 4193), loopback or link-local IPv6.
pub(crate) fn is_private(address: &str) -> bool {
    match address.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            let [a, b, ..] = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || (a == 100 && (64..128).contains(&b))
        }
        Ok(IpAddr::V6(v6)) => {
            v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local()
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_is_the_pretty_name() {
        let os_release = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 26.04 LTS\"\nID=ubuntu\n";
        assert_eq!(pretty_name(os_release).as_deref(), Some("Ubuntu 26.04 LTS"));
        assert_eq!(pretty_name("ID=alpine\n"), None);
    }

    #[test]
    fn private_addresses_are_those_only_a_private_network_reaches() {
        for private in [
            "10.0.0.5",
            "192.168.1.2",
            "172.16.0.1",
            "100.64.0.1",
            "fd00::1",
        ] {
            assert!(is_private(private), "{private}");
        }
        for public in [
            "203.0.113.7",
            "100.128.0.1",
            "2001:db8::1",
            "k8s.example.com",
        ] {
            assert!(!is_private(public), "{public}");
        }
    }
}
