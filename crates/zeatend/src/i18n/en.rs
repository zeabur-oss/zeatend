//! English.

use zeatend_plan::cluster::{Distribution, PortUse, Rancher};
use zeatend_plan::finding::{Finding, Status, Topic};
use zeatend_plan::join::{RTT_CRITICAL_US, RTT_WARN_US, ms};
use zeatend_plan::longhorn::{MODULES_FILE, MULTIPATH_FILE, Need};
use zeatend_plan::precondition::Stale;
use zeatend_plan::record::ZEATEND_VERSION;

use super::{Joined, Key, Messages, Ready, Screen, Screens, gib};

pub(super) const MESSAGES: Messages = Messages {
    checking_host: "Checking the host",
    preparing_longhorn: "Preparing this node for Longhorn",
    apply_stops: "apply stops until what failed is fixed.",
    host_not_ready: "the host is not ready; fix what failed and run apply again",
    not_plan: |path, reason| {
        format!(
            "nothing was changed: {path} is not a plan that `zeatend plan --json` printed ({reason})"
        )
    },
    stale,
    doctor_needs_root: "run doctor as root (sudo): the checks read the cluster's files and \
                        credentials",
    no_config_installer,
    no_config_defaults,
    following_unit,
    started_unit,
    telemetry_on: "zeatend sends its version, the distribution and whether the install worked, \
                   to help improve Zeatend. --no-telemetry or DO_NOT_TRACK=1 turns it off. What is \
                   sent: https://oss.zeabur.com/docs/zeatend/reference/telemetry/",
    starting_unit,
    restarting_unit,
    waiting_for_node,
    done: DONE,
    ready,
    joined,
    status,
    topic,
    finding,
    screens: &SCREENS,
};

const SCREENS: Screens = Screens {
    screen: |screen| match screen {
        Screen::Server => "Server",
        Screen::Access => "Access",
        Screen::Review => "Review",
        Screen::Install => "Install",
    },
    key: |key| match key {
        Key::Choose => "choose",
        Key::Next => "continue",
        Key::Back => "back",
        Key::Field => "next field",
        Key::Language => "language",
        Key::Quit => "quit",
        Key::Install => "install",
        Key::Leave => "close, the install goes on",
        Key::Close => "close",
    },

    server_title: "This server",
    host_name: "Host name",
    system: "System",
    cpu: "CPU",
    memory: "Memory",
    addresses: "Addresses",
    cpus: |n| {
        if n == 1 {
            "1 core".to_owned()
        } else {
            format!("{n} cores")
        }
    },
    private_address: |address| format!("{address} (private)"),
    no_address: "none",
    distribution_title: "Which Kubernetes to install",
    distribution_about: |distribution| match distribution {
        Distribution::KubeSolo => "One node, and the lightest. It takes no workers.",
        Distribution::K3s => "Light, hardened like RKE2's CIS profile. You can add workers later.",
        Distribution::Rke2 => "RKE2's CIS profile, for a control plane of three servers.",
    },
    checks_title: "Host checks",
    checks_summary: |failed, warned| match (failed, warned) {
        (0, 0) => "all passed".to_owned(),
        (0, 1) => "1 warning".to_owned(),
        (0, w) => format!("{w} warnings"),
        (f, 0) => format!("{f} failed"),
        (f, w) => format!(
            "{f} failed, {w} {}",
            if w == 1 { "warning" } else { "warnings" }
        ),
    },
    checks_stop: "Fix what failed, then run sudo zeatend again.",

    access_title: "How your computer reaches the cluster",
    access_private: "This server has only private addresses. Which address does your computer \
                     reach it at?",
    access_public: "Your computer reaches the API server at this address. Change it if it uses \
                    another one.",
    field_address: "Address",
    field_address_hint: "The API server's certificate covers it, so kubectl can connect.",
    field_name: "DNS name (optional)",
    field_name_hint: "A name that points at this server. The certificate covers it too.",
    field_node: "Node name (optional)",
    field_node_hint: |name| format!("Left empty, the node is {name}."),
    bad_address: "Enter an IP address, such as 203.0.113.7.",
    bad_name: "Enter a DNS name, such as k8s.example.com.",
    bad_node: "Use lower-case letters, digits, - and ., starting and ending with a letter or digit.",

    review_title: "Check before installing",
    review_release: "Kubernetes",
    review_address: "Address",
    review_name: "DNS name",
    review_no_name: "none, kubectl uses the address",
    review_node: "Node name",
    review_config: "Configuration",
    review_next: "What happens next",
    next_write: |path| format!("zeatend writes these choices to {path}."),
    next_update: |path| format!("zeatend saves these choices in {path}. Its other settings stay."),
    next_unit: "The install runs in a systemd unit, and goes on if this SSH session ends.",
    next_follow: "Run sudo zeatend again to follow it.",
    next_telemetry: "The install sends its version, the distribution and whether it worked, to \
                     help improve Zeatend. To turn it off, quit and run sudo zeatend \
                     --no-telemetry.",

    install_title: "Installing",
    install_unit: |unit| format!("Runs in {unit}"),
    install_elapsed: |time| format!("Elapsed {time}"),
    install_starting: "Starting the install",
    install_done: "The cluster is ready.",
    install_failed: "The install stopped.",
    duration: |minutes, seconds| {
        if minutes == 0 {
            format!("{seconds} s")
        } else {
            format!("{minutes} min {seconds} s")
        }
    },
    log_title: "Log",
    result_title: "Next",
    result_more: "Enter closes the screens and prints all of it in the terminal.",

    language_title: "Language",
    language_hint: "Changes right away.",
    language_needs_utf8: "needs a UTF-8 terminal",

    reading_host: "Reading this host",
    left: |unit| format!("The install goes on in {unit}. Run `sudo zeatend` to follow it again."),
    no_terminal: "zeatend opens its screens in a terminal. Without one, write \
                  /etc/zeatend/config.yaml and run:\n  sudo zeatend plan\n  sudo zeatend apply",
    needs_root: "run zeatend as root: sudo zeatend",
    needs_systemd: "this host does not run systemd, which the screens need. Write \
                    /etc/zeatend/config.yaml and run sudo zeatend apply --foreground",
    joins: |path| format!("{path} joins this host to a cluster. Run sudo zeatend apply"),
};

