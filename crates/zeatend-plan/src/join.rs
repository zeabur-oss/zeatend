//! The plan of a node joining an existing cluster: K3s or RKE2 only, as an agent (worker) or, on
//! RKE2, a server, with what the joining VM measured about its distance to the server.
//!
//! Every node of a cluster belongs at one provider in one region. etcd waits for a majority on
//! every write, the kubelet and the API server talk constantly, and volumes are local, so a node
//! farther away slows the whole cluster or drops out of it. `plan` measures the round trip and
//! always says the rule.

use serde::Serialize;

use crate::cluster::{self, HostFile, PortUse, Rancher};
use crate::config::{JoinConfig, Role};
use crate::finding::{Check, Finding};
use crate::plan::{ClusterPlan, Facts, cluster_plan};
use crate::precondition::Preconditions;
use crate::tuning::{HostTuning, MemoryTier};
use crate::upgrade::{self, Kubernetes};
use crate::{PlanError, node_name, ports};

/// Connections timed to the server's join port.
pub const RTT_SAMPLES: usize = 5;
/// Above this median round trip, in microseconds, the server is farther than one region allows:
/// between availability zones of one region round trips are about 0.5 to 2 ms.
pub const RTT_WARN_US: u64 = 10_000;
/// Above this median round trip, in microseconds, the cluster is at risk: etcd's heartbeat is
/// 100 ms by default and every write waits for a majority round trip.
pub const RTT_CRITICAL_US: u64 = 50_000;

/// TCP ports on the server this node must reach. The first one is the join port; a node cannot
/// join without it.
#[must_use]
pub fn server_ports(distribution: Rancher, role: Role) -> Vec<(u16, PortUse)> {
    let mut ports = match distribution {
        Rancher::K3s => vec![(ports::API_SERVER, PortUse::K3sApiAndSupervisor)],
        Rancher::Rke2 => vec![
            (ports::SUPERVISOR, PortUse::Rke2Supervisor),
            (ports::API_SERVER, PortUse::KubernetesApi),
        ],
    };
    ports.push((ports::KUBELET, PortUse::Kubelet));
    if role == Role::Server {
        ports.extend([
            (ports::ETCD_CLIENT, PortUse::EtcdClient),
            (ports::ETCD_PEER, PortUse::EtcdPeer),
        ]);
    }
    ports
}

/// What the joining VM found out, read by the caller so planning stays free of I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinFacts {
    /// The host.
    pub facts: Facts,
    /// TCP connect times to the server's join port, in microseconds; empty when it did not answer.
    pub rtt_us: Vec<u64>,
    /// Each port of `server_ports` and whether a TCP connection to it succeeded.
    pub ports: Vec<(u16, bool)>,
    /// The token carries the cluster CA's hash, so the node checks whom it joins.
    pub token_pinned: bool,
    /// The node's unit (`k3s-agent`, `rke2-server`, …) is running: the node joined already and
    /// holds its own ports.
    pub unit_running: bool,
}

/// One port and what was found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Port {
    /// The port number.
    pub number: u16,
    /// `tcp` or `udp`.
    pub protocol: &'static str,
    /// What uses it.
    pub purpose: PortUse,
    /// Whether it answered; `None` when it cannot be checked from here (UDP).
    pub open: Option<bool>,
}

/// Round trips to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rtt {
    /// Median, in microseconds.
    pub median_us: u64,
    /// Fastest.
    pub min_us: u64,
    /// Slowest.
    pub max_us: u64,
    /// How many connections were timed.
    pub samples: usize,
}

/// How far the server is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Latency {
    /// Within what one region gives.
    Fine,
    /// More than one region gives.
    High,
    /// The cluster will be slow and unstable.
    Critical,
}

/// The latency class of a median round trip.
#[must_use]
const fn latency(median_us: u64) -> Latency {
    if median_us > RTT_CRITICAL_US {
        Latency::Critical
    } else if median_us > RTT_WARN_US {
        Latency::High
    } else {
        Latency::Fine
    }
}

/// The median of the samples (the lower middle one for an even count).
#[must_use]
pub fn rtt(samples: &[u64]) -> Option<Rtt> {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let median = *sorted.get((sorted.len().checked_sub(1)?) / 2)?;
    Some(Rtt {
        median_us: median,
        min_us: *sorted.first()?,
        max_us: *sorted.last()?,
        samples: sorted.len(),
    })
}

