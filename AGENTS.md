# AGENTS.md

Zeabur Zeatend: a Kubernetes cluster manager for K3s, RKE2 and KubeSolo over SSH, with a terminal
UI. A Rust workspace in `crates/`; the binary is `zeabur-zeatend`.

- Run every command inside `nix develop` (`nix develop -c <command>`). Do not use system tools.
- Before you commit, run the checks in `CONTRIBUTING.md`; CI runs the same jobs through
  `scripts/ci/<job>.sh`.
- Commits are Conventional Commits signed off with `git commit -s`. No AI trailer
  (`Co-Authored-By`) in commits or pull requests.
- Text in the repository is English.
