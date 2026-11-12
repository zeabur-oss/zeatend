app-title = Zeabur 安装程序
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

access-title = 你要怎么打开 Zeabur
access-private-note = 这台服务器只能看到内网地址。你的电脑是用哪个地址连到它的？
access-addr-private = { $addr }，也就是内网地址
access-addr-other = 其他地址或域名
field-address = 地址
field-domain = 域名（可不填）
field-domain-hint = 不填就用地址打开。要用 HTTPS 得有域名。
field-name = 你的名字
field-name-hint = 在 Zeabur 里，你做的改动旁边会显示这个名字。

review-title = 安装前确认一下
review-distro = Kubernetes
review-address = 地址
review-domain = 域名
review-name = 你的名字
review-no-domain = 没有填，Zeabur 会用地址打开
review-plan-title = 接下来会做什么
plan-download = 下载大约 { $gb } GB
plan-disk = 占用大约 { $gb } GB 磁盘空间
plan-time = 大约要 { $min } 分钟
plan-restart = 服务器不会重启。

install-title = 正在安装
install-elapsed = 已用时 { $time }
install-left = 大约还要 { $time }
install-done = Zeabur 已经可以用了。用浏览器打开 { $url }。
install-downloaded = { $done } / { $total } MB
duration = { $m ->
    [0] { $s } 秒
   *[other] { $m } 分 { $s } 秒
}
install-step-check = 检查服务器
install-step-download = 下载 K3s 和镜像
install-step-k3s = 启动 K3s
install-step-network = 配置网络
install-step-zeabur = 安装 Zeabur
install-step-ready = 等 Zeabur 启动
log-title = 日志

lang-title = 语言
lang-hint = 选了马上切换。
lang-needs-utf8 = 终端需要支持 UTF-8
fallback-note = 这个终端显示不了中文和日文，所以安装程序改用英文。
