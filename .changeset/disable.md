---
default: minor
---

# K3s and RKE2 keep their packaged components unless `disable` turns them off

K3s now runs its packaged Traefik, and RKE2 its default ingress controller, as upstream ships them. `disable: [traefik, metrics-server]` in the configuration turns them off for a cluster that runs its own; `local-storage` turns off K3s's local-path storage class. `apply` waits until the server has removed the components it turns off, and when you remove a component from the list, the next `apply` turns it on again. A server that joins takes the same list. On a cluster that zeabur-installer installed, `disable` starts as the list the installer turned off. On K3s, a network policy lets traffic reach Traefik's entry points, as RKE2's `cis` profile does.