const DONE: &str = "Done";

fn no_config_installer(config: &str, installer: &str) -> String {
    format!(
        "No configuration at {config}; using the distribution and host settings of {installer}, \
         which zeabur-installer wrote."
    )
}

fn no_config_defaults(config: &str) -> String {
    format!("No configuration at {config}; using the defaults: K3s on this host.")
}

fn following_unit(unit: &str) -> String {
    format!("An install runs in {unit}; following it.")
}

fn started_unit(unit: &str) -> String {
    format!(
        "The install runs in {unit}. If this session ends, run `sudo zeatend apply` to follow it \
         again."
    )
}

fn starting_unit(unit: &str) -> String {
    format!("starting {unit}; the first start takes a few minutes")
}

fn restarting_unit(unit: &str) -> String {
    format!("restarting {unit} for the new configuration")
}

fn waiting_for_node(node: &str) -> String {
    format!("Waiting for node {node} to be Ready")
}

fn ready(r: &Ready<'_>) -> String {
    let host = r.host.unwrap_or("<this server>");
    let node = r.node;
    let sed = r.rewrite.map_or_else(String::new, |(from, to)| {
        format!("  sed -i.bak 's#{from}#{to}#' ~/.kube/{node}.yaml\n")
    });
    let mut text = format!(
        "{} runs on {node}.\n\nList the nodes on this server:\n  {} get nodes -o wide\n\n\
         Use the cluster from your own computer:\n  \
         mkdir -p ~/.kube\n  (umask 077; ssh <you>@{host} sudo cat {} > ~/.kube/{node}.yaml)\n{sed}  KUBECONFIG=~/.kube/{node}.yaml kubectl get nodes\n\n\
         Whoever has this file is cluster admin; keep it like an SSH key.",
        r.cluster, r.kubectl, r.kubeconfig
    );
    if r.rewrite.is_none() {
        text.push_str(
            " If your computer reaches the server at another address, change `server:` in the \
             file and add the address to host.tlsSan.",
        );
    }
    if r.add_nodes {
        text.push_str(
            "\n\nTo add a node, run `sudo zeatend node add` on this server. It prints the \
             commands to run on the new host.",
        );
    }
    text
}

fn joined(j: &Joined<'_>) -> String {
    let role = if j.server { "server" } else { "worker" };
    format!(
        "{} joined the cluster as a {role}. List the nodes on a server:\n  {} get nodes -o wide",
        j.node, j.kubectl
    )
}

