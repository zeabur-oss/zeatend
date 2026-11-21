//! The plan, written for people.

use zeatend_plan::cluster::Distribution;
use zeatend_plan::config::Role;
use zeatend_plan::finding::{Check, Status};
use zeatend_plan::join::{self, JoinPlan};
use zeatend_plan::plan::{Plan, Release};
use zeatend_plan::record::Changes;

use crate::i18n;

/// The plan of the first node as text.
pub(crate) fn plan(plan: &Plan) -> String {
    let mut s = String::new();
    let facts = &plan.facts;
    let host = format!(
        "on {} ({}, {} MiB)",
        facts.hostname, facts.arch, facts.memory_mib
    );
    let unit = match &plan.release {
        Release::Cluster(cluster) => {
            push(
                &mut s,
                &format!(
                    "{} {} ({}), CIS hardened, {host}",
                    cluster.distribution.distribution().title(),
                    cluster.version,
                    support(cluster.pinned),
                ),
            );
            cluster.distribution.distribution().layout().server_unit
        }
        Release::KubeSolo(solo) => {
            push(
                &mut s,
                &format!(
                    "KubeSolo {} (Kubernetes {}, {}), {host}",
                    solo.version,
                    solo.kubernetes,
                    support(true)
                ),
            );
            Distribution::KubeSolo.layout().server_unit
        }
    };
    node_name(&mut s, &plan.node_name);
    node_addresses(&mut s, &facts.node_ips);
    push(&mut s, "");
    s.push_str(&changes(plan.changes.as_ref(), unit));
    files(&mut s, &plan.host_file_paths, None);
    removed(&mut s, &plan.removed_files);
    findings(&mut s, &plan.findings);
    s
}

/// What `apply` changes for `unit` against the install record on the host; empty on a first
/// install.
pub(crate) fn changes(changes: Option<&Changes>, unit: &str) -> String {
    let mut s = String::new();
    let Some(c) = changes else {
        return s;
    };
    push(
        &mut s,
        &format!("Changes since the last apply (by {})", c.written_by),
    );
    let before = s.len();
    if let Some(v) = &c.version {
        push(&mut s, &format!("  {unit:<48} {} -> {}", v.from, v.to));
    }
    for path in &c.host_files {
        push(&mut s, &format!("  {path:<48} changes"));
    }
    if c.restart {
        push(
            &mut s,
            &format!(
                "  restarts {unit}. The Kubernetes API stops answering for about 10 seconds. \
                 Running apps keep serving."
            ),
        );
    }
    if s.len() == before {
        push(&mut s, "  nothing");
    }
    push(&mut s, "");
    s
}

/// The plan of a joining node as text.
pub(crate) fn join_plan(plan: &JoinPlan) -> String {
    let mut s = String::new();
    let cluster = &plan.cluster;
    let title = cluster.distribution.distribution().title();
    let (what, runs) = match plan.role {
        Role::Worker => ("agent", "a worker"),
        Role::Server => ("server", "a server (control plane and etcd)"),
    };
    push(&mut s, &format!("Join {} as {runs}", plan.server));
    push(
        &mut s,
        &format!(
            "{title} {what} {} ({}), CIS hardened, on {} ({}, {} MiB)",
            cluster.version,
            support(cluster.pinned),
            plan.facts.hostname,
            plan.facts.arch,
            plan.facts.memory_mib
        ),
    );
    node_name(&mut s, &plan.node_name);
    node_addresses(&mut s, &plan.facts.node_ips);
    push(&mut s, "");
    let token = format!(
        "{} (the token, mode 0600)",
        cluster.distribution.paths().join_token_file
    );
    files(&mut s, &plan.host_file_paths, Some(&token));
    removed(&mut s, &plan.removed_files);
    match &plan.rtt {
        Some(rtt) => push(
            &mut s,
            &format!(
                "Distance         {} ms to the server (median of {} TCP connects; {} to {})",
                join::ms(rtt.median_us),
                rtt.samples,
                join::ms(rtt.min_us),
                join::ms(rtt.max_us)
            ),
        ),
        None => push(&mut s, "Distance         the server did not answer"),
    }
    ports(&mut s, plan);
    findings(&mut s, &plan.findings);
    s
}

