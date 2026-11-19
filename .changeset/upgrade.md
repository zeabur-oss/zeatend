---
default: minor
---

# `apply` upgrades a cluster one minor version at a time

On an installed cluster, `zeatend apply` from a newer zeatend installs the release it pins, or the `version` you set, and restarts the service. Upgrade the servers first, one at a time, then the workers. `apply` stops before it changes anything when the release is older than the installed one, when a server would move Kubernetes more than one minor version, or when a worker would run a newer Kubernetes than its API server, as the Kubernetes version skew policy says. The message names the release to apply first.
