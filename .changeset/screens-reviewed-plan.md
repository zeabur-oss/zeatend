---
default: patch
---

# The install screens start the install with the plan you reviewed

The Review screen keeps the plan it shows, with what it found on the host, and the install unit applies that plan. When the host changed after the Review screen, `apply --plan` changes nothing and says so, instead of the screens comparing the release and node name only.