/// Milliseconds with one decimal.
#[must_use]
pub fn ms(us: u64) -> String {
    let tenths = (us + 50) / 100;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// The plan of a joining node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinPlan {
    /// Facts it was computed from.
    pub facts: Facts,
    /// The node's name in Kubernetes, written into the distribution's configuration.
    pub node_name: String,
    /// `worker` or `server`.
    pub role: Role,
    /// The URL it joins through.
    pub server: String,
    /// The systemd unit, such as `k3s-agent` or `rke2-server`.
    pub unit: &'static str,
    /// The release; it must be the cluster's.
    pub cluster: ClusterPlan,
    /// Files written before the node starts. The token file is not among them.
    #[serde(skip)]
    pub host_files: Vec<HostFile>,
    /// Their paths, for the JSON form.
    pub host_file_paths: Vec<String>,
    /// What `apply` does for Longhorn; `None` unless `storage.longhorn` is set.
    pub longhorn: Option<crate::longhorn::Prep>,
    /// Files of registry and proxy settings that zeatend wrote before and removes now.
    pub removed_files: Vec<String>,
    /// Round trips to the server.
    pub rtt: Option<Rtt>,
    /// Ports on the server, and UDP ports that cannot be checked.
    pub ports: Vec<Port>,
    /// Ports on this VM that `apply` checks are free before the node starts.
    pub local_ports: Vec<Port>,
    /// The node's unit is running, so its ports are in use by it and `apply` doesn't check them.
    pub unit_running: bool,
    /// What to read before applying; a failed one stops `apply`.
    pub findings: Vec<Check>,
    /// What the plan found on the host and what it writes; `apply` fills it. The token file is
    /// not among the files.
    pub preconditions: Preconditions,
}

const fn rtt_finding(rtt: Rtt, port: u16) -> Option<Finding> {
    let (median_us, samples) = (rtt.median_us, rtt.samples);
    match latency(median_us) {
        Latency::Fine => None,
        Latency::High => Some(Finding::ServerFar {
            median_us,
            samples,
            port,
        }),
        Latency::Critical => Some(Finding::ServerTooFar {
            median_us,
            samples,
            port,
        }),
    }
}

/// The ports this node listens on, without a probe.
fn local_ports(distribution: Rancher, role: Role) -> Vec<Port> {
    distribution
        .node_ports(role == Role::Server)
        .into_iter()
        .map(|(number, purpose)| Port {
            number,
            protocol: "tcp",
            purpose,
            open: None,
        })
        .collect()
}

/// The server's ports with what was found, a finding for closed ones besides the join port (a
/// finding of its own), and the UDP port that cannot be checked.
fn ports(
    distribution: Rancher,
    role: Role,
    found: &[(u16, bool)],
    host: &str,
    findings: &mut Vec<Finding>,
) -> Vec<Port> {
    let mut ports: Vec<Port> = server_ports(distribution, role)
        .into_iter()
        .map(|(port, purpose)| Port {
            number: port,
            protocol: "tcp",
            purpose,
            open: found
                .iter()
                .find(|(p, _)| *p == port)
                .map(|(_, open)| *open),
        })
        .collect();
    let closed: Vec<u16> = ports
        .iter()
        .filter(|p| p.number != distribution.join_port() && p.open == Some(false))
        .map(|p| p.number)
        .collect();
    if !closed.is_empty() {
        findings.push(Finding::ServerPortsClosed {
            ports: closed,
            host: host.to_owned(),
        });
    }
    let (number, purpose) = distribution.vxlan();
    ports.push(Port {
        number,
        protocol: "udp",
        purpose,
        open: None,
    });
    ports
}

/// What to tell before a node joins, on the server: the same messages `plan` shows on the new VM,
/// less what only the VM can find out.
#[must_use]
pub fn advice(distribution: Rancher, role: Role) -> Vec<Finding> {
    let mut all = vec![Finding::SameRegion];
    all.extend(
        standing_findings(distribution, role, true)
            .into_iter()
            .filter(|f| !matches!(f, Finding::UdpUnchecked { .. })),
    );
    all
}

