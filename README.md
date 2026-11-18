# Zeabur Zeatend

A Kubernetes cluster manager in the spirit of [k3sup](https://github.com/alexellis/k3sup). It
sets up and manages K3s, RKE2 and KubeSolo clusters on hosts over SSH, with a terminal UI.

Zeatend is made by the Zeabur project and is in early development.

## Usage

Today `zeatend` runs on the Linux host it installs, as root. It reads one configuration file,
`/etc/zeatend/config.yaml` by default, and installs K3s, RKE2 or KubeSolo hardened to the CIS
benchmark:

```yaml
distribution: k3s # k3s (default), rke2 or kubesolo
host:
  tlsSan: [k8s.example.com]
```

- `zeatend plan` shows what `apply` would install and runs the host checks; `--json` prints the
  plan as JSON.
- `zeatend apply` installs, or brings an install in line with the configuration. It is safe to
  run again.

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
