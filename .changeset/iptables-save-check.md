---
default: patch
---

# Stop when iptables-save can't read the host's rules

A new host check runs `iptables-save` and `ip6tables-save`. When either fails, as it does on rules Tailscale writes in its nftables mode, `plan` shows the error and `apply` stops: Calico and kube-proxy read the rules the same way and would not start. The private network page says to run Tailscale in its iptables mode on cluster nodes.
