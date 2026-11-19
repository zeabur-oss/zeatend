---
default: patch
---

# `zeatend uninstall` removes the routes into the pod network

After the K3s or RKE2 uninstall script runs, `zeatend uninstall` deletes the routes into the pod network `10.42.0.0/16` that are left, such as a blackhole route for the node's block of pod addresses. A later install of any distribution on the same host can then reach its own pods.
