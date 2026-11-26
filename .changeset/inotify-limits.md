---
default: patch
---

# Every node gets higher inotify limits

The containers of a node share the inotify limits of root on the host. With the kernel's 128 instances, the pods that start last after a reboot could fail with "too many open files". zeatend now writes `/etc/sysctl.d/60-zeatend-inotify.conf` on every server and every node that `node add` joins. It sets `fs.inotify.max_user_instances` to 1024 and raises `fs.inotify.max_user_watches` to 524288 when the host has less.
