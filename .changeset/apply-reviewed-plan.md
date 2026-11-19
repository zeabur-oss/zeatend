---
default: minor
---

# `apply --plan` applies the plan you reviewed

`zeatend plan --json > plan.json` writes the plan, and `zeatend apply --plan plan.json` applies it. The plan holds the hash of each file it writes, the installed release and the install record as they were. When one of them changed since, or the plan made now writes something else, `apply` changes nothing and asks you to make and review the plan again. A file, release or record that the plan itself already left is fine, so a second `apply --plan` after an interruption goes on. The install screens pass the plan you confirmed to the install unit in the same way. `apply` without `--plan` plans and applies as before.
