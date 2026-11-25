---
default: patch
---

# The plan warns when the node would be named `localhost`

`localhost` does not say which machine a warning is about. On K3s and RKE2 the note points to `host.nodeName` and the Node name field on the Access screen. On KubeSolo, which names the node after the host name, it says to change the host name before the install.
