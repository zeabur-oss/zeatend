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
    /// `v1.36.6+rke2r1`.
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

/// The RKE2 release zeatend installs: the `stable` channel on 2026-11-18.
pub const RKE2: Rke2 = Rke2 {
    version: "v1.36.6+rke2r1",
    kubernetes: "1.36",
    amd64: Rke2Files {
        tarball: "e111ac6333ddb2b819d133a413914d4456dd2195a3d694865cb7acda5bf81ae4",
        images_core: "58c153e94944039fc05b8e23de9e8f489df049519e55dd33e5790bf5ae19a217",
        images_canal: "40a09eb8460e27e79dc13247517445c9d94461ef5078ef9e4d7fe1f95aced8b0",
    },
    arm64: Rke2Files {
        tarball: "19988cc4e78a9e04cfdec3977c2cada60975b0c401e44cb7406159e7154482e3",
        images_core: "ff198bb9d9a0a73d273cf185af30abd03dfd92d5b3b7feec47d7aa34fe0f5563",
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
    /// `v1.36.6+k3s1`.
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
    version: "v1.36.6+k3s1",
    kubernetes: "1.36",
    binary: ArchFiles {
        amd64: "66ceb8e67edfa547195135d74783bfadc2ecc04a91059e90ae5f70a3dbc98c15",
        arm64: "49dcecdb4f64f5e92bdc52d204d407fb39729e492f09074488125ee58b6f2895",
    },
    images: ArchFiles {
        amd64: "b3029b1a682b8597587e89f5b1c48493e579a6ea1d0ec6a9aac718a8c356e5be",
        arm64: "881b93edba1b9d91cb0a4ea09ca08298d0f22ec9442ce9a80e697da97945532c",
    },
    install_script: "46177d4c99440b4c0311b67233823a8e8a2fc09693f6c89af1a7161e152fbfad",
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

/// The `KubeSolo` release zeatend installs. Upstream publishes checksums only on the
/// release page, so the digests are copied here as for RKE2.
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
