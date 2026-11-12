app-title = Zeatend
step-server = 伺服器
step-access = 連線方式
step-review = 確認
step-install = 安裝

key-choose = 選擇
key-next = 繼續
key-back = 上一步
key-field = 下一欄
key-lang = 語言
key-quit = 離開
key-install = 開始安裝

server-title = 這台伺服器
server-host = 主機名稱
server-system = 系統
server-cpu = CPU
server-memory = 記憶體
server-disk = 磁碟
server-address = 位址
cpu-cores = { $n } 核心
memory-gb = { $gb } GB
disk-free = 還有 { $free } GB 可用，共 { $total } GB
addr-private = { $addr }（內網）

distro-title = 要裝哪一種 Kubernetes
distro-kubesolo = 只有一台機器，最省資源。適合記憶體只有 1 到 2 GB 的小主機。
distro-k3s = 輕巧、用的人多，之後還能再加伺服器。
distro-rke2 = 預先照 CIS 等資安規範設定好，記憶體至少要 4 GB。

checks-title = 檢查結果
checks-summary = { $n ->
    [0] 一切正常
   *[other] 有 { $n } 件事要留意
}
check-ports = 80、443、6443 連接埠都沒被占用
check-swap = 這台有開 swap。Kubernetes 不開 swap 跑得比較好，安裝程式會幫你關掉。

access-title = 你的電腦要怎麼連到叢集
access-private-note = 這台伺服器只看得到內網位址。你的電腦是用哪個位址連到它？
access-addr-private = { $addr }，也就是內網位址
access-addr-other = 其他位址或網域
field-address = 位址
field-domain = 位址的名稱（可不填）
field-domain-hint = 會加進 API 憑證，kubectl 才能用這個名稱連線。
field-name = 叢集名稱
field-name-hint = 會當成 kubeconfig 裡的 context 名稱。

review-title = 安裝前確認一下
review-distro = Kubernetes
review-address = 位址
review-domain = 網域
review-name = 叢集名稱
review-no-domain = 沒有填，kubectl 用位址連線
review-plan-title = 接下來會做什麼
plan-download = 下載大約 { $gb } GB
plan-disk = 用掉大約 { $gb } GB 磁碟空間
plan-time = 大約要 { $min } 分鐘
plan-restart = 伺服器不會重新開機。

install-title = 正在安裝
install-elapsed = 已經過了 { $time }
install-left = 大約還要 { $time }
install-done = 叢集已經可以用了。kubeconfig 存在 { $path }。
install-downloaded = { $done } / { $total } MB
duration = { $m ->
    [0] { $s } 秒
   *[other] { $m } 分 { $s } 秒
}
install-step-check = 檢查伺服器
install-step-download = 下載 K3s 和映像檔
install-step-k3s = 啟動 K3s
install-step-network = 設定網路
install-step-ready = 等節點變成 Ready
log-title = 紀錄

lang-title = 語言
lang-hint = 選了馬上換。
lang-needs-utf8 = 終端機要支援 UTF-8
fallback-note = 這個終端機顯示不了中文，所以 Zeatend 改用英文。
