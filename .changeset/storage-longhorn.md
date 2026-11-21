---
default: minor
---

# `storage.longhorn` prepares nodes for Longhorn

With `storage: {longhorn: true}`, `apply` installs the iSCSI initiator and the NFS client with apt or dnf, loads `iscsi_tcp` and `dm_crypt` at boot, enables `iscsid` and keeps multipathd off Longhorn's disks. `plan` lists what a node lacks, `zeatend doctor` reports it, and `zeatend node add` turns it on for each new node and tells you to turn it on on the server. K3s and RKE2 only.