/// The files `apply` writes, and `extra` after them.
fn files(s: &mut String, paths: &[String], extra: Option<&str>) {
    push(s, "Files");
    for path in paths {
        push(s, &format!("  {path}"));
    }
    if let Some(extra) = extra {
        push(s, &format!("  {extra}"));
    }
    push(s, "");
}

/// The files of registry and proxy settings that `apply` removes, if any.
fn removed(s: &mut String, paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    push(s, "Removes");
    for path in paths {
        push(s, &format!("  {path}"));
    }
    push(s, "");
}

/// The failed findings as errors, then the others as warnings.
fn findings(s: &mut String, findings: &[Check]) {
    for (status, label) in [(Status::Fail, "error"), (Status::Warn, "warning")] {
        for check in findings.iter().filter(|c| c.status == status) {
            push(
                s,
                &format!("{label}: {}", (i18n::ENGLISH.finding)(&check.finding)),
            );
        }
    }
}

/// The ports on the server this VM must reach, and the ones on this VM that must be free.
fn ports(s: &mut String, plan: &JoinPlan) {
    push(s, "Ports on the server");
    for port in &plan.ports {
        let state = match port.open {
            Some(true) => "open   ",
            Some(false) => "CLOSED ",
            None => "unknown",
        };
        push(
            s,
            &format!(
                "  {state} {} {:<5} {}",
                port.protocol.to_ascii_uppercase(),
                port.number,
                i18n::port_use(port.purpose)
            ),
        );
    }
    if plan.unit_running {
        let numbers: Vec<String> = plan
            .local_ports
            .iter()
            .map(|p| p.number.to_string())
            .collect();
        push(
            s,
            &format!(
                "Ports on this VM: {} belong to the running {}",
                numbers.join(", "),
                plan.unit
            ),
        );
        push(s, "");
        return;
    }
    push(
        s,
        "Ports on this VM, free before the node starts (apply checks them)",
    );
    for port in &plan.local_ports {
        push(
            s,
            &format!(
                "  {} {:<5} {}",
                port.protocol.to_ascii_uppercase(),
                port.number,
                i18n::port_use(port.purpose)
            ),
        );
    }
    push(s, "");
}

const fn support(pinned: bool) -> &'static str {
    if pinned {
        "pinned, checked against zeatend's digests"
    } else {
        "UNSUPPORTED"
    }
}

fn node_name(s: &mut String, name: &str) {
    push(s, &format!("Node name        {name}"));
}

/// The addresses the node registers with.
fn node_addresses(s: &mut String, addresses: &[String]) {
    let text = if addresses.is_empty() {
        "chosen by Kubernetes: this host's addresses could not be read".to_owned()
    } else {
        format!(
            "{} (temporary IPv6 addresses left out)",
            addresses.join(", ")
        )
    };
    push(s, &format!("Node addresses   {text}"));
}

fn push(s: &mut String, line: &str) {
    s.push_str(line);
    s.push('\n');
}

#[cfg(test)]
mod tests {
    use zeatend_plan::config::{Config, JoinConfig};
    use zeatend_plan::join::JoinFacts;
    use zeatend_plan::plan::Facts;
    use zeatend_plan::record::VersionChange;

    use super::*;

    /// A worker that joins a K3s cluster.
    const WORKER: &str =
        "distribution: k3s\njoin:\n  server: https://10.0.0.5:6443\n  tokenFile: /root/token\n";

    /// Five TCP connect times to the server's join port, in microseconds, as `plan` measures them.
    /// Under 1 ms: the same region.
    const SAME_REGION_US: [u64; 5] = [640, 610, 700, 900, 650];
    /// About 24 ms: another region.
    const OTHER_REGION_US: [u64; 5] = [24_000, 23_500, 25_100, 24_200, 23_900];

    fn host() -> Facts {
        Facts {
            arch: "amd64".to_owned(),
            hostname: "vps-2".to_owned(),
            node_name: None,
            host_hex: "a1b2c3".to_owned(),
            memory_mib: 8192,
            node_ips: vec!["10.0.0.6".to_owned(), "fd00::6".to_owned()],
            record: None,
            kubernetes: None,
            apparmor_restricts_unconfined: false,
            k3s_own_network: false,
            installer_webhook: false,
            registry_files: Vec::new(),
            registry_passwords: std::collections::BTreeMap::new(),
        }
    }

