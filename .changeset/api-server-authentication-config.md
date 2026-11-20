---
default: minor
---

# `apiServer.authenticationConfig` sets the API server's authentication configuration

`apiServer.authenticationConfig` takes a Kubernetes `AuthenticationConfiguration`, such as the OpenID Connect issuers whose tokens the API server accepts. On a K3s, RKE2 or KubeSolo server, `apply` writes it to a file and gives the file to the API server, and writes it again when it changes. On K3s and RKE2, anonymous requests stay off unless the configuration says otherwise.
