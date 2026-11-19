---
default: major
---

# `version` replaces `k3sVersion` and `rke2Version`

The configuration names a K3s or RKE2 release other than the pinned one with one key, `version`, such as `version: v1.37.1+k3s1` or `version: v1.37.1+rke2r1`. It must be a release tag of the configured distribution. `k3sVersion` and `rke2Version` are no longer read: rename the key in `/etc/zeatend/config.yaml`. A configuration that zeabur-installer wrote keeps working, as zeatend reads the installer's key for the installer's distribution.
