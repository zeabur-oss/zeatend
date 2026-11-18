//! Host tuning: the memory the kubelet keeps from pods, its eviction thresholds and the
//! `GOMEMLIMIT` of the distribution's process. The host's [`MemoryTier`] picks the values.
//!
//! The kubelet subtracts `system-reserved`, `kube-reserved` and the hard eviction threshold from
//! the node's memory, and the scheduler places pods only in what is left
//! (<https://kubernetes.io/docs/tasks/administer-cluster/reserve-compute-resources/#node-allocatable>).
//! `GOMEMLIMIT` makes the Go runtime collect garbage and return memory before its heap passes the
//! limit (<https://pkg.go.dev/runtime#hdr-Environment_Variables>); without it, the heap of
//! `k3s server` grows to about twice its live objects.

use serde::Serialize;

use crate::cluster::{Distribution, Rancher};

/// The host's memory class, which picks its [`HostTuning`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryTier {
    /// A host with 4 GiB or less, such as a 2 CPU VM with 2 GiB or 4 GiB.
    Small,
    /// A host with more than 4 GiB.
    Standard,
}

impl MemoryTier {
    /// The most memory a small host reports. A host with 4 GiB reports a little less, because the
    /// kernel and the firmware keep some.
    pub const SMALL_MAX_MIB: u64 = 4096;

    /// The tier of a host that reports `memory_mib`.
    #[must_use]
    pub const fn of(memory_mib: u64) -> Self {
        if memory_mib <= Self::SMALL_MAX_MIB {
            Self::Small
        } else {
            Self::Standard
        }
    }
}

/// What zeatend sets on a host for memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostTuning {
    /// The kubelet's reserves and eviction thresholds; `None` where zeatend sets none.
    pub kubelet: Option<KubeletReserves>,
    /// The `GOMEMLIMIT` of the server process in MiB; `None` where zeatend sets none.
    pub gomemlimit_mib: Option<u64>,
}

/// The memory the kubelet keeps from pods. Every field is in MiB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KubeletReserves {
    /// `system-reserved`: the OS.
    pub system: u64,
    /// `kube-reserved` on a server: the distribution's own processes. A worker sets none.
    pub kube: u64,
    /// Below this much available memory the kubelet evicts at once.
    pub eviction_hard: u64,
    /// Below this much for 30 seconds the kubelet evicts too; `None` turns soft eviction off.
    pub eviction_soft: Option<u64>,
}

/// The OS at idle.
pub const OS_IDLE_MIB: u64 = 350;
/// Twice the kubelet's default of 100Mi, so it evicts before the kernel has to kill.
const EVICTION_HARD_MIB: u64 = 200;
/// Catches memory that grows slowly before it reaches the hard threshold, on a host with room for
/// it.
const EVICTION_SOFT_MIB: u64 = 400;

/// K3s on a host with more than 4 GiB: `k3s server` and containerd at idle, 1530 MiB, and the same
/// figure for `GOMEMLIMIT`, so the reserve and the process agree.
const K3S_STANDARD: HostTuning = HostTuning {
    kubelet: Some(KubeletReserves {
        system: OS_IDLE_MIB,
        kube: 1530,
        eviction_hard: EVICTION_HARD_MIB,
        eviction_soft: Some(EVICTION_SOFT_MIB),
    }),
    gomemlimit_mib: Some(1530),
};

/// K3s on a 2 CPU host with 2 GiB or 4 GiB, as measured. The soft threshold is off: on 2 GiB it
/// would evict pods that fit.
const K3S_SMALL: HostTuning = HostTuning {
    kubelet: Some(KubeletReserves {
        system: OS_IDLE_MIB,
        kube: 100,
        eviction_hard: EVICTION_HARD_MIB,
        eviction_soft: None,
    }),
    gomemlimit_mib: Some(600),
};

/// The `GOMEMLIMIT` of `KubeSolo` on a 1 CPU host with 1.5 GiB, as measured.
const KUBESOLO_GOMEMLIMIT_MIB: u64 = 500;