fn stale(stale: &Stale) -> String {
    let why = match stale {
        Stale::Writes => "the plan in the file is not what apply would do now: the configuration, \
                          this host or zeatend changed"
            .to_owned(),
        Stale::File(path) => format!("{path} changed after the plan was made"),
        Stale::Release { reviewed, now } => format!(
            "the installed release was {} when the plan was made and is {} now",
            reviewed.as_deref().unwrap_or("none"),
            now.as_deref().unwrap_or("none")
        ),
        Stale::Record => "the install record changed after the plan was made".to_owned(),
    };
    format!("nothing was changed: {why}. Run `zeatend plan --json` again and review the new plan.")
}

const fn status(status: Status) -> &'static str {
    match status {
        Status::Pass => "ok  ",
        Status::Warn => "warn",
        Status::Fail => "FAIL",
    }
}

pub(super) const fn topic(topic: Topic) -> &'static str {
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
        Topic::Service => "service",
        Topic::Api => "Kubernetes API",
        Topic::InstallRecord => "install record",
        Topic::Nodes => "nodes",
        Topic::KubeletReserves => "kubelet reserves",
        Topic::SecretsEncryption => "Secrets encryption",
        Topic::Memory => "memory",
        Topic::Release => "release",
        Topic::Server => "server",
        Topic::Firewall => "firewall",
        Topic::HighAvailability => "high availability",
        Topic::Storage => "storage",
        Topic::JoinToken => "join token",
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

