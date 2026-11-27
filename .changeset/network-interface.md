---
default: patch
---

# Choose the network interface a node uses

The new setting `network.interface` names the interface a node uses for the cluster network, such as the private interface of a cloud VPC. zeatend takes the node addresses from it, sets the CNI on it (`flannel-iface` on K3s, Canal's interface on RKE2), keeps the default-route addresses as the node's external addresses, and uses it for the join and registry addresses. KubeSolo and Tailscale interfaces reject it.
