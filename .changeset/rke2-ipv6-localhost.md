---
default: patch
---

# Check that localhost is ::1 on IPv6-only RKE2 nodes

On an RKE2 node whose addresses are all IPv6, RKE2 runs etcd, the controller manager, the scheduler and kube-proxy on `::1`, and their health checks call `localhost`. Ubuntu's `/etc/hosts` maps `localhost` to `127.0.0.1` only, so the checks failed and the kubelet restarted these pods. The host checks now stop `apply` until `/etc/hosts` maps `localhost` to `::1`.
