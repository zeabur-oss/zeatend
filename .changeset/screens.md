---
default: minor
---

# Install screens in English, 繁體中文 and 简体中文

`sudo zeatend` with no command opens four screens that lead to an install: Server shows the host, the choice of KubeSolo, K3s or RKE2 and the host checks of that choice; Access asks for the address and DNS name of the API server and the node name; Review shows the release and the node name from the plan; Install shows the progress. The choices go into the configuration file, which keeps its other settings, and the install runs in a `zeatend-install-<id>.service` unit, as `zeatend apply` does, in the language of the screens. It goes on when the SSH session ends, and `sudo zeatend` opens the Install screen of a running install again.

The screens are in English, 繁體中文 or 简体中文 from the locale, and F2 changes the language. A terminal that cannot show Chinese gets English with ASCII borders. Without a terminal, `zeatend` says to run `zeatend plan` and `zeatend apply`.
