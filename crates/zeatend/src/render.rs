//! The plan, written for people.

use zeatend_host::preflight::Finding;
use zeatend_plan::config::Role;
use zeatend_plan::join::{self, JoinPlan};
use zeatend_plan::kubesolo;
use zeatend_plan::plan::{Plan, Release, Warning};
use zeatend_plan::record::Changes;

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
                    cluster.distribution.title(),
                    cluster.version,
                    support(cluster.pinned),
                ),
            );
            cluster.distribution.server_unit()
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
            kubesolo::UNIT
        }
    };
    node_name(&mut s, &plan.node_name);
    node_addresses(&mut s, &facts.node_ips);
    push(&mut s, "");
    s.push_str(&changes(plan.changes.as_ref(), unit));
    files(&mut s, &plan.host_file_paths, None);
    warnings(&mut s, &plan.warnings, "warning");
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
        &format!(
            "Changes since the last apply (by zeatend {})",
            c.zeatend_version
        ),
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
    let title = cluster.distribution.title();
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
        cluster.distribution.layout().join_token_file
    );
    files(&mut s, &plan.host_file_paths, Some(&token));
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
    warnings(&mut s, &plan.blockers, "error");
    warnings(&mut s, &plan.warnings, "warning");
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

fn warnings(s: &mut String, warnings: &[Warning], label: &str) {
    for warning in warnings {
        push(s, &format!("{label}: {}", warning.text));
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
                port.purpose
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
                port.purpose
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

/// A preflight finding as an English sentence: what was found and what to do.
pub(crate) fn finding(finding: &Finding) -> String {
    match finding {
        Finding::RootOk => "running as root".to_owned(),
        Finding::RootNo => "run apply as root (sudo)".to_owned(),
        Finding::SystemdOk => "systemd is the init system".to_owned(),
        Finding::SystemdNo => {
            "Kubernetes runs as a systemd service; this host does not run systemd".to_owned()
        }
        Finding::TimeOk => "the clock is synchronised".to_owned(),
        Finding::TimeDrift => "the clock is not synchronised; certificates and tokens fail when \
                               it drifts (timedatectl set-ntp true)"
            .to_owned(),
        Finding::TimeUnknown => {
            "could not ask timedatectl whether the clock is synchronised".to_owned()
        }
        Finding::SwapOff => "swap is off".to_owned(),
        Finding::SwapOn => "swap is on; the kubelet expects it off (swapoff -a, and remove it \
                            from /etc/fstab)"
            .to_owned(),
        Finding::SwapUnknown => "could not read /proc/swaps".to_owned(),
        Finding::CrashNone => "no memory kept for a crash kernel".to_owned(),
        Finding::CrashKept { mib } => format!(
            "the kernel keeps {mib} MiB for a crash kernel (kdump); to free it: sudo apt purge \
             kdump-tools && sudo update-grub, then reboot"
        ),
        Finding::OthersNone => "none found".to_owned(),
        Finding::OthersFound { found } => {
            let named: Vec<String> = found
                .iter()
                .map(|l| format!("{} ({})", l.distribution, l.path))
                .collect();
            format!(
                "found files of {}; if it still runs, uninstall it first",
                named.join(", ")
            )
        }
        Finding::PortsFree { ports } => {
            let numbers: Vec<String> = ports.iter().map(u16::to_string).collect();
            format!("{} are free", and_list(&numbers))
        }
        Finding::PortsBusy { ports } => {
            let named: Vec<String> = ports
                .iter()
                .map(|p| format!("{} ({})", p.port, p.purpose))
                .collect();
            format!("in use: {}; `ss -ltnp` shows by what", named.join(", "))
        }
        Finding::DiskUnknown { path } => format!("could not read the free space of {path}"),
        Finding::DiskLow {
            free_tenths_gib,
            path,
            needed_gib,
        } => format!(
            "{} GiB free under {path}; the cluster's images and data need at least {needed_gib} GiB",
            gib(*free_tenths_gib)
        ),
        Finding::DiskTight {
            free_tenths_gib,
            path,
            roomy_gib,
        } => format!(
            "{} GiB free under {path}; {roomy_gib} GiB or more leaves room for images, logs and \
             volumes",
            gib(*free_tenths_gib)
        ),
        Finding::DiskOk {
            free_tenths_gib,
            path,
        } => format!("{} GiB free under {path}", gib(*free_tenths_gib)),
        Finding::CryptYes { paths } => {
            let named: Vec<String> = paths.iter().map(|p| (*p).to_owned()).collect();
            let verb = if paths.len() == 1 { "is" } else { "are" };
            format!("{} {verb} on dm-crypt devices", and_list(&named))
        }
        Finding::CryptRequiredPlain { path } => {
            format!("{path} is not on an encrypted device, and host.requireEncryptedDisk is set")
        }
        Finding::CryptRequiredUnknown { path } => format!(
            "could not tell whether {path} is encrypted, and host.requireEncryptedDisk is set"
        ),
        Finding::CryptNo { path } => format!(
            "{path} is not on an encrypted disk, so volumes and databases are stored \
             unencrypted; most VPS plans can't encrypt it afterwards, so for sensitive data \
             choose a plan whose provider encrypts disks"
        ),
        Finding::CryptUnknown { path } => format!("could not tell whether {path} is encrypted"),
        Finding::DockerNone => "no Docker, and the iptables FORWARD policy is not DROP".to_owned(),
        Finding::DockerFound { found } => format!(
            "Docker is on this host ({}). It sets the iptables FORWARD policy to DROP, so Pods \
             cannot reach the internet. Stop and remove Docker first",
            found.join(", ")
        ),
        Finding::ForwardDrop => "the iptables FORWARD policy is DROP, so Pods cannot reach the \
                                 internet; find what sets it (Docker, ufw) and change it"
            .to_owned(),
    }
}

/// Tenths of a GiB as GiB with one decimal.
fn gib(tenths: u64) -> String {
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// `a, b and c`.
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
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
            apparmor_restricts_unconfined: false,
            k3s_own_network: false,
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
            zeatend_version: "0.1.1".to_owned(),
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

    #[test]
    fn preflight_findings_of_the_host_in_english() {
        let cases = [
            (Finding::RootOk, "running as root"),
            (Finding::RootNo, "run apply as root (sudo)"),
            (Finding::SystemdOk, "systemd is the init system"),
            (
                Finding::SystemdNo,
                "Kubernetes runs as a systemd service; this host does not run systemd",
            ),
            (Finding::TimeOk, "the clock is synchronised"),
            (
                Finding::TimeDrift,
                "the clock is not synchronised; certificates and tokens fail when it drifts (timedatectl set-ntp true)",
            ),
            (
                Finding::TimeUnknown,
                "could not ask timedatectl whether the clock is synchronised",
            ),
            (Finding::SwapOff, "swap is off"),
            (
                Finding::SwapOn,
                "swap is on; the kubelet expects it off (swapoff -a, and remove it from /etc/fstab)",
            ),
            (Finding::SwapUnknown, "could not read /proc/swaps"),
            (Finding::CrashNone, "no memory kept for a crash kernel"),
            (
                Finding::CrashKept { mib: 512 },
                "the kernel keeps 512 MiB for a crash kernel (kdump); to free it: sudo apt purge kdump-tools && sudo update-grub, then reboot",
            ),
        ];
        for (f, text) in cases {
            assert_eq!(finding(&f), text, "{f:?}");
        }
    }

    #[test]
    fn preflight_findings_with_paths_and_ports_in_english() {
        use zeatend_host::preflight::{BusyPort, Leftover};
        let cases = [
            (Finding::OthersNone, "none found"),
            (
                Finding::OthersFound {
                    found: vec![
                        Leftover {
                            distribution: "K3s",
                            path: "/usr/local/bin/k3s",
                        },
                        Leftover {
                            distribution: "k0s",
                            path: "/var/lib/k0s",
                        },
                    ],
                },
                "found files of K3s (/usr/local/bin/k3s), k0s (/var/lib/k0s); if it still runs, uninstall it first",
            ),
            (
                Finding::PortsFree {
                    ports: vec![80, 443, 6443, 10250],
                },
                "80, 443, 6443 and 10250 are free",
            ),
            (
                Finding::PortsBusy {
                    ports: vec![
                        BusyPort {
                            port: 6443,
                            purpose: "Kubernetes API",
                        },
                        BusyPort {
                            port: 10250,
                            purpose: "kubelet",
                        },
                    ],
                },
                "in use: 6443 (Kubernetes API), 10250 (kubelet); `ss -ltnp` shows by what",
            ),
        ];
        for (f, text) in cases {
            assert_eq!(finding(&f), text, "{f:?}");
        }
    }

    #[test]
    fn preflight_findings_of_disks_and_docker_in_english() {
        let var_lib = || "/var/lib".to_owned();
        let cases = [
            (
                Finding::DiskUnknown { path: var_lib() },
                "could not read the free space of /var/lib",
            ),
            (
                Finding::DiskLow {
                    free_tenths_gib: 31,
                    path: var_lib(),
                    needed_gib: 10,
                },
                "3.1 GiB free under /var/lib; the cluster's images and data need at least 10 GiB",
            ),
            (
                Finding::DiskTight {
                    free_tenths_gib: 100,
                    path: var_lib(),
                    roomy_gib: 30,
                },
                "10.0 GiB free under /var/lib; 30 GiB or more leaves room for images, logs and volumes",
            ),
            (
                Finding::DiskOk {
                    free_tenths_gib: 468,
                    path: var_lib(),
                },
                "46.8 GiB free under /var/lib",
            ),
            (
                Finding::CryptYes { paths: vec!["/"] },
                "/ is on dm-crypt devices",
            ),
            (
                Finding::CryptYes {
                    paths: vec!["/", "/var/lib/rancher", "/etc/rancher/k3s"],
                },
                "/, /var/lib/rancher and /etc/rancher/k3s are on dm-crypt devices",
            ),
            (
                Finding::CryptRequiredPlain { path: "/" },
                "/ is not on an encrypted device, and host.requireEncryptedDisk is set",
            ),
            (
                Finding::CryptRequiredUnknown { path: "/" },
                "could not tell whether / is encrypted, and host.requireEncryptedDisk is set",
            ),
            (
                Finding::CryptNo { path: "/" },
                "/ is not on an encrypted disk, so volumes and databases are stored unencrypted; most VPS plans can't encrypt it afterwards, so for sensitive data choose a plan whose provider encrypts disks",
            ),
            (
                Finding::CryptUnknown { path: "/" },
                "could not tell whether / is encrypted",
            ),
            (
                Finding::DockerNone,
                "no Docker, and the iptables FORWARD policy is not DROP",
            ),
            (
                Finding::DockerFound {
                    found: vec!["/usr/bin/docker".to_owned(), "docker.service".to_owned()],
                },
                "Docker is on this host (/usr/bin/docker, docker.service). It sets the iptables FORWARD policy to DROP, so Pods cannot reach the internet. Stop and remove Docker first",
            ),
            (
                Finding::ForwardDrop,
                "the iptables FORWARD policy is DROP, so Pods cannot reach the internet; find what sets it (Docker, ufw) and change it",
            ),
        ];
        for (f, text) in cases {
            assert_eq!(finding(&f), text, "{f:?}");
        }
    }
}
