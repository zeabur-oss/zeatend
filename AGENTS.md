# AGENTS.md

Zeabur Zeatend: a Kubernetes cluster manager for K3s, RKE2 and KubeSolo over SSH, with a terminal
UI. A Rust workspace in `crates/`; the binary is `zeabur-zeatend`.

- Run every command inside `nix develop` (`nix develop -c <command>`). Do not use system tools.
- Before you commit, run the checks in `.claude/skills/zeabur-zeatend-dev/SKILL.md`; CI runs the
  same jobs through `scripts/ci.sh`.
- Skills in `.claude/skills` hold the conventions. `zeabur-oss-*` and `zeabur-writing` are copies of
  [zeabur-oss/skills](https://github.com/zeabur-oss/skills); change them there.
- Commits are Conventional Commits signed off with `git commit -s`. No AI trailer
  (`Co-Authored-By`) in commits or pull requests.
- Text in the repository is English.
