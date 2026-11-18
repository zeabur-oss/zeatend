---
default: minor
---

# Downloads resume after a failure and show their progress

`zeatend apply` shows how much of each download has arrived and how fast, about once a second. A
download that fails or stops sending data for a minute is tried again from where it stopped, and
the file is still checked against its SHA-256 digest.
