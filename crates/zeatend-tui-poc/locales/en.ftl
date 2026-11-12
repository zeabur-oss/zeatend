app-title = Zeatend
step-server = Server
step-access = Access
step-review = Review
step-install = Install

key-choose = choose
key-next = continue
key-back = back
key-field = next field
key-lang = language
key-quit = quit
key-install = install

server-title = This server
server-host = Hostname
server-system = System
server-cpu = CPU
server-memory = Memory
server-disk = Disk
server-address = Address
cpu-cores = { $n ->
    [one] 1 core
   *[other] { $n } cores
}
memory-gb = { $gb } GB
disk-free = { $free } GB free of { $total } GB
addr-private = { $addr } (private)

distro-title = Which Kubernetes to install
distro-kubesolo = Single node and the lightest. For small servers with 1 to 2 GB of memory.
distro-k3s = Light and widely used. You can add more servers later.
distro-rke2 = Hardened for security rules such as CIS. Needs at least 4 GB of memory.

checks-title = Checks
checks-summary = { $n ->
    [0] Everything looks good
    [one] 1 thing to know
   *[other] { $n } things to know
}
check-ports = Ports 80, 443 and 6443 are free
check-swap = Swap is on. Kubernetes runs better without it, so the installer turns it off.

access-title = How your computer reaches the cluster
access-private-note = This server only sees a private address. Which address does your computer use to reach it?
access-addr-private = { $addr }, the private address
access-addr-other = Another address or domain
field-address = Address
field-domain = Name for the address (optional)
field-domain-hint = Added to the API certificate, so kubectl can use the name.
field-name = Cluster name
field-name-hint = Shown as the context name in your kubeconfig.

review-title = Check before installing
review-distro = Kubernetes
review-address = Address
review-domain = Domain
review-name = Cluster name
review-no-domain = None, kubectl uses the address
review-plan-title = What happens next
plan-download = Downloads about { $gb } GB
plan-disk = Uses about { $gb } GB of disk
plan-time = { $min ->
    [one] Takes about 1 minute
   *[other] Takes about { $min } minutes
}
plan-restart = The server does not restart.

install-title = Installing
install-elapsed = Elapsed { $time }
install-left = About { $time } left
install-done = The cluster is ready. kubeconfig saved to { $path }.
install-downloaded = { $done } of { $total } MB
duration = { $m ->
    [0] { $s } s
   *[other] { $m } min { $s } s
}
install-step-check = Check the server
install-step-download = Download K3s and images
install-step-k3s = Start K3s
install-step-network = Set up the network
install-step-ready = Wait for the node to be Ready
log-title = Log

lang-title = Language
lang-hint = Changes right away.
lang-needs-utf8 = needs a UTF-8 terminal
fallback-note = This terminal cannot show Chinese, so Zeatend uses English.
