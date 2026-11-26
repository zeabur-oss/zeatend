---
default: patch
---

# KubeSolo nodes take host.nodeName

A KubeSolo node now takes `host.nodeName` and the node-name field like a K3s or RKE2 node, and a host name made from an address becomes `node-` and six hex digits. KubeSolo v1.3.1 uses the configured name in kube-proxy. A KubeSolo node that is already installed keeps its name.
