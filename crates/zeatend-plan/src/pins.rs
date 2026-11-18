//! What this zeatend release installs, pinned by digest.
//!
//! K3s and RKE2 releases carry no signatures or provenance; the only check upstream offers is a
//! `sha256sum-<arch>.txt` served from the same release page, which does not help when that page is
//! the thing tampered with. The digests are therefore copied into zeatend, and zeatend's own
//! signature is where trust starts. Another RKE2 version can still be chosen, but
//! the plan marks it unsupported and checks it only against the release's list.

/// An RKE2 release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rke2 {
    /// `v1.37.2+rke2r1`.
    pub version: &'static str,
    /// The Kubernetes minor it ships.
    pub kubernetes: &'static str,
    /// SHA-256 per architecture.
    pub amd64: Rke2Files,
    /// SHA-256 per architecture.
    pub arm64: Rke2Files,
}

/// The files of one architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rke2Files {
    /// `rke2.linux-<arch>.tar.gz`.
    pub tarball: &'static str,
    /// `rke2-images-core.linux-<arch>.tar.zst`.
    pub images_core: &'static str,
    /// `rke2-images-canal.linux-<arch>.tar.zst`.
    pub images_canal: &'static str,
}

/// The RKE2 release zeatend installs, published 2026-11-05: the newest Kubernetes 1.37 release
/// past the seven-day minimum release age.
pub const RKE2: Rke2 = Rke2 {
    version: "v1.37.2+rke2r1",
    kubernetes: "1.37",
    amd64: Rke2Files {
        tarball: "0568f48d772c7feff7056062225d5ba82fb3f73ea7acaea63862fe601dae6d92",
        images_core: "e517534d66ec29cd67eca8e16b4eb6f9838378e50a55569ecac2c420101aabd6",
        images_canal: "18728448092a4578358910a818ad10f9eb3c1f2e45abd1cdc301633c2d011ee6",
    },
    arm64: Rke2Files {
        tarball: "849bf989b079e330f73d142259b1be4d2fd487927431555e7544cf1687627822",
        images_core: "fd119f9a5ffffd64e7fa9384fc256e17979bf33da5a02a0b72b7438e322b62fe",
        images_canal: "deb2e4bce4613633897bf4fd41c0941bc73dfee195f04aad96d138a1d186c537",
    },
};

impl Rke2 {
    /// The pinned files of an architecture (`amd64`, `arm64`).
    #[must_use]
    pub fn files(&self, arch: &str) -> Option<Rke2Files> {
        match arch {
            "amd64" => Some(self.amd64),
            "arm64" => Some(self.arm64),
            _ => None,
        }
    }
}

/// The GitHub release URL of an RKE2 file.
#[must_use]
pub fn rke2_url(version: &str, file: &str) -> String {
    format!(
        "https://github.com/rancher/rke2/releases/download/{}/{file}",
        version.replace('+', "%2B")
    )
}

/// A K3s release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct K3s {
    /// `v1.37.2+k3s1`.
    pub version: &'static str,
    /// The Kubernetes minor it ships.
    pub kubernetes: &'static str,
    /// SHA-256 of the binary, `k3s` (amd64) or `k3s-arm64`.
    pub binary: ArchFiles,
    /// SHA-256 of `k3s-airgap-images-<arch>.tar.zst`.
    pub images: ArchFiles,
    /// SHA-256 of `install.sh` at the release tag. It writes the systemd unit and the uninstall
    /// script; zeatend gives it the checked binary.
    pub install_script: &'static str,
}

/// The K3s release zeatend installs, published 2026-11-03: the Kubernetes release of
/// [`RKE2`].
pub const K3S: K3s = K3s {
    version: "v1.37.2+k3s1",
    kubernetes: "1.37",
    binary: ArchFiles {
        amd64: "265b3ee6a3198ef14033c63ef46cbb32370d83795199fb6d1b01f21575eddbda",
        arm64: "a779ccf1e5e2e33a62f5a29c2f1c0bbf99431e100579d4c10f0aa0742a6aa84b",
    },
    images: ArchFiles {
        amd64: "35a5af1e69b57b8b627a4f6b1c29ea104209b661067b8c97fd40dc3f860bfcd2",
        arm64: "ca384f1a77aa83024bd82ad26fa4774971adb038e656a3b5b9af926be445a442",
    },
    install_script: "ed01f89fd977bf20ac1516bbebf8370bf3ddbaa55dac8aba610956a4c78cc00b",
};

impl K3s {
    /// The binary's file name on the release page.
    #[must_use]
    pub fn binary_file(arch: &str) -> String {
        if arch == "amd64" {
            "k3s".to_owned()
        } else {
            format!("k3s-{arch}")
        }
    }
}

/// The GitHub release URL of a K3s file.
#[must_use]
pub fn k3s_url(version: &str, file: &str) -> String {
    format!(
        "https://github.com/k3s-io/k3s/releases/download/{}/{file}",
        version.replace('+', "%2B")
    )
}

/// The URL of K3s's `install.sh` at a release tag.
#[must_use]
pub fn k3s_install_script_url(version: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/k3s-io/k3s/{}/install.sh",
        version.replace('+', "%2B")
    )
}

