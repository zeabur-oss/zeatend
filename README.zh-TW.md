<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/cover-zh-TW-dark.avif">
  <img alt="Zeatend：一台 Linux 主機，變成可以直接用的叢集。zeatend apply 把 Linux 主機變成執行中的 Kubernetes 叢集。" src="docs/images/cover-zh-TW-light.avif">
</picture>

<!-- The names of the languages, each in its own script. -->
<!-- vale ZeaburZhTW.Script = NO -->
[English](README.md) | 繁體中文 | [简体中文](README.zh-CN.md)
<!-- vale ZeaburZhTW.Script = YES -->

# Zeatend

Zeatend 是管理 Kubernetes 叢集的工具，概念和 [k3sup](https://github.com/alexellis/k3sup) 相近。它透過 SSH 在主機上安裝並管理 K3s、RKE2 和 KubeSolo 叢集，也提供終端機介面。

Zeatend 由 Zeabur 專案開發，目前還在早期開發階段。

## 安裝

### 需要的主機

- 一台以 systemd 啟動的 Linux 主機，處理器是 x86-64（amd64）或 ARM64（arm64）。K3s、RKE2 和 KubeSolo 都以 systemd 服務執行，所以主機沒有 systemd 時，`zeatend apply` 不會開始安裝。
- 能用 `sudo` 取得 root 權限。
- 記憶體：K3s 至少 1900 MiB，2 GiB 的主機就夠；KubeSolo 至少 1400 MiB；RKE2 需要更多。記憶體不夠時，主機檢查會提醒。
- 磁碟：叢集資料所在的檔案系統至少要有 10 GiB 可用空間，建議 30 GiB 以上。
- 網路：主機要能連到 GitHub，下載 K3s、RKE2 或 KubeSolo。
- Kubernetes 需要的連接埠沒有被占用。KubeSolo 的主機上不能有 Docker。

主機檢查也會看時間是否同步、swap 是否關閉，以及先前安裝的 Kubernetes 有沒有留下檔案。

### 下載 zeatend

在主機上執行下面的指令。第一行從 [GitHub Releases](https://github.com/zeabur-oss/zeatend/releases) 取得最新版本，例如 `v0.1.0`；要裝其他版本，把第一行換成 `version=v0.1.0` 這樣的版本號。

```sh
version=$(basename "$(curl -fsSLo /dev/null -w '%{url_effective}' https://github.com/zeabur-oss/zeatend/releases/latest)")
arch=$(uname -m | sed 's/x86_64/amd64/; s/aarch64/arm64/')
curl -fsSLO "https://github.com/zeabur-oss/zeatend/releases/download/$version/zeatend-$version-linux-$arch.tar.gz" &&
  curl -fsSLO "https://github.com/zeabur-oss/zeatend/releases/download/$version/SHA256SUMS" &&
  sha256sum --ignore-missing -c SHA256SUMS &&
  tar -xzf "zeatend-$version-linux-$arch.tar.gz" zeatend &&
  sudo install -m 0755 zeatend /usr/local/bin/zeatend &&
  zeatend --version
```

`sha256sum` 印出 `OK`，代表下載的檔案和 release 一致。`zeatend` 是靜態執行檔，不需要再安裝其他套件。

### 安裝叢集

用安裝畫面或指令，兩種擇一：

- 執行 `sudo zeatend`，依照畫面選擇發行版，填寫位址和節點名稱，確認安裝計畫後開始安裝。
- 先執行 `sudo zeatend plan` 檢視要安裝的項目和主機檢查，再執行 `sudo zeatend apply` 安裝。沒有 `/etc/zeatend/config.yaml` 時，zeatend 會用預設值安裝 K3s；要改用 RKE2 或 KubeSolo，先寫好設定檔，寫法見下方〈使用方式〉。

```sh
sudo zeatend plan
sudo zeatend apply
```

`plan` 不會變更主機。主機檢查沒有通過時，`apply` 不會開始安裝，請先依照訊息修正再執行一次。

### 安裝成功時

安裝完成時，安裝畫面會顯示「叢集已經可以使用」，下方是接下來可以執行的指令：

<img alt="繁體中文的安裝畫面：安裝完成，叢集已經可以使用。" src="docs/screens/80x24/zh_TW-done.svg" width="744">

`zeatend apply` 結束時，會印出版本、節點和接下來的指令。K3s 的輸出像這樣：

```text
==> 等待節點 vps-1 就緒
==> 完成
K3s v1.37.2+k3s1 已經在 vps-1 上執行。

在這台伺服器上列出節點：
  sudo /var/lib/rancher/k3s/data/current/bin/kubectl --kubeconfig /etc/rancher/k3s/k3s.yaml get nodes -o wide
```

終端機接著會印出把 kubeconfig 複製到你電腦上的指令。之後隨時可以執行 `sudo zeatend doctor` 檢查主機和叢集；有項目沒通過時，那一行會寫出該執行的指令。

## 使用方式

目前 `zeatend` 要以 root 身分，在它安裝的那台 Linux 主機上執行。不加指令執行 `sudo zeatend`，會開啟安裝畫面，可以選擇英文、繁體中文或簡體中文：

<img alt="繁體中文的伺服器畫面：主機資訊、K3s、RKE2 或 KubeSolo 的選項，以及主機檢查。" src="docs/screens/80x24/zh_TW-server.svg" width="744">

畫面會顯示主機資訊和檢查結果，讓你選擇發行版，填寫 API 伺服器的位址、DNS 名稱和節點名稱，最後顯示安裝計畫。你的選擇會寫進 `/etc/zeatend/config.yaml`，安裝過程和 `zeatend apply` 相同。所有畫面在 80x24 和 120x36 下的樣子都放在 [docs/screens](docs/screens)。語言跟主機檢查一樣取自語系設定，按 F2 可以切換。Linux 主控台這類無法顯示中文的終端機，會改用英文和 ASCII 邊框。

不開啟畫面時，`zeatend` 會讀取一個設定檔（預設是 `/etc/zeatend/config.yaml`），安裝依 CIS 基準強化的 K3s、RKE2 或 KubeSolo：

```yaml
distribution: k3s # k3s（預設）、rke2 或 kubesolo
host:
  tlsSan: [k8s.example.com]
```

K3s（`v1.37.2+k3s1`）和 RKE2（`v1.37.2+rke2r1`）安裝 Kubernetes 1.37。KubeSolo 還沒有 1.37 版，所以 KubeSolo `v1.3.0` 安裝的是 Kubernetes 1.36.6。

K3s 和 RKE2 照上游的預設，部署自帶的元件，例如 Traefik。要改用自己安裝的版本，就用 `disable` 關掉，例如 `disable: [traefik, metrics-server]`。`local-storage` 會關掉 K3s 的 local-path 儲存類別。KubeSolo 沒有這些元件。

`storage: {longhorn: true}` 會替主機準備好 Longhorn 需要的環境，Zeabur Catalog 會在多節點叢集上安裝 Longhorn：`apply` 用 apt 或 dnf 安裝 iSCSI initiator 和 NFS 用戶端，設定開機時載入 `iscsi_tcp` 和 `dm_crypt`，啟動 `iscsid`，並讓 multipathd 避開 Longhorn 的磁碟。節點缺少其中任何一項時，`zeatend doctor` 會回報；`zeatend node add` 會讓新節點沿用同樣的設定。

`apiServer.authenticationConfig` 是 API 伺服器的[驗證設定](https://kubernetes.io/docs/reference/access-authn-authz/authentication/#using-authentication-configuration)，照 Kubernetes 文件的格式寫，例如讓 API 伺服器接受某個 OpenID Connect 簽發者的權杖：

```yaml
apiServer:
  authenticationConfig:
    apiVersion: apiserver.config.k8s.io/v1
    kind: AuthenticationConfiguration
    jwt:
      - issuer:
          url: https://identity.example.com
          audiences: [kubernetes]
        claimMappings:
          username: {claim: preferred_username, prefix: "zeabur:"}
          groups: {claim: groups, prefix: "zeabur:"}
```

在伺服器節點上，zeatend 會把它寫成檔案交給 API 伺服器。欄位由 Kubernetes 在 API 伺服器啟動或重新讀取檔案時檢查。在 K3s 和 RKE2 上，如果設定裡沒有 `anonymous`，zeatend 會加上 `anonymous: {enabled: false}`，匿名請求就維持關閉。改了設定之後，下一次 `apply` 就會生效。

- `zeatend plan` 顯示 `apply` 會安裝哪些東西，並執行主機檢查；加上 `--json` 會以 JSON 輸出計畫。
- `zeatend apply` 負責安裝，或讓現有的安裝符合設定檔。重複執行也沒有問題。`apply --plan <檔案>` 會套用 `plan --json` 寫進檔案的計畫；如果計畫要寫入的檔案、安裝的版本或安裝紀錄在那之後有了變化，就不會做任何變更。
- `zeatend uninstall` 移除 K3s、RKE2 或 KubeSolo，以及 zeatend 寫入的檔案。它會先列出要移除和保留的項目，並詢問是否繼續（`--yes` 可以略過詢問）。除非加上 `--delete-volumes`，磁碟區都會保留。

在使用 systemd 的主機上，`apply` 會在暫時性單元 `zeatend-install-<id>.service` 裡執行安裝，並顯示進度。SSH 連線中斷後，安裝不會停止。再執行一次 `sudo zeatend apply` 就能接著檢視：安裝單元執行期間，`apply` 會從 journal 顯示這個單元的進度，不會另外開始新的安裝。完整輸出可以用 `journalctl -u 'zeatend-install-*'` 檢視。`apply --foreground` 不建立單元，直接在目前的 zeatend 裡執行安裝，`apply --json` 則以 JSON 逐行輸出進度。

設定檔裡有 `join` 區段時，這台主機會加入現有的 K3s 或 RKE2 叢集：

```yaml
distribution: k3s
join:
  server: https://10.0.0.5:6443 # RKE2 用 9345 埠
  role: worker # RKE2 也可以設為 server
  tokenFile: /etc/zeatend/join-token # 權限 0600
```

## 參與貢獻

請見 [CONTRIBUTING.md](CONTRIBUTING.md)。回報安全性問題的方式見 [SECURITY.md](SECURITY.md)。

## 授權

[Apache-2.0](LICENSE)。
