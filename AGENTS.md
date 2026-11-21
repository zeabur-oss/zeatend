# AGENTS.md

Zeabur Zeatend: a Kubernetes cluster manager for K3s, RKE2 and KubeSolo over SSH, with a terminal
UI. A Rust workspace in `crates/`: the binary `zeatend` (`crates/zeatend`), the host operations
(`crates/zeatend-host`) and the plan, which does no I/O (`crates/zeatend-plan`).

- Run every command inside `nix develop` (`nix develop -c <command>`). Do not use system tools.
- Before you commit, run the checks in `CONTRIBUTING.md`; CI runs the same jobs through
  `scripts/ci/<job>.sh`.
- Commits are Conventional Commits signed off with `git commit -s`. No AI trailer
  (`Co-Authored-By`) in commits or pull requests.
- Text in the repository is English, except `README.zh-TW.md`, `README.zh-CN.md`, the user docs
  in `docs/zh-TW/` and `docs/zh-CN/`, and the 繁體中文 and 简体中文 catalogues in
  `crates/zeatend/src/i18n/`. A change to `README.md` or to a page in `docs/` changes the two
  translations in the same pull request.
