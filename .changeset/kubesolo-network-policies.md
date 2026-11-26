---
default: patch
---

# The docs say that KubeSolo enforces no network policies

KubeSolo's built-in bridge network enforces no network policies, so Zeabur OSS can't isolate its projects from each other on a KubeSolo host. The README, the overview and the install guide say so and point to K3s or RKE2 when projects must stay apart.