impl HostTuning {
    /// The tuning of `distribution` on a host of `tier`.
    ///
    /// RKE2 runs its control plane as static pods, whose requests the scheduler counts already,
    /// so it keeps the same reserves on every host and gets no `GOMEMLIMIT`. `KubeSolo` gets only
    /// `GOMEMLIMIT`.
    #[must_use]
    pub const fn new(distribution: Distribution, tier: MemoryTier) -> Self {
        match (distribution, tier) {
            (Distribution::K3s, MemoryTier::Small) => K3S_SMALL,
            (Distribution::K3s, MemoryTier::Standard) => K3S_STANDARD,
            (Distribution::Rke2, _) => Self {
                kubelet: Some(KubeletReserves {
                    system: OS_IDLE_MIB,
                    kube: Rancher::RKE2_KUBE_RESERVED_MIB,
                    eviction_hard: EVICTION_HARD_MIB,
                    eviction_soft: Some(EVICTION_SOFT_MIB),
                }),
                gomemlimit_mib: None,
            },
            (Distribution::KubeSolo, _) => Self {
                kubelet: None,
                gomemlimit_mib: Some(KUBESOLO_GOMEMLIMIT_MIB),
            },
        }
    }

    /// The `Environment=` line of a systemd unit that sets `GOMEMLIMIT`, if there is a limit.
    #[must_use]
    pub fn environment(&self) -> Option<String> {
        self.gomemlimit_mib
            .map(|mib| format!("Environment=GOMEMLIMIT={mib}MiB\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024;

    /// What a host sold with `gib` GiB reports in `/proc/meminfo`: a little less.
    const fn reported(gib: u64) -> u64 {
        gib * GIB - 100
    }

    #[test]
    fn hosts_with_up_to_4_gib_are_small() {
        assert_eq!(MemoryTier::of(reported(2)), MemoryTier::Small);
        assert_eq!(MemoryTier::of(reported(4)), MemoryTier::Small);
        assert_eq!(MemoryTier::of(4 * GIB), MemoryTier::Small);
        assert_eq!(MemoryTier::of(reported(8)), MemoryTier::Standard);
        assert_eq!(MemoryTier::of(reported(16)), MemoryTier::Standard);
    }

    #[test]
    fn k3s_at_2_4_8_and_16_gib() {
        let small = Some(KubeletReserves {
            system: 350,
            kube: 100,
            eviction_hard: 200,
            eviction_soft: None,
        });
        let standard = Some(KubeletReserves {
            system: 350,
            kube: 1530,
            eviction_hard: 200,
            eviction_soft: Some(400),
        });
        for (gib, kubelet, limit) in [
            (2, small, 600),
            (4, small, 600),
            (8, standard, 1530),
            (16, standard, 1530),
        ] {
            let tuning = HostTuning::new(Distribution::K3s, MemoryTier::of(reported(gib)));
            assert_eq!(tuning.kubelet, kubelet, "{gib} GiB");
            assert_eq!(tuning.gomemlimit_mib, Some(limit), "{gib} GiB");
        }
    }

    #[test]
    fn rke2_keeps_its_reserves_on_every_tier_and_kubesolo_gets_only_gomemlimit() {
        assert_eq!(
            HostTuning::new(Distribution::Rke2, MemoryTier::Small),
            HostTuning::new(Distribution::Rke2, MemoryTier::Standard)
        );
        assert_eq!(
            HostTuning::new(Distribution::Rke2, MemoryTier::Small).gomemlimit_mib,
            None
        );
        for tier in [MemoryTier::Small, MemoryTier::Standard] {
            let solo = HostTuning::new(Distribution::KubeSolo, tier);
            assert_eq!(solo.kubelet, None);
            assert_eq!(
                solo.environment().as_deref(),
                Some("Environment=GOMEMLIMIT=500MiB\n")
            );
        }
    }
}
