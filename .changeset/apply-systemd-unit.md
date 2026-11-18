---
default: minor
---

# `apply` goes on after a disconnect

On a host with systemd, `zeatend apply` runs the install in the transient unit `zeatend-install-<id>.service` and shows its progress from the journal. When the SSH session ends, the install goes on; run `sudo zeatend apply` again to follow it. `apply --foreground` runs the install in the same process, as before, and `apply --json` prints the progress as JSON lines.
