---
default: minor
---

# Registry mirrors and an HTTPS proxy for servers that cannot reach GitHub

`registryMirrors` names the mirrors containerd pulls each registry's images from, `registryHosts` gives a mirror or registry a CA and credentials, and `proxy` sends zeatend's downloads and the node's pulls through an HTTPS proxy. `apply` writes them as `registries.yaml` on K3s and RKE2 and as containerd `hosts.toml` files on KubeSolo, and removes them again when they leave the configuration. containerd tries the registry itself after the mirrors. `zeatend doctor` checks that each mirror answers and, before an install, that the release downloads answer.
