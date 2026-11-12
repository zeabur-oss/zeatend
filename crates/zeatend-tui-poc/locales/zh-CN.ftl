app-title = Zeatend
step-server = 服务器
step-access = 访问方式
step-review = 确认
step-install = 安装

key-choose = 选择
key-next = 继续
key-back = 上一步
key-field = 下一项
key-lang = 语言
key-quit = 退出
key-install = 开始安装

server-title = 这台服务器
server-host = 主机名
server-system = 系统
server-cpu = CPU
server-memory = 内存
server-disk = 磁盘
server-address = 地址
cpu-cores = { $n } 核
memory-gb = { $gb } GB
disk-free = 还有 { $free } GB 可用，共 { $total } GB
addr-private = { $addr }（内网）

distro-title = 要装哪种 Kubernetes
distro-kubesolo = 只有一台机器，最省资源。适合内存只有 1 到 2 GB 的小主机。
distro-k3s = 轻量、用的人多，以后还能再加服务器。
distro-rke2 = 预先按 CIS 等安全规范配置好，内存至少要 4 GB。

checks-title = 检查结果
checks-summary = { $n ->
    [0] 一切正常
   *[other] 有 { $n } 件事要注意
}
check-ports = 80、443、6443 端口都没被占用
check-swap = 这台开了 swap。Kubernetes 不开 swap 跑得更好，安装程序会帮你关掉。

access-title = 你的电脑要怎么连到集群
access-private-note = 这台服务器只能看到内网地址。你的电脑是用哪个地址连到它的？
access-addr-private = { $addr }，也就是内网地址
access-addr-other = 其他地址或域名
field-address = 地址
field-domain = 地址的名称（可不填）
field-domain-hint = 会加进 API 证书，kubectl 才能用这个名称连接。
field-name = 集群名称
field-name-hint = 会当成 kubeconfig 里的 context 名称。

review-title = 安装前确认一下
review-distro = Kubernetes
review-address = 地址
review-domain = 域名
review-name = 集群名称
review-no-domain = 没有填，kubectl 用地址连接
review-plan-title = 接下来会做什么
plan-download = 下载大约 { $gb } GB
plan-disk = 占用大约 { $gb } GB 磁盘空间
plan-time = 大约要 { $min } 分钟
plan-restart = 服务器不会重启。

install-title = 正在安装
install-elapsed = 已用时 { $time }
install-left = 大约还要 { $time }
install-done = 集群已经可以用了。kubeconfig 存在 { $path }。
install-downloaded = { $done } / { $total } MB
duration = { $m ->
    [0] { $s } 秒
   *[other] { $m } 分 { $s } 秒
}
install-step-check = 检查服务器
install-step-download = 下载 K3s 和镜像
install-step-k3s = 启动 K3s
install-step-network = 配置网络
install-step-ready = 等节点变成 Ready
log-title = 日志

lang-title = 语言
lang-hint = 选了马上切换。
lang-needs-utf8 = 终端需要支持 UTF-8
fallback-note = 这个终端显示不了中文，所以 Zeatend 改用英文。
