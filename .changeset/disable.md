---
default: minor
---

# K3s and RKE2 keep their packaged components unless `disable` turns them off

K3s now runs its packaged Traefik, and RKE2 its default ingress controller, as upstream ships them. `disable: [traefik, metrics-server]` in the configuration turns them off for a cluster that runs its own; `local-storage` turns off K3s's local-path storage class. A server that joins takes the same `disable` list. A cluster that an earlier install set up keeps off what it turned off, including RKE2's `ingress-controller: none`.
