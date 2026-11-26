---
default: patch
---

# An uninstall sends its events

`zeatend uninstall` now sends `zeatend_plan`, `zeatend_apply_started` and `zeatend_apply_finished` with `action: uninstall`, before it removes anything, unless telemetry is off. See the experience program reference.
