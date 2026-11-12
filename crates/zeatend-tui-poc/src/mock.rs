//! Mock server facts and a fake install timeline. Nothing here touches the machine.

pub(crate) const HOSTNAME: &str = "tpe-edge-01";
pub(crate) const SYSTEM: &str = "Ubuntu 24.04.1 LTS (x86_64)";
pub(crate) const CPU_CORES: i64 = 4;
pub(crate) const MEMORY_GB: i64 = 16;
pub(crate) const DISK_FREE_GB: &str = "58.2";
pub(crate) const DISK_TOTAL_GB: &str = "120";
pub(crate) const PRIVATE_ADDR: &str = "10.0.0.12";

pub(crate) const DOWNLOAD_GB: &str = "1.2";
pub(crate) const DISK_USE_GB: &str = "3.5";
pub(crate) const DOWNLOAD_MB: u64 = 1_210;

pub(crate) const DISTROS: [(&str, &str); 3] = [
    ("KubeSolo", "distro-kubesolo"),
    ("K3s", "distro-k3s"),
    ("RKE2", "distro-rke2"),
];

/// Install steps: message id and how long the fake step takes, in seconds.
pub(crate) const STEPS: [(&str, u32); 5] = [
    ("install-step-check", 4),
    ("install-step-download", 95),
    ("install-step-k3s", 30),
    ("install-step-network", 20),
    ("install-step-ready", 45),
];

pub(crate) fn total_seconds() -> u32 {
    let mut total = 0;
    for (_, seconds) in STEPS {
        total += seconds;
    }
    total
}

/// Where the timeline is at `elapsed` seconds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Progress {
    /// Index of the running step; `STEPS.len()` when done.
    pub(crate) step: usize,
    /// Seconds into the running step.
    pub(crate) into_step: u32,
}

pub(crate) fn progress(elapsed: u32) -> Progress {
    let mut start = 0;
    for (i, (_, seconds)) in STEPS.iter().enumerate() {
        if elapsed < start + seconds {
            return Progress {
                step: i,
                into_step: elapsed - start,
            };
        }
        start += seconds;
    }
    Progress {
        step: STEPS.len(),
        into_step: 0,
    }
}

/// Log lines are tool output and stay in English in every language.
const LOG: [(u32, &str); 18] = [
    (0, "zeatend: checking ports 80, 443, 6443"),
    (
        2,
        "zeatend: swap is on (4.0 GiB), turning it off: swapoff -a",
    ),
    (4, "zeatend: downloading k3s v1.33.4+k3s1 (linux/amd64)"),
    (
        20,
        "zeatend: k3s-airgap-images-amd64.tar.zst 18% (226 MB of 1,210 MB)",
    ),
    (
        45,
        "zeatend: k3s-airgap-images-amd64.tar.zst 47% (571 MB of 1,210 MB)",
    ),
    (
        70,
        "zeatend: k3s-airgap-images-amd64.tar.zst 75% (906 MB of 1,210 MB)",
    ),
    (98, "zeatend: download done, sha256 verified"),
    (
        100,
        "systemd[1]: Started k3s.service - Lightweight Kubernetes.",
    ),
    (
        112,
        "k3s[2210]: Running kube-apiserver --advertise-address=10.0.0.12 --allow-privileged=true --authorization-mode=Node,RBAC",
    ),
    (125, "k3s[2210]: Node tpe-edge-01 is ready"),
    (130, "zeatend: waiting for flannel and kube-router"),
    (
        141,
        "k3s[2210]: flannel: Running backend vxlan on interface eth0",
    ),
    (149, "zeatend: network ready"),
    (150, "zeatend: waiting for node 10-0-0-12 to be Ready"),
    (165, "k3s[2210]: node 10-0-0-12 condition Ready=False"),
    (185, "k3s[2210]: node 10-0-0-12 condition Ready=True"),
    (190, "zeatend: kubeconfig written to ~/.kube/zeatend-k3s.yaml"),
    (194, "zeatend: done"),
];

pub(crate) fn log_until(elapsed: u32) -> Vec<&'static str> {
    let mut lines = Vec::new();
    for (at, line) in LOG {
        if at <= elapsed {
            lines.push(line);
        }
    }
    lines
}

/// Downloaded megabytes during the download step.
pub(crate) fn downloaded_mb(into_step: u32) -> u64 {
    let seconds = u64::from(STEPS[1].1);
    DOWNLOAD_MB * u64::from(into_step).min(seconds) / seconds
}
