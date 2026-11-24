---
default: minor
---

# `zeatend plan`, `zeatend apply` and the install screens join the experience program

They send the version, the distribution, the role, whether the run came from a command or the install screens, which host checks failed, and whether an install or join worked and at which step it stopped, with a random installation ID in `/etc/zeatend/installation-id`. They never send names, addresses, tokens, paths or error messages. `--no-telemetry`, `telemetry: false` in the configuration file, `DO_NOT_TRACK=1` or `ZEATEND_TELEMETRY=off` turns it off; `ZEATEND_TELEMETRY=debug` prints what would be sent. See the [experience program](https://oss.zeabur.com/docs/zeatend/reference/telemetry/) reference.
