# Draft written plainly. Needs a native reviewer before it ships.
app-title = Zeabur インストーラー
step-server = サーバー
step-access = アクセス
step-review = 確認
step-install = インストール

key-choose = 選択
key-next = 次へ
key-back = 戻る
key-field = 次の項目
key-lang = 言語
key-quit = 終了
key-install = インストール開始

server-title = このサーバー
server-host = ホスト名
server-system = OS
server-cpu = CPU
server-memory = メモリ
server-disk = ディスク
server-address = アドレス
cpu-cores = { $n } コア
memory-gb = { $gb } GB
disk-free = 空き { $free } GB（全体 { $total } GB）
addr-private = { $addr }（プライベート）

distro-title = インストールする Kubernetes
distro-kubesolo = 1 台だけで動き、いちばん軽量です。メモリが 1〜2 GB の小さなサーバー向けです。
distro-k3s = 軽くて広く使われています。あとからサーバーを追加できます。
distro-rke2 = CIS などのセキュリティ基準に沿って設定済みです。メモリが 4 GB 以上必要です。

checks-title = チェック結果
checks-summary = { $n ->
    [0] 問題はありません
   *[other] 確認してほしいことが { $n } 件あります
}
check-ports = ポート 80、443、6443 は空いています
check-swap = スワップが有効です。Kubernetes はスワップなしのほうが安定するため、インストーラーが無効にします。

access-title = Zeabur を開く方法
access-private-note = このサーバーからはプライベートアドレスしか見えません。お使いのパソコンからはどのアドレスで接続しますか？
access-addr-private = { $addr }（プライベートアドレス）
access-addr-other = ほかのアドレスまたはドメイン
field-address = アドレス
field-domain = ドメイン（省略可）
field-domain-hint = 空欄ならアドレスで開きます。HTTPS にはドメインが必要です。
field-name = 名前
field-name-hint = Zeabur で行った変更の横に表示されます。

review-title = インストール前の確認
review-distro = Kubernetes
review-address = アドレス
review-domain = ドメイン
review-name = 名前
review-no-domain = なし（アドレスで開きます）
review-plan-title = このあと行うこと
plan-download = 約 { $gb } GB をダウンロードします
plan-disk = ディスクを約 { $gb } GB 使います
plan-time = 約 { $min } 分かかります
plan-restart = サーバーは再起動しません。

install-title = インストール中
install-elapsed = 経過時間 { $time }
install-left = 残り約 { $time }
install-done = Zeabur の準備ができました。ブラウザーで { $url } を開いてください。
install-downloaded = { $done } / { $total } MB
duration = { $m ->
    [0] { $s }秒
   *[other] { $m }分{ $s }秒
}
install-step-check = サーバーを確認
install-step-download = K3s とイメージをダウンロード
install-step-k3s = K3s を起動
install-step-network = ネットワークを設定
install-step-zeabur = Zeabur をインストール
install-step-ready = Zeabur の起動を待つ
log-title = ログ

lang-title = 言語
lang-hint = すぐに切り替わります。
lang-needs-utf8 = UTF-8 の端末が必要です
fallback-note = この端末では中国語と日本語を表示できないため、英語で表示します。
