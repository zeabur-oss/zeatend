---
default: minor
---

# Host checks in 繁體中文 and 简体中文

`zeatend plan` and `zeatend apply` show the results of the host checks in English, 繁體中文 or 简体中文. The language comes from `LC_ALL`, `LC_MESSAGES` or `LANG`. Chinese needs a UTF-8 locale and a terminal other than the Linux console (`TERM=linux`); otherwise the text stays in English. The install unit of `apply` gets the session's locale and `TERM`, so it shows the same language.

In `plan --json` of a joining node, a port's `purpose` is now a key such as `kubernetesApi` instead of an English phrase.
