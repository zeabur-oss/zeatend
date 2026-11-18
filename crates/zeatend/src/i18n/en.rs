//! English.

use zeatend_host::preflight::{Finding, Status, Topic};
use zeatend_plan::cluster::PortUse;

use super::{Messages, gib};

pub(super) const MESSAGES: Messages = Messages {
    checking_host: "Checking the host",
    apply_stops: "apply stops until what failed is fixed.",
    host_not_ready: "the host is not ready; fix what failed and run apply again",
    status,
    topic,
    finding,
};

const fn status(status: Status) -> &'static str {
    match status {
        Status::Pass => "ok  ",
        Status::Warn => "warn",
        Status::Fail => "FAIL",
    }
}

const fn topic(topic: Topic) -> &'static str {
    match topic {
        Topic::Root => "root",
        Topic::Systemd => "systemd",
        Topic::Time => "time",
        Topic::Swap => "swap",
        Topic::CrashKernel => "crash kernel",
        Topic::OtherKubernetes => "other Kubernetes",
        Topic::Ports => "ports",
        Topic::DiskSpace => "disk space",
        Topic::DiskEncryption => "disk encryption",
        Topic::Docker => "Docker",
    }
}

/// `a, b and c`.
pub(crate) fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn finding(finding: &Finding) -> String {
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
                .map(|p| format!("{} ({})", p.port, port_use(p.purpose)))
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

pub(crate) const fn port_use(port_use: PortUse) -> &'static str {
    match port_use {
        PortUse::KubernetesApi => "Kubernetes API",
        PortUse::K3sApiAndSupervisor => "Kubernetes API and K3s supervisor",
        PortUse::Rke2Supervisor => "RKE2 supervisor",
        PortUse::Kubelet => "kubelet",
        PortUse::EtcdClient => "etcd client",
        PortUse::EtcdPeer => "etcd peer",
        PortUse::ServiceLbHttp => {
            "HTTP ingress: ServiceLB holds it on every node for a LoadBalancer Service"
        }
        PortUse::ServiceLbHttps => {
            "HTTPS ingress: ServiceLB holds it on every node for a LoadBalancer Service"
        }
        PortUse::KubeSoloHttp => {
            "HTTP ingress: KubeSolo gives a LoadBalancer Service this host's address"
        }
        PortUse::KubeSoloHttps => {
            "HTTPS ingress: KubeSolo gives a LoadBalancer Service this host's address"
        }
        PortUse::KubeSoloWebhook => "KubeSolo's node-assignment webhook",
        PortUse::FlannelVxlan => "flannel VXLAN",
        PortUse::CanalVxlan => "Canal VXLAN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                            purpose: PortUse::KubernetesApi,
                        },
                        BusyPort {
                            port: 10250,
                            purpose: PortUse::Kubelet,
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
