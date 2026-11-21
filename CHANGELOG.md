# Changelog

All notable changes to Zeabur Zeatend. Versions follow [Semantic Versioning](https://semver.org/); each section comes from the change files in `.changeset/` (CONTRIBUTING.md, "Releases").

## 0.1.1 (2026-11-21)

### Features

#### `apiServer.authenticationConfig` sets the API server's authentication configuration

`apiServer.authenticationConfig` takes a Kubernetes `AuthenticationConfiguration`, such as the OpenID Connect issuers whose tokens the API server accepts. On a K3s, RKE2 or KubeSolo server, `apply` writes it to a file and gives the file to the API server, and writes it again when it changes. On K3s and RKE2, anonymous requests stay off unless the configuration says otherwise.

#### `storage.longhorn` prepares nodes for Longhorn

With `storage: {longhorn: true}`, `apply` installs the iSCSI initiator and the NFS client with apt or dnf, loads `iscsi_tcp` and `dm_crypt` at boot, enables `iscsid` and keeps multipathd off Longhorn's disks. `plan` lists what a node lacks, `zeatend doctor` reports it, and `zeatend node add` gives a new node the same `storage` setting. K3s and RKE2 only.

## 0.1.0 (2026-11-20)

### Features

#### `apply --plan` applies the plan you reviewed

`zeatend plan --json > plan.json` writes the plan, and `zeatend apply --plan plan.json` applies it. The plan holds the hash of each file it writes, the installed release and the install record as they were. When one of them changed since, or the plan made now writes something else, `apply` changes nothing and asks you to make and review the plan again. A file, release or record that the plan itself already left is fine, so a second `apply --plan` after an interruption goes on. The install screens pass the plan you confirmed to the install unit in the same way. `apply` without `--plan` plans and applies as before.

#### `apply` goes on after a disconnect

On a host with systemd, `zeatend apply` runs the install in the transient unit `zeatend-install-<id>.service` and shows its progress from the journal. When the SSH session ends, the install goes on; run `sudo zeatend apply` again to follow it. `apply --foreground` runs the install in the same process, as before, and `apply --json` prints the progress as JSON lines.

#### K3s and RKE2 keep their packaged components unless `disable` turns them off

K3s now runs its packaged Traefik, and RKE2 its default ingress controller, as upstream ships them. `disable: [traefik, metrics-server]` in the configuration turns them off for a cluster that runs its own; `local-storage` turns off K3s's local-path storage class. `apply` waits until the server has removed the components it turns off, and when you remove a component from the list, the next `apply` turns it on again. A server that joins takes the same list. On a cluster that zeabur-installer installed, `disable` starts as the list the installer turned off. On K3s, a network policy lets traffic reach Traefik's entry points, as RKE2's `cis` profile does.

#### `zeatend doctor` checks the host and the cluster

`sudo zeatend doctor` runs the host checks of `apply` and then checks the cluster on the host: the K3s, RKE2 or KubeSolo service, the API server, the install record, that the nodes are Ready, that each kubelet keeps at least the memory the OS needs at idle from pods, and, on a K3s or RKE2 server, that Secrets are stored encrypted. Each line that is not OK names the command to run. It exits 0 when all is well, 1 for a warning, 2 for something critical and 3 when the checks cannot run, as monitoring plugins do. A worker checks only its own node; run doctor on a server to check every node.

#### zeatend takes over a cluster that zeabur-installer installed

On a host where zeabur-installer installed K3s, RKE2 or KubeSolo, `zeatend plan`, `apply`, `doctor`, `node add` and `uninstall` work without a configuration of their own. With no `/etc/zeatend/config.yaml`, zeatend reads the distribution, the release, the host settings and the join section from `/etc/zeabur-installer/config.yaml`, and reads the installer's install record. `apply` keeps the components that the installer turned off, such as Traefik, local-storage and metrics-server, off. `uninstall` also removes the installer's state, kubeconfig and join token. What the installer put on the cluster, such as Zeabur, stays as it is.

`doctor` now names what wrote the install record, and `apply` no longer reinstalls KubeSolo on every run, because it read the version only from standard output, where KubeSolo does not print it.

#### `version` picks the K3s or RKE2 release

The configuration names a K3s or RKE2 release other than the pinned one with one key, `version`, such as `version: v1.37.1+k3s1` or `version: v1.37.1+rke2r1`. It must be a release tag of the configured distribution. A configuration that zeabur-installer wrote keeps working, as zeatend reads the installer's key for the installer's distribution.

#### Kubernetes 1.37 on K3s and RKE2

New K3s clusters install `v1.37.2+k3s1` and new RKE2 clusters install `v1.37.2+rke2r1`. KubeSolo has no 1.37 release yet, so KubeSolo hosts stay on `v1.3.0` with Kubernetes 1.36.6.

#### `zeatend node add` joins a new VM to a K3s or RKE2 cluster

Run `sudo zeatend node add` on a server. It prints the configuration and the commands that join a new VM, with a join token: for a worker, a bootstrap token from `k3s token create` or `rke2 token create` that expires after `--ttl` (1h by default); for another RKE2 server (`--role server`), the cluster's server token. The configuration names the cluster's nodes, so the new node takes a name of its own, and the cluster's release when it is not the one this zeatend pins, so the new node installs the same release. On the new VM, `zeatend plan` checks the ports and the round-trip time to the server, and warns above 10 ms and 50 ms, before `zeatend apply` joins it.

#### Host checks in 繁體中文 and 简体中文

`zeatend plan` and `zeatend apply` show the results of the host checks in English, 繁體中文 or 简体中文. The language comes from `LC_ALL`, `LC_MESSAGES` or `LANG`, and a `LANGUAGE` list such as `zh_TW:en` wins over them as in GNU gettext, unless the locale is `C`. Chinese needs a UTF-8 locale and a terminal other than the Linux console (`TERM=linux`); otherwise the text stays in English. The install unit of `apply` gets the session's locale and `TERM`, so it shows the same language.

In `plan --json` of a joining node, a port's `purpose` is now a key such as `kubernetesApi` instead of an English phrase.

#### Install screens in English, 繁體中文 and 简体中文

`sudo zeatend` with no command opens four screens that lead to an install: Server shows the host, the choice of KubeSolo, K3s or RKE2 and the host checks of that choice; Access asks for the address and DNS name of the API server and the node name; Review shows the release and the node name from the plan; Install shows the progress. The choices go into the configuration file, which keeps its other settings, and the install runs in a `zeatend-install-<id>.service` unit, as `zeatend apply` does, in the language of the screens. It goes on when the SSH session ends, and `sudo zeatend` opens the Install screen of a running install again.

The screens are in English, 繁體中文 or 简体中文 from the locale, and F2 changes the language. A terminal that cannot show Chinese gets English with ASCII borders. Without a terminal, `zeatend` says to run `zeatend plan` and `zeatend apply`.

#### `zeatend uninstall` removes the cluster from the host

`zeatend uninstall` runs `k3s-uninstall.sh`, `k3s-agent-uninstall.sh` or `rke2-uninstall.sh`, or on KubeSolo stops it and removes its service, binary, configuration and data directory, and then removes the files zeatend wrote: its state directories with the install record and downloads, the manifests, configuration files, join token and `sysctl` file. It shows what it removes and what stays and asks first; `--yes` skips the question. Volumes in the local-path storage of K3s and KubeSolo, `/var/lib/zeabur/storage` and `/var/lib/longhorn` stay unless `--delete-volumes` is given. A node that joined a cluster stays in the cluster's node list, and the output names the `kubectl delete node` to run on a server. On a clean host it says there is nothing to remove.

#### `apply` upgrades a cluster one minor version at a time

On an installed cluster, `zeatend apply` from a newer zeatend installs the release it pins, or the `version` you set, and restarts the service. Upgrade the servers first, one at a time, then the workers. `apply` stops before it changes anything when the release is older than the installed one, when a server would move Kubernetes more than one minor version, or when a worker would run a newer Kubernetes than its API server, as the Kubernetes version skew policy says. The message names the release to apply first.

### Fixes

#### The install screens start the install with the plan you reviewed

The Review screen keeps the plan it shows, with what it found on the host, and the install unit applies that plan. When the host changed after the Review screen, `apply --plan` changes nothing and says so, instead of the screens comparing the release and node name only.

#### `zeatend uninstall` removes the routes into the pod network

After the K3s or RKE2 uninstall script runs, `zeatend uninstall` deletes the routes into the pod network `10.42.0.0/16` that are left, such as a blackhole route for the node's block of pod addresses. A later install of any distribution on the same host can then reach its own pods.