#[expect(
    clippy::too_many_lines,
    reason = "one arm for each finding, so a finding without words does not compile"
)]
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
             unencrypted. Most VPS plans can't encrypt it afterwards, so for sensitive data \
             choose a plan whose provider encrypts disks."
        ),
        Finding::CryptUnknown { path } => format!("could not tell whether {path} is encrypted"),
        Finding::DockerNone => "no Docker, and the iptables FORWARD policy is not DROP".to_owned(),
        Finding::DockerFound { found } => format!(
            "Docker is on this host ({}). It sets the iptables FORWARD policy to DROP, so Pods \
             cannot reach the internet. Stop and remove Docker first.",
            found.join(", ")
        ),
        Finding::ForwardDrop => "the iptables FORWARD policy is DROP, so Pods cannot reach the \
                                 internet. Find what sets it (Docker, ufw) and change it."
            .to_owned(),
        Finding::NotInstalled => "K3s, RKE2 and KubeSolo are not installed on this host; install \
                                  with `sudo zeatend apply`"
            .to_owned(),
        Finding::ServiceRunning { unit } => format!("{unit} is running"),
        Finding::ServiceStopped { unit } => format!(
            "{unit} is not running; `sudo journalctl -u {unit} -n 100` shows why, and `sudo \
             systemctl restart {unit}` starts it again"
        ),
        Finding::ApiReady => "the API server answers /readyz".to_owned(),
        Finding::ApiDown { unit } => format!(
            "the API server does not answer /readyz within 30 seconds; `sudo journalctl -u {unit} \
             -n 100` shows why"
        ),
        Finding::RecordOk { writer } => format!("{writer} wrote the install record"),
        Finding::RecordMissing { path } => format!(
            "no install record at {path}: zeatend did not install this node, or apply stopped \
             before the end. Run `sudo zeatend apply`."
        ),
        Finding::RecordNewer { zeatend_version } => format!(
            "zeatend {zeatend_version} wrote the install record, and this is zeatend \
             {ZEATEND_VERSION}. Use zeatend {zeatend_version} or newer."
        ),
        Finding::NodesReady { names } => format!("{} {} Ready", and_list(names), be(names.len())),
        Finding::NodesNotReady { names, command } => format!(
            "{} {} not Ready; `{command}` shows why",
            and_list(names),
            be(names.len())
        ),
        Finding::NodesNone { unit } => format!(
            "the cluster has no nodes; `sudo journalctl -u {unit} -n 100` shows why the kubelet \
             did not register"
        ),
        Finding::NodesUnknown { reason } => format!("could not read the nodes: {reason}"),
        Finding::ReservesSet { names } => format!(
            "the kubelet on {} keeps at least the memory the OS needs from pods",
            and_list(names)
        ),
        Finding::ReservesLow {
            nodes,
            needed_mib,
            distribution,
        } => {
            let kept: Vec<String> = nodes
                .iter()
                .map(|k| format!("{} MiB on {}", k.mib, k.name))
                .collect();
            format!(
                "the kubelet keeps only {} from pods, less than the {needed_mib} MiB the OS needs \
                 at idle, so pods can take the memory the OS and the kubelet need (no \
                 system-reserved or kube-reserved). {}.",
                and_list(&kept),
                reserves_fix(*distribution)
            )
        }
        Finding::ReservesUnknown { name } => {
            format!("node {name} reports no memory capacity or allocatable memory")
        }
        Finding::SecretsEncrypted { provider } => {
            format!("the API server stores Secrets encrypted ({provider})")
        }
        Finding::SecretsPlain { distribution } => format!(
            "the API server stores Secrets unencrypted; `sudo {} secrets-encrypt status` shows \
             the state",
            distribution.distribution().id()
        ),
        Finding::MemoryLow {
            have_mib,
            min_mib,
            distribution: Distribution::KubeSolo,
        } => format!(
            "This host has {have_mib} MiB of memory, and KubeSolo needs at least {min_mib} MiB."
        ),
        Finding::MemoryLow {
            have_mib,
            min_mib,
            distribution,
        } => format!(
            "This host has {have_mib} MiB of memory, and {} needs at least {min_mib} MiB, so \
             some pods may not start.",
            distribution.title()
        ),
        Finding::ReleaseUnpinned {
            distribution,
            version,
        } => format!(
            "{} {version} is not supported by this zeatend release: its downloads are checked \
             only against the release's own checksum list.",
            distribution.distribution().title()
        ),
        Finding::ServerUnreachable { server, port } => format!(
            "Cannot connect to {server}. Check the address and that TCP {port} is open from this \
             VM."
        ),
        Finding::ServerFar {
            median_us,
            samples,
            port,
        } => format!(
            "The server is {} ms away (median of {samples} TCP connects to port {port}); within a \
             region it is usually under {} ms.",
            ms(*median_us),
            ms(RTT_WARN_US)
        ),
        Finding::ServerTooFar {
            median_us,
            samples,
            port,
        } => format!(
            "The server is {} ms away (median of {samples} TCP connects to port {port}). Above {} \
             ms the cluster will be slow and unstable.",
            ms(*median_us),
            ms(RTT_CRITICAL_US)
        ),
        Finding::SameRegion => "Put every node at the same hosting provider and in the same \
                                region. Different availability zones in one region are fine. \
                                Between providers or regions, latency slows etcd, the Kubernetes \
                                API and storage, and can make the cluster unstable."
            .to_owned(),
        Finding::ServerPortsClosed { ports, host } => {
            let numbers: Vec<String> = ports.iter().map(u16::to_string).collect();
            format!(
                "TCP {} on {host} cannot be reached from this VM; check the firewall or security \
                 group.",
                numbers.join(", ")
            )
        }
        Finding::JoinPorts { distribution } => join_ports(*distribution),
        Finding::UdpUnchecked { port } => format!(
            "UDP {port} cannot be checked from here. If it is blocked, pods on different nodes \
             cannot reach each other."
        ),
        Finding::ServersHa => "The control plane survives losing a server only with three or \
                               more servers: etcd needs a majority. With two, losing either one \
                               stops the cluster, so add a third."
            .to_owned(),
        Finding::LocalStorage => "Volumes on the default local-path storage class stay on the \
                                  node that created them: a pod using one runs only on that \
                                  node, and the data goes with the node."
            .to_owned(),
        Finding::TokenUnpinned => "The token has no CA hash (K10<hash>::…), so this VM trusts \
                                   whichever server answers at that address. Use the token with \
                                   its CA hash, as the server's token file holds it."
            .to_owned(),
        Finding::LonghornReady => {
            "this node has the iSCSI initiator, the NFS client, the modules and iscsid that \
             Longhorn needs"
                .to_owned()
        }
        Finding::LonghornMissing { needs } => format!(
            "Longhorn cannot attach volumes on this node: it lacks {}. Run `sudo zeatend apply`",
            and_list(&needs.iter().map(need).collect::<Vec<_>>())
        ),
    }
}

/// Something a node lacks for Longhorn.
fn need(need: &Need) -> String {
    match need {
        Need::Package { program, apt, dnf } => {
            format!("{program} (package {apt}, or {dnf} on RHEL)")
        }
        Need::ModulesFile => format!("{MODULES_FILE}, which loads Longhorn's modules at boot"),
        Need::Module { module } => format!("the {module} module"),
        Need::Iscsid => "iscsid enabled and running".to_owned(),
        Need::MultipathBlacklist => {
            format!("{MULTIPATH_FILE}, which keeps multipathd off Longhorn's disks")
        }
    }
}

