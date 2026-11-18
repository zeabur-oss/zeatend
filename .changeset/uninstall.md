---
default: minor
---

# `zeatend uninstall` removes the cluster from the host

`zeatend uninstall` runs `k3s-uninstall.sh`, `k3s-agent-uninstall.sh` or `rke2-uninstall.sh`, or on KubeSolo stops it and removes its service, binary, configuration and data directory, and then removes the files zeatend wrote: its state directories with the install record and downloads, the manifests, configuration files, join token and `sysctl` file. It shows what it removes and what stays and asks first; `--yes` skips the question. Volumes in the local-path storage of K3s and KubeSolo, `/var/lib/zeabur/storage` and `/var/lib/longhorn` stay unless `--delete-volumes` is given. A node that joined a cluster stays in the cluster's node list, and the output names the `kubectl delete node` to run on a server. On a clean host it says there is nothing to remove.
