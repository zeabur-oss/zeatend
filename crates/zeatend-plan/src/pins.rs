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
    /// `v1.36.5+rke2r1`.
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

/// The RKE2 release zeatend installs: the `stable` channel on 2026-10-21.
pub const RKE2: Rke2 = Rke2 {
    version: "v1.36.5+rke2r1",
    kubernetes: "1.36",
    amd64: Rke2Files {
        tarball: "4fdbe8a8a32517fbcb6e80c71ca393b96828fbb87685102d9058b51527f9278d",
        images_core: "adc5fe6ca25a3ebc8866e3dad3ff3690d38c8af4a5d312e71bf954e6e3213203",
        images_canal: "294426213e146205f37037d2ab7bc629225d53a785e3f89d5a5f7405d2f0c98d",
    },
    arm64: Rke2Files {
        tarball: "244454bed11d9525452e10a2e5055da5d5fc6b61928ca3fae778bc4254ce467f",
        images_core: "5030039486973fc679224bf297116df99972cb41ef4dc3387334685c6e1f170f",
        images_canal: "626bb4dc147d51b909aa73fbdaaad4104001ec6a92680be04ed2e974fb3a145c",
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
    /// `v1.36.5+k3s1`.
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

/// The K3s release zeatend installs, published 2026-09-30: the Kubernetes release of
/// [`RKE2`].
pub const K3S: K3s = K3s {
    version: "v1.36.5+k3s1",
    kubernetes: "1.36",
    binary: ArchFiles {
        amd64: "d73847bcd3c5fccef0115b372e2f9a91f3032dc84bbf71518a4617565294d313",
        arm64: "135e34cb9e8a1cfae3cb55577789e93501efe7edc080a8b2630458a85b07721e",
    },
    images: ArchFiles {
        amd64: "7d939471907df2d4e537a9c308bf94173e453fff4a2e9e77e9ae92a64be3007f",
        arm64: "aeb7b47b0f0cb1356a5f0bc1c11b7c499deceeeafcd411963064e02bfc77da84",
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
    /// `v1.2.3`.
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
    version: "v1.2.3",
    kubernetes: "1.35",
    offline: ArchFiles {
        amd64: "87dfdb609cc7700dfb4cdd7dcb0750f6fc9c1898710fb9dedc0996bc43d522ef",
        arm64: "c5104f127b2e437ed457c8eba0d6c73e07369526fbbea779bfc24e8745f82f19",
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
    /// `v1.35.7`.
    pub version: &'static str,
    /// SHA-256 of the binary.
    pub binary: ArchFiles,
}

/// The kubectl zeatend uses with `KubeSolo`.
pub const KUBECTL: Kubectl = Kubectl {
    version: "v1.35.7",
    binary: ArchFiles {
        amd64: "12e97f9d23a9f6cbb87b89becd6bd291e1a858a3379a4e11e2c822c4c1530052",
        arm64: "5103d45b8881434d417694057b8ccb5ae79fd310a5c5c13e403c5e62e15909be",
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
