---
default: minor
---

# zeatend takes over a cluster that zeabur-installer installed

On a host where zeabur-installer installed K3s, RKE2 or KubeSolo, `zeatend plan`, `apply`, `doctor`, `node add` and `uninstall` work without a configuration of their own. With no `/etc/zeatend/config.yaml`, zeatend reads the distribution, the release, the host settings and the join section from `/etc/zeabur-installer/config.yaml`, and reads the installer's install record. `apply` keeps the components that the installer turned off, such as Traefik, local-storage and metrics-server, off. `uninstall` also removes the installer's state, kubeconfig and join token. What the installer put on the cluster, such as Zeabur, stays as it is.

`doctor` now names what wrote the install record, and `apply` no longer reinstalls KubeSolo on every run, because it read the version only from standard output, where KubeSolo does not print it.
