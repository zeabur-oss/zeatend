# Contributing

Zeabur Zeatend is a Kubernetes cluster manager for K3s, RKE2 and KubeSolo over SSH, with a
terminal UI.

## Developer Certificate of Origin

This project uses the [Developer Certificate of Origin 1.1](DCO). Sign off every commit to
certify that you wrote it or otherwise have the right to submit it under the
[Apache License 2.0](LICENSE):

```sh
git commit -s
```

CI rejects pull requests whose commits lack a `Signed-off-by` line matching the author. Commit
messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`,
`docs:`, `ci:`, `chore:`). Pull requests are squash-merged, so the pull request title is the commit
message on `main` and follows the same form. Do not add an AI `Co-Authored-By` trailer.

## Before you open a pull request

Each job in `.github/workflows/ci.yml` runs `nix develop -c scripts/ci/<job>.sh`, so
`nix develop -c scripts/ci/lint.sh` runs a job exactly as CI does. The jobs are `lint` and `test`.
Underneath:

```sh
cargo fmt --all --check && nixfmt --check flake.nix && shellcheck scripts/ci/*.sh && actionlint
cargo lint                                   # clippy, all targets and features, -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --locked
cargo nextest run --workspace --all-features --locked --no-tests=warn
cargo deny --locked check
```

A pull request titled `feat`, `fix` or `perf`, or marked breaking with `!`, adds a change file in
`.changeset/` (`knope document-change`) or carries the label `changelog: none`.

## Toolchain, lints and dependencies

Shared with [Zeabur Builder](https://github.com/zeabur-oss/builder) and
[Zeabur Identity](https://github.com/zeabur-oss/identity):

- `flake.lock` is the only place the compiler version is set: the newest nightly in the locked
  rust-overlay that has every component listed in `flake.nix`. Renovate's lock file maintenance
  (`renovate.json`) moves it every Monday.
- Lints are set once in `Cargo.toml`. All are `warn`, and `cargo lint` turns them into errors.
  Suppress with `#[expect(lint, reason = "…")]`. Only the binary's `output` module writes to
  stdout or stderr.
- `cargo deny check` enforces `deny.toml`: OSI-approved licences, crates.io only, aws-lc-rs as
  the only cryptographic backend, no advisories.
- Cargo does not pick a release younger than seven days (`.cargo/config.toml`).

Documentation is written in English. The conventions for agents and contributors are in
`.claude/skills`.