/// The ports to allow between the nodes of `distribution`. A K3s cluster that an earlier zeatend
/// installed with Calico keeps it, and a worker cannot tell which network the cluster runs, so
/// the K3s text names both ports.
fn join_ports(distribution: Rancher) -> String {
    let (udp, _) = distribution.vxlan();
    match distribution {
        Rancher::K3s => format!(
            "Allow these between the nodes: TCP 6443 to the server, TCP 10250 and UDP {udp} \
             (flannel VXLAN) between all nodes. Keep UDP {udp} closed to the internet. A cluster \
             that an earlier zeatend installed with Calico uses UDP 4789 (Calico VXLAN) instead."
        ),
        Rancher::Rke2 => format!(
            "Allow these between the nodes: TCP 9345 and 6443 to the servers, TCP 10250 and 9099 \
             and UDP {udp} (Canal VXLAN) between all nodes, and TCP 2379 to 2381 between servers. \
             Keep UDP {udp} closed to the internet."
        ),
    }
}

/// `is` or `are`, for a list of `count` items.
const fn be(count: usize) -> &'static str {
    if count == 1 { "is" } else { "are" }
}

/// How to set the kubelet's reserves on the nodes of `distribution`.
const fn reserves_fix(distribution: Distribution) -> &'static str {
    match distribution {
        Distribution::KubeSolo => {
            "zeatend does not set them on KubeSolo yet (zeabur-oss/zeatend#4)"
        }
        Distribution::K3s | Distribution::Rke2 => {
            "Run `sudo zeatend apply` on each one to set them"
        }
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
    fn plan_findings_in_english() {
        let cases = [
            (
                Finding::MemoryLow {
                    have_mib: 1024,
                    min_mib: 1536,
                    distribution: Distribution::KubeSolo,
                },
                "This host has 1024 MiB of memory, and KubeSolo needs at least 1536 MiB.",
            ),
            (
                Finding::MemoryLow {
                    have_mib: 1800,
                    min_mib: 1900,
                    distribution: Distribution::K3s,
                },
                "This host has 1800 MiB of memory, and K3s needs at least 1900 MiB, so some pods may not start.",
            ),
            (
                Finding::ReleaseUnpinned {
                    distribution: Rancher::Rke2,
                    version: "v1.37.1+rke2r1".to_owned(),
                },
                "RKE2 v1.37.1+rke2r1 is not supported by this zeatend release: its downloads are checked only against the release's own checksum list.",
            ),
            (
                Finding::ServerUnreachable {
                    server: "https://10.0.0.5:9345".to_owned(),
                    port: 9345,
                },
                "Cannot connect to https://10.0.0.5:9345. Check the address and that TCP 9345 is open from this VM.",
            ),
            (
                Finding::ServerFar {
                    median_us: 20_000,
                    samples: 5,
                    port: 9345,
                },
                "The server is 20.0 ms away (median of 5 TCP connects to port 9345); within a region it is usually under 10.0 ms.",
            ),
            (
                Finding::ServerTooFar {
                    median_us: 70_000,
                    samples: 5,
                    port: 6443,
                },
                "The server is 70.0 ms away (median of 5 TCP connects to port 6443). Above 50.0 ms the cluster will be slow and unstable.",
            ),
            (
                Finding::ServerPortsClosed {
                    ports: vec![6443, 10250],
                    host: "10.0.0.5".to_owned(),
                },
                "TCP 6443, 10250 on 10.0.0.5 cannot be reached from this VM; check the firewall or security group.",
            ),
            (
                Finding::JoinPorts {
                    distribution: Rancher::K3s,
                },
                "Allow these between the nodes: TCP 6443 to the server, TCP 10250 and UDP 8472 (flannel VXLAN) between all nodes. Keep UDP 8472 closed to the internet. A cluster that an earlier zeatend installed with Calico uses UDP 4789 (Calico VXLAN) instead.",
            ),
            (
                Finding::JoinPorts {
                    distribution: Rancher::Rke2,
                },
                "Allow these between the nodes: TCP 9345 and 6443 to the servers, TCP 10250 and 9099 and UDP 8472 (Canal VXLAN) between all nodes, and TCP 2379 to 2381 between servers. Keep UDP 8472 closed to the internet.",
            ),
            (
                Finding::UdpUnchecked { port: 8472 },
                "UDP 8472 cannot be checked from here. If it is blocked, pods on different nodes cannot reach each other.",
            ),
            (
                Finding::SameRegion,
                "Put every node at the same hosting provider and in the same region. Different availability zones in one region are fine. Between providers or regions, latency slows etcd, the Kubernetes API and storage, and can make the cluster unstable.",
            ),
            (
                Finding::ServersHa,
                "The control plane survives losing a server only with three or more servers: etcd needs a majority. With two, losing either one stops the cluster, so add a third.",
            ),
            (
                Finding::LocalStorage,
                "Volumes on the default local-path storage class stay on the node that created them: a pod using one runs only on that node, and the data goes with the node.",
            ),
            (
                Finding::TokenUnpinned,
                "The token has no CA hash (K10<hash>::…), so this VM trusts whichever server answers at that address. Use the token with its CA hash, as the server's token file holds it.",
            ),
        ];
        for (f, text) in cases {
            assert_eq!(finding(&f), text, "{f:?}");
        }
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
        use zeatend_plan::finding::{BusyPort, Leftover};
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
                "/ is not on an encrypted disk, so volumes and databases are stored unencrypted. Most VPS plans can't encrypt it afterwards, so for sensitive data choose a plan whose provider encrypts disks.",
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
                "Docker is on this host (/usr/bin/docker, docker.service). It sets the iptables FORWARD policy to DROP, so Pods cannot reach the internet. Stop and remove Docker first.",
            ),
            (
                Finding::ForwardDrop,
                "the iptables FORWARD policy is DROP, so Pods cannot reach the internet. Find what sets it (Docker, ufw) and change it.",
            ),
        ];
        for (f, text) in cases {
            assert_eq!(finding(&f), text, "{f:?}");
        }
    }

    #[test]
    fn doctor_findings_say_what_to_run() {
        use zeatend_plan::finding::Kept;

        let nodes = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
        let cases = [
            (
                Finding::ServiceStopped { unit: "k3s" },
                "k3s is not running; `sudo journalctl -u k3s -n 100` shows why, and `sudo systemctl restart k3s` starts it again",
            ),
            (
                Finding::NodesReady {
                    names: nodes(&["vps-1"]),
                },
                "vps-1 is Ready",
            ),
            (
                Finding::NodesNotReady {
                    names: nodes(&["vps-2", "vps-3"]),
                    command: "sudo kubectl describe node vps-2".to_owned(),
                },
                "vps-2 and vps-3 are not Ready; `sudo kubectl describe node vps-2` shows why",
            ),
            (
                Finding::NodesNone {
                    unit: "rke2-server",
                },
                "the cluster has no nodes; `sudo journalctl -u rke2-server -n 100` shows why the kubelet did not register",
            ),
            (
                Finding::ReservesLow {
                    nodes: vec![
                        Kept {
                            name: "solo".to_owned(),
                            mib: 100,
                        },
                        Kept {
                            name: "bare".to_owned(),
                            mib: 0,
                        },
                    ],
                    needed_mib: 350,
                    distribution: Distribution::KubeSolo,
                },
                "the kubelet keeps only 100 MiB on solo and 0 MiB on bare from pods, less than the 350 MiB the OS needs at idle, so pods can take the memory the OS and the kubelet need (no system-reserved or kube-reserved). zeatend does not set them on KubeSolo yet (zeabur-oss/zeatend#4).",
            ),
            (
                Finding::ReservesLow {
                    nodes: vec![Kept {
                        name: "vps-2".to_owned(),
                        mib: 100,
                    }],
                    needed_mib: 350,
                    distribution: Distribution::K3s,
                },
                "the kubelet keeps only 100 MiB on vps-2 from pods, less than the 350 MiB the OS needs at idle, so pods can take the memory the OS and the kubelet need (no system-reserved or kube-reserved). Run `sudo zeatend apply` on each one to set them.",
            ),
            (
                Finding::SecretsPlain {
                    distribution: Rancher::Rke2,
                },
                "the API server stores Secrets unencrypted; `sudo rke2 secrets-encrypt status` shows the state",
            ),
        ];
        for (f, text) in cases {
            assert_eq!(finding(&f), text, "{f:?}");
        }
    }
}
