---
default: patch
---

# Restart Canal when its interface changes

Canal's pods read `network.interface` only when they start, and the RKE2 chart doesn't restart them when it changes. A first RKE2 server could keep flannel on the default route, and a changed or removed `network.interface` stayed on the old interface. `apply` on an RKE2 server now waits until Canal's `ConfigMap` has the interface, then restarts Canal on each node, one at a time, when its pods started with another one.
