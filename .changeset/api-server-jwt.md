---
default: minor
---

# The API server trusts OpenID Connect issuers from `apiServer.jwt`

`apiServer.jwt` lists issuers whose tokens the API server accepts, each with its audiences and a username prefix, and optionally the username claim, a groups claim with its prefix and a CA certificate. On a K3s, RKE2 or KubeSolo server, `apply` writes them as the API server's authentication configuration, and writes it again when the list changes.