/// What every join is told: the ports, UDP, high availability for servers, where volumes live on
/// K3s, and an unpinned token.
fn standing_findings(distribution: Rancher, role: Role, token_pinned: bool) -> Vec<Finding> {
    let mut findings = vec![
        Finding::JoinPorts { distribution },
        Finding::UdpUnchecked {
            port: distribution.vxlan().0,
        },
    ];
    if role == Role::Server {
        findings.push(Finding::ServersHa);
    }
    // RKE2 has no storage class until the Zeabur Catalog installs one.
    if distribution == Rancher::K3s {
        findings.push(Finding::LocalStorage);
    }
    if !token_pinned {
        findings.push(Finding::TokenUnpinned);
    }
    findings
}

/// Computes the plan of a joining node.
///
/// # Errors
///
/// When a newer zeatend applied, the architecture has no release, K3s is asked for a second
/// server, the node name cannot be used, or the plan would downgrade the release, skip a
/// Kubernetes minor version or run a worker newer than the API server (see [`crate::upgrade`]).
pub fn plan(config: &JoinConfig, facts: &JoinFacts) -> Result<JoinPlan, PlanError> {
    facts.facts.check_zeatend_version()?;
    let join = &config.join;
    let node_name = node_name::choose(
        config.host.node_name.as_deref(),
        &facts.facts,
        &join.node_names,
    )?;
    let distribution = config.rancher()?;
    let mut findings: Vec<Finding> =
        node_name::localhost_note(&node_name, &facts.facts, distribution.into())
            .into_iter()
            .collect();
    let mut cluster = cluster_plan(
        distribution,
        config.version.as_deref(),
        &facts.facts.arch,
        &mut findings,
    )?;
    cluster.proxy.clone_from(&config.proxy);

    upgrade::check_release(
        facts.facts.record.as_ref(),
        distribution.into(),
        &cluster.version,
    )?;
    if let Some(planned) = Kubernetes::parse(&cluster.version) {
        upgrade::check_kubernetes(
            facts.facts.kubernetes,
            planned,
            join.role,
            distribution.into(),
        )?;
    }

    let tuning = HostTuning::new(distribution.into(), MemoryTier::of(facts.facts.memory_mib));
    let (mut host_files, unit) = match join.role {
        Role::Worker => (
            distribution.agent_files(&tuning)?,
            distribution.distribution().layout().unit(Role::Worker),
        ),
        Role::Server if !distribution.takes_servers() => return Err(PlanError::SecondServer),
        Role::Server => (
            distribution.server_files(&cluster::Options {
                tls_san: config.host.tls_san.clone(),
                tuning,
                // Only RKE2 takes a second server, and it always runs Canal.
                own_network: false,
                disable: config.disable.clone(),
                api_server: config.api_server.clone(),
            })?,
            distribution.distribution().layout().server_unit,
        ),
    };
    host_files.push(distribution.join_file(&join.server)?);
    host_files.push(distribution.node_name_file(&node_name)?);
    host_files.extend(distribution.node_ip_file(&facts.facts.node_ips)?);
    let (host, port) = crate::config::server_address(&join.server, distribution.join_port())?;
    let mut direct = facts.facts.node_ips.clone();
    direct.push(host.clone());
    host_files.extend(config.registry(&facts.facts.registry_passwords).files(
        distribution.into(),
        join.role,
        &direct,
    )?);
    host_files.extend(facts.facts.apparmor_profile());
    host_files.push(facts.facts.inotify_sysctl());

    findings.push(Finding::SameRegion);

    let rtt = rtt(&facts.rtt_us);
    match rtt {
        None => findings.push(Finding::ServerUnreachable {
            server: join.server.clone(),
            port,
        }),
        Some(rtt) => findings.extend(rtt_finding(rtt, port)),
    }

    let ports = ports(distribution, join.role, &facts.ports, &host, &mut findings);
    findings.extend(standing_findings(
        distribution,
        join.role,
        facts.token_pinned,
    ));

    Ok(JoinPlan {
        facts: facts.facts.clone(),
        node_name,
        role: join.role,
        server: join.server.clone(),
        unit,
        cluster,
        host_file_paths: host_files.iter().map(|f| f.path.clone()).collect(),
        removed_files: crate::registry::removed(&facts.facts.registry_files, &host_files),
        host_files,
        longhorn: facts.facts.longhorn(config.storage)?,
        rtt,
        ports,
        local_ports: local_ports(distribution, join.role),
        unit_running: facts.unit_running,
        findings: findings.into_iter().map(Check::from).collect(),
        preconditions: Preconditions::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Document, JoinConfig};
    use crate::finding::Status;

    fn config(extra: &str) -> JoinConfig {
        let yaml = format!(
            "distribution: rke2\njoin:\n  server: https://10.0.0.5:9345\n  tokenFile: /etc/zeatend/join-token\n{extra}"
        );
        match Document::parse(&yaml).unwrap() {
            Document::Join(c) => c,
            Document::Install(_) => panic!("not a join"),
        }
    }

    fn facts(rtt_us: &[u64]) -> JoinFacts {
        JoinFacts {
            facts: Facts {
                arch: "amd64".to_owned(),
                hostname: "vps-2".to_owned(),
                node_name: None,
                host_hex: "a1b2c3".to_owned(),
                memory_mib: 4096,
                node_ips: vec!["10.0.0.6".to_owned(), "fd00::6".to_owned()],
                record: None,
                kubernetes: None,
                apparmor_restricts_unconfined: false,
                inotify_max_user_watches: 0,
                k3s_own_network: false,
                installer_webhook: false,
                longhorn: None,
                registry_files: Vec::new(),
                registry_passwords: std::collections::BTreeMap::new(),
            },
            rtt_us: rtt_us.to_vec(),
            ports: vec![(9345, true), (6443, true), (10250, false)],
            token_pinned: true,
            unit_running: false,
        }
    }

    fn findings(plan: &JoinPlan) -> Vec<&Finding> {
        plan.findings.iter().map(|c| &c.finding).collect()
    }

    #[test]
    fn rtt_thresholds() {
        assert_eq!(latency(800), Latency::Fine);
        assert_eq!(latency(RTT_WARN_US), Latency::Fine);
        assert_eq!(latency(RTT_WARN_US + 1), Latency::High);
        assert_eq!(latency(RTT_CRITICAL_US), Latency::High);
        assert_eq!(latency(RTT_CRITICAL_US + 1), Latency::Critical);
        // One slow handshake does not move the median.
        let r = rtt(&[900, 700, 250_000, 800, 600]).unwrap();
        assert_eq!((r.median_us, r.min_us, r.max_us), (800, 600, 250_000));
        assert_eq!(rtt(&[]), None);
        assert_eq!(rtt(&[3, 1, 2, 4]).unwrap().median_us, 2);
        assert_eq!(ms(649), "0.6");
        assert_eq!(ms(12_340), "12.3");
    }

    #[test]
    fn a_worker_near_the_server() {
        let p = plan(&config(""), &facts(&[600, 700, 650, 900, 640])).unwrap();
        assert_eq!(p.unit, "rke2-agent");
        assert_eq!(
            p.host_file_paths,
            [
                "/etc/sysctl.d/60-rke2-cis.conf",
                "/etc/rancher/rke2/config.yaml",
                "/var/lib/rancher/rke2/agent/etc/kubelet.conf.d/50-zeatend.conf",
                "/etc/rancher/rke2/config.yaml.d/10-join.yaml",
                "/etc/rancher/rke2/config.yaml.d/15-node-name.yaml",
                "/etc/rancher/rke2/config.yaml.d/20-node-ip.yaml",
                "/etc/sysctl.d/60-zeatend-inotify.conf",
            ]
        );
        assert_eq!(
            p.local_ports.iter().map(|p| p.number).collect::<Vec<_>>(),
            [80, 443, 10250]
        );
        assert_eq!(
            findings(&p),
            [
                &Finding::SameRegion,
                &Finding::ServerPortsClosed {
                    ports: vec![10250],
                    host: "10.0.0.5".to_owned()
                },
                &Finding::JoinPorts {
                    distribution: Rancher::Rke2
                },
                &Finding::UdpUnchecked { port: 8472 },
            ]
        );
        assert!(p.findings.iter().all(|c| c.status == Status::Warn));
        let udp = p.ports.iter().find(|p| p.protocol == "udp").unwrap();
        assert_eq!(udp.open, None);
    }

    #[test]
    fn a_joining_node_is_prepared_for_longhorn_as_the_first_one() {
        let mut read = facts(&[600]);
        read.facts.longhorn = Some(crate::longhorn::HostState {
            package_manager: Some(crate::longhorn::PackageManager::Apt),
            iscsid_running: false,
            ..crate::longhorn::HostState::default()
        });
        let p = plan(&config("storage: {longhorn: true}\n"), &read).unwrap();
        let steps = p.longhorn.unwrap().steps;
        assert!(
            steps.contains(&crate::longhorn::Step::StartIscsid),
            "{steps:?}"
        );
        assert_eq!(plan(&config(""), &read).unwrap().longhorn, None);
    }

    #[test]
    fn a_joining_node_is_named_once_and_apart_from_the_other_nodes() {
        let name_in = |p: &JoinPlan| {
            let file = p
                .host_files
                .iter()
                .find(|f| f.path.ends_with("15-node-name.yaml"))
                .unwrap();
            let value: serde_json::Value = serde_saphyr::from_str(&file.content).unwrap();
            value["node-name"].as_str().unwrap().to_owned()
        };
        let p = plan(&config(""), &facts(&[500; 5])).unwrap();
        assert_eq!(
            (p.node_name.as_str(), name_in(&p).as_str()),
            ("vps-2", "vps-2")
        );
        let taken = plan(&config("  nodeNames: [vps-1, vps-2]\n"), &facts(&[500; 5])).unwrap();
        assert_eq!(name_in(&taken), "node-a1b2c3");
        assert!(
            plan(
                &config("  nodeNames: [vps-1]\nhost: {nodeName: vps-1}\n"),
                &facts(&[500; 5])
            )
            .is_err()
        );
    }

    #[test]
    fn a_joining_node_named_localhost_gets_a_note() {
        let mut f = facts(&[]);
        f.facts.hostname = "localhost".to_owned();
        let p = plan(&config(""), &f).unwrap();
        assert!(findings(&p).contains(&&Finding::NodeNameLocalhost {
            distribution: cluster::Distribution::Rke2
        }));
    }

    #[test]
    fn far_servers_are_warned_about() {
        let c = config("");
        let p = plan(&c, &facts(&[70_000; 5])).unwrap();
        assert!(findings(&p).contains(&&Finding::ServerTooFar {
            median_us: 70_000,
            samples: 5,
            port: 9345
        }));

        let mut f = facts(&[20_000; 5]);
        f.token_pinned = false;
        let p = plan(&c, &f).unwrap();
        assert!(findings(&p).contains(&&Finding::ServerFar {
            median_us: 20_000,
            samples: 5,
            port: 9345
        }));
        assert!(findings(&p).contains(&&Finding::TokenUnpinned));

        let p = plan(&c, &facts(&[])).unwrap();
        let failed: Vec<&Finding> = p
            .findings
            .iter()
            .filter(|c| c.status == Status::Fail)
            .map(|c| &c.finding)
            .collect();
        assert_eq!(
            failed,
            [&Finding::ServerUnreachable {
                server: "https://10.0.0.5:9345".to_owned(),
                port: 9345
            }]
        );
    }

    #[test]
    fn the_advice_before_a_join_matches_the_plan() {
        let ports = Finding::JoinPorts {
            distribution: Rancher::Rke2,
        };
        assert_eq!(
            advice(Rancher::Rke2, Role::Worker),
            [Finding::SameRegion, ports.clone()]
        );
        assert_eq!(
            advice(Rancher::Rke2, Role::Server),
            [Finding::SameRegion, ports, Finding::ServersHa]
        );
    }

    #[test]
    fn a_server_gets_the_server_files_and_the_ha_warning() {
        let c = config("  role: server\nhost: {tlsSan: [k8s.example.com]}\n");
        let p = plan(&c, &facts(&[500; 5])).unwrap();
        assert_eq!(p.unit, "rke2-server");
        assert!(findings(&p).contains(&&Finding::ServersHa));
        let config = p
            .host_files
            .iter()
            .find(|f| f.path.ends_with("/config.yaml"))
            .unwrap();
        let config: serde_json::Value = serde_saphyr::from_str(&config.content).unwrap();
        assert_eq!(config["tls-san"], serde_json::json!(["k8s.example.com"]));
        assert!(p.ports.iter().any(|port| port.number == 2380));
    }

    #[test]
    fn servers_and_workers_on_a_host_that_restricts_unconfined_get_containerds_apparmor_profile() {
        let mut restricted = facts(&[500; 5]);
        restricted.facts.apparmor_restricts_unconfined = true;
        for extra in ["", "  role: server\n"] {
            let plain = plan(&config(extra), &facts(&[500; 5])).unwrap();
            assert!(!plain.host_files.contains(&cluster::apparmor_profile()));
            let p = plan(&config(extra), &restricted).unwrap();
            assert!(
                p.host_files.contains(&cluster::apparmor_profile()),
                "{extra}"
            );
        }
    }
}
