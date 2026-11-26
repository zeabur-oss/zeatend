---
default: patch
---

# KubeSolo v1.3.1

Zeatend installs KubeSolo v1.3.1, which still ships Kubernetes 1.36.6. It fixes an idle node that used about 1.3 CPU cores since v1.3.0, and aggregated APIs such as metrics-server that rejected the API server a year after the install ([release notes](https://github.com/portainer/kubesolo/releases/tag/v1.3.1)). `zeatend apply` on a KubeSolo v1.3.0 host upgrades it.
