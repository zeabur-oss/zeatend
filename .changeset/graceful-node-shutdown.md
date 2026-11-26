---
default: patch
---

# K3s and RKE2 nodes stop their pods before the host shuts down

The kubelet now takes part in a reboot or shutdown of the host, from a drop-in in the kubelet's configuration directory. It stops regular pods in the first 20 seconds and critical pods in the last 10, as a deletion stops them: `preStop` hooks run, then each container gets SIGTERM. Before, systemd stopped the containers itself and `preStop` hooks did not run.