/// Calico, the network of a K3s cluster: the release RKE2's Canal carries, with VXLAN.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Calico {
    /// `v3.32.2`.
    pub version: &'static str,
    /// SHA-256 of `manifests/calico-vxlan.yaml` at the release tag.
    pub manifest_sha256: &'static str,
    /// The images the manifest names by tag, and the digest each one is pinned to.
    pub images: &'static [(&'static str, &'static str)],
}

/// The Calico release of a K3s cluster.
pub const CALICO: Calico = Calico {
    version: "v3.32.2",
    manifest_sha256: "5aa751a909fdf761b58e97953ec92173018115b3ae405f4c9dfc475f54bf737f",
    images: &[
        (
            "quay.io/calico/cni:v3.32.2",
            "sha256:0ef740bc587f25565905adf1d1f61a7faff0d571c449c6bdd789feed743d3ef7",
        ),
        (
            "quay.io/calico/node:v3.32.2",
            "sha256:99b03fe91e8bfbcb153ae65ef4b701b24ce541ffdd74ff314eb041096008f7fd",
        ),
        (
            "quay.io/calico/kube-controllers:v3.32.2",
            "sha256:7870b67ebb13fabc3005252b44fe6e78b21635649bd3072b80afa1684b6565d0",
        ),
    ],
};

impl Calico {
    /// The manifest.
    #[must_use]
    pub fn manifest(&self) -> Asset {
        Asset {
            url: format!(
                "https://raw.githubusercontent.com/projectcalico/calico/{}/manifests/calico-vxlan.yaml",
                self.version
            ),
            file: "calico-vxlan.yaml".to_owned(),
            sha256: self.manifest_sha256,
        }
    }
}

/// A file of a GitHub release and its SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// The file name.
    pub file: String,
    /// The download URL.
    pub url: String,
    /// The expected SHA-256.
    pub sha256: &'static str,
}

/// A file of one architecture and its SHA-256.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchFiles {
    /// SHA-256 of the amd64 file.
    pub amd64: &'static str,
    /// SHA-256 of the arm64 file.
    pub arm64: &'static str,
}

impl ArchFiles {
    /// The SHA-256 of an architecture (`amd64`, `arm64`).
    #[must_use]
    pub const fn get(&self, arch: &str) -> Option<&'static str> {
        match arch.as_bytes() {
            b"amd64" => Some(self.amd64),
            b"arm64" => Some(self.arm64),
            _ => None,
        }
    }
}

/// A `KubeSolo` release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KubeSolo {
    /// `v1.3.0`.
    pub version: &'static str,
    /// The Kubernetes minor it ships.
    pub kubernetes: &'static str,
    /// The offline archive, which carries the images `KubeSolo` runs (pause, `CoreDNS`,
    /// local-path-provisioner). The online one pulls them by tag.
    pub offline: ArchFiles,
}

/// The `KubeSolo` release zeatend installs.
///
/// It ships Kubernetes 1.36.6: `KubeSolo` has no 1.37 release yet, so a `KubeSolo` host runs one
/// minor behind K3s and RKE2. Upstream publishes checksums only on the release page, so the
/// digests are copied here as for RKE2.
pub const KUBESOLO: KubeSolo = KubeSolo {
    version: "v1.3.0",
    kubernetes: "1.36",
    offline: ArchFiles {
        amd64: "32956d77c00990676dd9b28b99bbc02f6fb8abdcd410d294aae93a4d8e2c6786",
        arm64: "3f401b96d5d88f913c3ade2538aa471a73feba1ec5eff484c6d2ed6d930553dd",
    },
};

impl KubeSolo {
    /// The offline archive of an architecture.
    #[must_use]
    pub fn archive(&self, arch: &str) -> Option<Asset> {
        let sha256 = self.offline.get(arch)?;
        let file = format!("kubesolo-{}-linux-{arch}-offline.tar.gz", self.version);
        Some(Asset {
            url: format!(
                "https://github.com/portainer/kubesolo/releases/download/{}/{file}",
                self.version
            ),
            file,
            sha256,
        })
    }
}

/// kubectl of the Kubernetes release `KubeSolo` ships; `KubeSolo` has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kubectl {
    /// `v1.36.6`.
    pub version: &'static str,
    /// SHA-256 of the binary.
    pub binary: ArchFiles,
}

/// The kubectl zeatend uses with `KubeSolo`.
pub const KUBECTL: Kubectl = Kubectl {
    version: "v1.36.6",
    binary: ArchFiles {
        amd64: "29818b6729dc9262b692307470d0558d027725e738be5dd47c760261868a2634",
        arm64: "35507ef408c3d38849e5d3203e18f8e861db822aa664101a9ced0869edd06046",
    },
};

impl Kubectl {
    /// The binary of an architecture.
    #[must_use]
    pub fn binary(&self, arch: &str) -> Option<Asset> {
        let sha256 = self.binary.get(arch)?;
        Some(Asset {
            url: format!(
                "https://dl.k8s.io/release/{}/bin/linux/{arch}/kubectl",
                self.version
            ),
            file: "kubectl".to_owned(),
            sha256,
        })
    }
}
