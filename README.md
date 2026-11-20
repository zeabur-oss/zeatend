<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/cover-dark.avif">
  <img alt="Zeatend: turn one Linux server into a cluster that’s ready to use. zeatend apply turns a Linux server into a running Kubernetes cluster." src="docs/images/cover-light.avif">
</picture>

English | [繁體中文](README.zh-TW.md) | [简体中文](README.zh-CN.md)

# Zeabur Zeatend

A Kubernetes cluster manager in the spirit of [k3sup](https://github.com/alexellis/k3sup). It
sets up and manages K3s, RKE2 and KubeSolo clusters on hosts over SSH, with a terminal UI.

Zeatend is made by the Zeabur project and is in early development.

## Usage

Today `zeatend` runs on the Linux host it installs, as root. `sudo zeatend` with no command opens
the install screens in English, 繁體中文 or 简体中文:

<img alt="The Server screen: the host's facts, the choice of K3s, RKE2 or KubeSolo, and the host checks." src="docs/screens/80x24/en-server.svg" width="744">

They show the host and its checks, ask for the distribution, the address and DNS name of the API
server and the node name, and show the plan. The choices go into `/etc/zeatend/config.yaml`, and
the install runs as with `zeatend apply`. Every screen, at 80x24 and 120x36, is in
[docs/screens](docs/screens). The language comes from the locale, as for the host checks, and F2
changes it. A terminal that cannot show Chinese, such as the Linux console, gets English with
ASCII borders.

Without the screens, `zeatend` reads one configuration file,
`/etc/zeatend/config.yaml` by default, and installs K3s, RKE2 or KubeSolo hardened to the CIS
benchmark:

```yaml
distribution: k3s # k3s (default), rke2 or kubesolo
host:
  tlsSan: [k8s.example.com]
```

K3s (`v1.37.2+k3s1`) and RKE2 (`v1.37.2+rke2r1`) install Kubernetes 1.37. KubeSolo `v1.3.0` installs
Kubernetes 1.36.6, because KubeSolo has no 1.37 release yet.

K3s and RKE2 deploy their packaged components as upstream ships them, Traefik among them. To run
your own instead, turn them off with `disable`, such as `disable: [traefik, metrics-server]`.
`local-storage` turns off K3s's local-path storage class. KubeSolo packages none of these. On a
cluster that zeabur-installer installed, `disable` starts as the list the installer turned off.

`apiServer.jwt` makes the API server accept tokens from OpenID Connect issuers. Zeatend writes them
to the API server's
[authentication configuration](https://kubernetes.io/docs/reference/access-authn-authz/authentication/#using-authentication-configuration)
on a server:

```yaml
apiServer:
  jwt:
    - issuer: https://identity.example.com
      audiences: [kubernetes]
      usernameClaim: preferred_username # sub by default
      usernamePrefix: "zeabur:"
      groupsClaim: groups # optional, with groupsPrefix
      groupsPrefix: "zeabur:"
```

`certificateAuthority` takes a PEM certificate when the issuer's certificate is not from a public
authority. A user or group with the `system:` prefix is refused. A changed list takes effect on
the next `apply`.

- `zeatend plan` shows what `apply` would install and runs the host checks; `--json` prints the
  plan as JSON.
- `zeatend apply` installs, or brings an install in line with the configuration. It is safe to
  run again. `apply --plan <file>` applies the plan that `plan --json` wrote to the file, and
  changes nothing when a file it writes, the installed release or the install record changed
  since.
- `zeatend uninstall` removes K3s, RKE2 or KubeSolo and the files zeatend wrote. It shows what it
  removes and keeps, and asks first (`--yes` skips the question). Volumes stay unless you add
  `--delete-volumes`.

On a host with systemd, `apply` runs the install in the transient unit
`zeatend-install-<id>.service` and shows its progress. When the SSH session ends, the install goes
on. Run `sudo zeatend apply` again to follow it: while an install unit runs, `apply` shows that
unit's progress from the journal and starts nothing new. `journalctl -u 'zeatend-install-*'` has the
full output. `apply --foreground` runs the install in the same process instead, and `apply --json`
prints the progress as JSON lines.

A configuration with a `join` section joins this host to an existing K3s or RKE2 cluster:

```yaml
distribution: k3s
join:
  server: https://10.0.0.5:6443 # port 9345 on RKE2
  role: worker # or server, on RKE2 only
  tokenFile: /etc/zeatend/join-token # mode 0600
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Report security issues as described in
[SECURITY.md](SECURITY.md).

## License

[Apache-2.0](LICENSE).
