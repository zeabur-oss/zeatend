---
default: minor
---

# `zeatend node add` joins a new VM to a K3s or RKE2 cluster

Run `sudo zeatend node add` on a server. It prints the configuration and the commands that join a new VM, with a join token: for a worker, a bootstrap token from `k3s token create` or `rke2 token create` that expires after `--ttl` (1h by default); for another RKE2 server (`--role server`), the cluster's server token. The configuration names the cluster's nodes, so the new node takes a name of its own, and the cluster's release when it is not the one this zeatend pins, so the new node installs the same release. On the new VM, `zeatend plan` checks the ports and the round-trip time to the server, and warns above 10 ms and 50 ms, before `zeatend apply` joins it.
