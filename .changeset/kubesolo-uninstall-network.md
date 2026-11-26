---
default: patch
---

# `uninstall` removes KubeSolo's pod network

On KubeSolo, `uninstall` reads the network name and bridge from KubeSolo's CNI configuration before it removes it. Then it deletes the network namespaces of the pods, the bridge and its route, and the addresses and results in `/var/lib/cni`. `/var/lib/kubelet` goes too. The bridge and `/var/lib/kubelet` stay when K3s or RKE2 is on the host. Before, they stayed, and a later K3s install started with KubeSolo's `cni0` bridge and its route to `10.42.0.0/16`.