    /// A fresh VM that reached the server in `rtt_us`; none means the server did not answer.
    fn facts(rtt_us: &[u64]) -> JoinFacts {
        JoinFacts {
            facts: host(),
            rtt_us: rtt_us.to_vec(),
            ports: vec![
                (9345, !rtt_us.is_empty()),
                (6443, !rtt_us.is_empty()),
                (10250, true),
                (2379, true),
                (2380, false),
            ],
            token_pinned: true,
            unit_running: false,
        }
    }

    fn join_of(yaml: &str, facts: &JoinFacts) -> JoinPlan {
        join::plan(&JoinConfig::parse(yaml).unwrap(), facts).unwrap()
    }

    #[test]
    fn a_worker_near_the_server() {
        let p = join_of(WORKER, &facts(&SAME_REGION_US));
        let text = join_plan(&p).replace(&p.cluster.version, "<version>");
        assert_eq!(text, include_str!("../fixtures/join-worker.txt"));
    }

    #[test]
    fn a_server_far_from_the_server() {
        let p = join_of(
            "distribution: rke2\njoin:\n  server: https://10.0.0.5:9345\n  role: server\n  \
             tokenFile: /root/token\n",
            &facts(&OTHER_REGION_US),
        );
        let text = join_plan(&p).replace(&p.cluster.version, "<version>");
        assert_eq!(text, include_str!("../fixtures/join-server.txt"));
    }

    #[test]
    fn an_unreachable_server_is_an_error() {
        let text = join_plan(&join_of(WORKER, &facts(&[])));
        assert!(text.contains("Distance         the server did not answer"));
        assert!(text.contains("error: Cannot connect to https://10.0.0.5:6443."));
    }

    #[test]
    fn a_node_that_joined_already_holds_its_own_ports() {
        let mut joined = facts(&SAME_REGION_US);
        joined.unit_running = true;
        let text = join_plan(&join_of(WORKER, &joined));
        assert!(
            text.contains("Ports on this VM: 80, 443, 10250 belong to the running k3s-agent"),
            "{text}"
        );
        assert!(!text.contains("free before the node starts"), "{text}");
    }

    #[test]
    fn a_first_server_names_the_release_and_its_files() {
        for (yaml, first) in [
            ("distribution: k3s\n", "K3s "),
            ("distribution: rke2\n", "RKE2 "),
            ("distribution: kubesolo\n", "KubeSolo "),
        ] {
            let config = Config::parse(yaml).unwrap();
            let p = zeatend_plan::plan::plan(&config, &host()).unwrap();
            let text = plan(&p);
            assert!(text.starts_with(first), "{text}");
            assert!(
                text.contains("pinned, checked against zeatend's digests"),
                "{text}"
            );
            assert!(text.contains("Node name        vps-2"), "{text}");
            for path in &p.host_file_paths {
                assert!(text.contains(path.as_str()), "{path} in\n{text}");
            }
        }
    }

    #[test]
    fn an_upgrade_names_what_changes() {
        let mut changes = Changes {
            written_by: "zeatend 0.1.1".to_owned(),
            version: None,
            host_files: Vec::new(),
            restart: false,
        };
        assert!(super::changes(Some(&changes), "k3s").contains("  nothing"));
        changes.set_host_files(vec!["/etc/rancher/k3s/config.yaml".to_owned()]);
        let text = super::changes(Some(&changes), "k3s");
        assert!(text.starts_with("Changes since the last apply (by zeatend 0.1.1)\n"));
        assert!(text.contains("/etc/rancher/k3s/config.yaml"), "{text}");
        assert!(text.contains("restarts k3s"), "{text}");

        let release = Changes {
            version: Some(VersionChange {
                from: "v1.2.2".to_owned(),
                to: "v1.2.3".to_owned(),
            }),
            host_files: Vec::new(),
            restart: true,
            ..changes
        };
        let text = super::changes(Some(&release), "kubesolo");
        assert!(text.contains("v1.2.2 -> v1.2.3") && text.contains("restarts kubesolo"));
        assert_eq!(super::changes(None, "k3s"), "");
    }
}
