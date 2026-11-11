---
name: zeabur-zeatend-dev
description: Development loop for the Zeabur Zeatend repository — the checks CI runs and the shared skills that hold the engineering standards. Use before changing any Rust, Nix or CI file in this repo.
---

# Zeabur Zeatend development loop

Zeatend is a Kubernetes cluster manager for K3s, RKE2 and KubeSolo over SSH, with a terminal UI.
The binary crate is `zeabur-zeatend`. Pull request rules: `CONTRIBUTING.md`.

## Checks (what CI runs)
Each CI job runs `nix develop -c scripts/ci/<job>.sh` (`lint`, `test`). Underneath:
```sh
nix develop -c bash -c 'cargo fmt --all --check && nixfmt --check flake.nix && shellcheck scripts/ci/*.sh && actionlint'
nix develop -c cargo lint
RUSTDOCFLAGS="-D warnings" nix develop -c cargo doc --workspace --no-deps --document-private-items --locked
nix develop -c cargo nextest run --workspace --all-features --locked --no-tests=warn
nix develop -c cargo deny --locked check
```

## Shared skills
- Rust, lints, toolchain: `zeabur-oss-rust`. Comments, tests, design, scale: `zeabur-oss-standards`
  (apply it to every comment, commit message, test and abstraction you add).
- Adding or updating a dependency: `zeabur-oss-dependencies`. CI and failed runs: `zeabur-oss-ci`.
- Branches, commits, pull requests, issues: `zeabur-oss-workflow`. Protocol behaviour (SSH and
  the like): `zeabur-oss-spec-work`.
- Docs, strings, README text: `zeabur-writing` and `no-ai-flavour`.
