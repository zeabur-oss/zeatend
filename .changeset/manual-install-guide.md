---
default: minor
---

# A guide to install by hand, step by step

The new page "Install by hand" runs the steps of `apply` one at a time, with a `zeatend doctor` check after each, for example to download the release on another computer when the server cannot reach GitHub. `apply` and `join` now number their steps, such as `==> 3/10 Writing the configuration files`. `zeatend plan --json` gives the files to write in `hostFiles`, each with its path, mode and content, in place of `hostFilePaths`.
