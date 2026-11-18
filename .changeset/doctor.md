---
default: minor
---

# `zeatend doctor` checks the host and the cluster

`sudo zeatend doctor` runs the host checks of `apply` and then checks the cluster on the host: the K3s, RKE2 or KubeSolo service, the API server, the install record, that the nodes are Ready, that each kubelet keeps at least the memory the OS needs at idle from pods, and, on a K3s or RKE2 server, that Secrets are stored encrypted. Each line that is not OK names the command to run. It exits 0 when all is well, 1 for a warning, 2 for something critical and 3 when the checks cannot run, as monitoring plugins do. A worker checks only its own node; run doctor on a server to check every node.
