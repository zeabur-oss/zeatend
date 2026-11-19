//! Traditional Chinese, as written in Taiwan.

use zeatend_plan::cluster::{Distribution, PortUse};
use zeatend_plan::finding::{Finding, Status, Topic};
use zeatend_plan::precondition::Stale;
use zeatend_plan::record::ZEATEND_VERSION;

use super::{Joined, Key, Messages, Ready, Screen, Screens, gib, zh_list};

pub(super) const MESSAGES: Messages = Messages {
    checking_host: "正在檢查主機",
    apply_stops: "修好失敗的項目之前，apply 不會繼續執行。",
    host_not_ready: "主機還沒準備好，修好失敗的項目後再執行一次 apply",
    not_plan: |path, reason| {
        format!("沒有做任何變更：{path} 不是 `zeatend plan --json` 輸出的計畫（{reason}）")
    },
    stale,
    doctor_needs_root: "用 root 身分執行 doctor（加上 sudo），檢查需要讀取叢集的檔案和憑證",
    no_config_installer,
    no_config_defaults,
    following_unit,
    started_unit,
    starting_unit,
    restarting_unit,
    waiting_for_node,
    done: DONE,
    ready,
    joined,
    status,
    topic,
    finding,
    screens: &SCREENS,
};

const SCREENS: Screens = Screens {
    screen: |screen| match screen {
        Screen::Server => "伺服器",
        Screen::Access => "連線方式",
        Screen::Review => "確認",
        Screen::Install => "安裝",
    },
    key: |key| match key {
        Key::Choose => "選擇",
        Key::Next => "繼續",
        Key::Back => "上一步",
        Key::Field => "下一欄",
        Key::Language => "語言",
        Key::Quit => "離開",
        Key::Install => "開始安裝",
        Key::Leave => "關閉畫面，安裝會繼續",
        Key::Close => "關閉",
    },

    server_title: "這台伺服器",
    host_name: "主機名稱",
    system: "系統",
    cpu: "CPU",
    memory: "記憶體",
    addresses: "位址",
    cpus: |n| format!("{n} 核 CPU"),
    private_address: |address| format!("{address}（內網）"),
    no_address: "沒有",
    distribution_title: "要安裝哪一種 Kubernetes",
    distribution_about: |distribution| match distribution {
        Distribution::KubeSolo => "只有一個節點，最省資源，不能加入工作節點。",
        Distribution::K3s => "輕巧，比照 RKE2 的 CIS 設定強化，之後可以加入工作節點。",
        Distribution::Rke2 => "使用 RKE2 的 CIS 設定，適合三台伺服器組成的控制平面。",
    },
    checks_title: "主機檢查",
    checks_summary: |failed, warned| match (failed, warned) {
        (0, 0) => "全部通過".to_owned(),
        (0, w) => format!("{w} 項要注意"),
        (f, 0) => format!("{f} 項失敗"),
        (f, w) => format!("{f} 項失敗，{w} 項要注意"),
    },
    checks_stop: "修好失敗的項目，再執行一次 sudo zeatend。",

    access_title: "你的電腦怎麼連到叢集",
    access_private: "這台伺服器只有內網位址。你的電腦用哪個位址連到它？",
    access_public: "你的電腦用這個位址連到 API 伺服器。如果用的是別的位址，請修改。",
    field_address: "位址",
    field_address_hint: "API 伺服器的憑證會包含這個位址，kubectl 才能連線。",
    field_name: "DNS 名稱（可不填）",
    field_name_hint: "指向這台伺服器的名稱，憑證也會包含它。",
    field_node: "節點名稱（可不填）",
    field_node_hint: |name| format!("不填的話，節點名稱是 {name}。"),
    bad_address: "請輸入 IP 位址，例如 203.0.113.7。",
    bad_name: "請輸入 DNS 名稱，例如 k8s.example.com。",
    bad_node: "請用小寫英文字母、數字、- 和 .，開頭和結尾要是字母或數字。",

    review_title: "安裝前確認",
    review_release: "Kubernetes",
    review_address: "位址",
    review_name: "DNS 名稱",
    review_no_name: "沒有填，kubectl 用位址連線",
    review_node: "節點名稱",
    review_config: "設定檔",
    review_next: "接下來會做什麼",
    next_write: |path| format!("zeatend 把這些選擇寫進 {path}。"),
    next_update: |path| format!("zeatend 把這些選擇存進 {path}，其他設定不變。"),
    next_unit: "安裝在 systemd 單元裡執行，SSH 連線中斷也會繼續。",
    next_follow: "再執行一次 sudo zeatend 就能繼續查看進度。",

    install_title: "正在安裝",
    install_unit: |unit| format!("在 {unit} 裡執行"),
    install_elapsed: |time| format!("已經過 {time}"),
    install_starting: "正在開始安裝",
    install_done: "叢集已經可以使用。",
    install_failed: "安裝停止了。",
    duration: |minutes, seconds| {
        if minutes == 0 {
            format!("{seconds} 秒")
        } else {
            format!("{minutes} 分 {seconds} 秒")
        }
    },
    log_title: "紀錄",
    result_title: "接下來",
    result_more: "按 Enter 關閉畫面後，終端機會印出完整內容。",

    language_title: "語言",
    language_hint: "選了馬上切換。",
    language_needs_utf8: "終端機要支援 UTF-8",

    reading_host: "正在讀取這台主機的資訊",
    left: |unit| format!("安裝會在 {unit} 裡繼續。執行 `sudo zeatend` 可以再次查看進度。"),
    no_terminal: "zeatend 要在終端機裡開啟畫面。沒有終端機的話，請寫好 \
                  /etc/zeatend/config.yaml，再執行：\n  sudo zeatend plan\n  sudo zeatend apply",
    needs_root: "用 root 身分執行 zeatend：sudo zeatend",
    needs_systemd: "這台主機沒有使用 systemd，無法開啟畫面。請寫好 /etc/zeatend/config.yaml，\
                    再執行 sudo zeatend apply --foreground",
    joins: |path| format!("{path} 會讓這台主機加入叢集，請執行 sudo zeatend apply"),
};

const DONE: &str = "完成";

fn no_config_installer(config: &str, installer: &str) -> String {
    format!("找不到設定檔 {config}，改用 zeabur-installer 寫在 {installer} 的發行版和主機設定。")
}

fn no_config_defaults(config: &str) -> String {
    format!("找不到設定檔 {config}，改用預設值：在這台主機上安裝 K3s。")
}

fn following_unit(unit: &str) -> String {
    format!("{unit} 已經在安裝，接著顯示它的進度。")
}

fn started_unit(unit: &str) -> String {
    format!("安裝在 {unit} 裡執行。這個工作階段結束後，執行 `sudo zeatend apply` 就能再看到進度。")
}

fn starting_unit(unit: &str) -> String {
    format!("正在啟動 {unit}，第一次啟動要花幾分鐘")
}

fn restarting_unit(unit: &str) -> String {
    format!("正在重新啟動 {unit}，套用新的設定")
}

fn waiting_for_node(node: &str) -> String {
    format!("等待節點 {node} 就緒")
}

fn ready(r: &Ready<'_>) -> String {
    let host = r.host.unwrap_or("<伺服器位址>");
    let node = r.node;
    let sed = r.rewrite.map_or_else(String::new, |(from, to)| {
        format!("  sed -i.bak 's#{from}#{to}#' ~/.kube/{node}.yaml\n")
    });
    let mut text = format!(
        "{} 已經在 {node} 上執行。\n\n在這台伺服器上列出節點：\n  {} get nodes -o wide\n\n\
         在你自己的電腦上使用這個叢集：\n  \
         mkdir -p ~/.kube\n  (umask 077; ssh <使用者>@{host} sudo cat {} > ~/.kube/{node}.yaml)\n{sed}  KUBECONFIG=~/.kube/{node}.yaml kubectl get nodes\n\n\
         拿到這個檔案的人就是叢集管理員，要像 SSH 金鑰一樣保管好。",
        r.cluster, r.kubectl, r.kubeconfig
    );
    if r.rewrite.is_none() {
        text.push_str(
            "如果你的電腦要用其他位址連到伺服器，改掉檔案裡的 `server:`，並把這個位址加進 \
             host.tlsSan。",
        );
    }
    if r.add_nodes {
        text.push_str(
            "\n\n要加入節點，在這台伺服器上執行 `sudo zeatend node add`，它會印出新主機要執行的指令。",
        );
    }
    text
}

fn joined(j: &Joined<'_>) -> String {
    let role = if j.server {
        "伺服器"
    } else {
        "工作節點"
    };
    format!(
        "{} 已經以{role}的身分加入叢集。在伺服器上列出節點：\n  {} get nodes -o wide",
        j.node, j.kubectl
    )
}

fn stale(stale: &Stale) -> String {
    let why = match stale {
        Stale::Writes => {
            "設定檔、這台主機或 zeatend 有了變化，現在 apply 要做的事和檔案裡的計畫不一樣"
                .to_owned()
        }
        Stale::File(path) => format!("產生計畫之後，{path} 被改過了"),
        Stale::Release { reviewed, now } => format!(
            "產生計畫時安裝的版本是 {}，現在是 {}",
            reviewed.as_deref().unwrap_or("（沒有）"),
            now.as_deref().unwrap_or("（沒有）")
        ),
        Stale::Record => "產生計畫之後，安裝紀錄有了變化".to_owned(),
    };
    format!("沒有做任何變更：{why}。請重新執行 `zeatend plan --json`，看過新的計畫再套用。")
}

const fn status(status: Status) -> &'static str {
    match status {
        Status::Pass => "通過",
        Status::Warn => "注意",
        Status::Fail => "失敗",
    }
}

const fn topic(topic: Topic) -> &'static str {
    match topic {
        Topic::Root => "root 權限",
        Topic::Systemd => "systemd",
        Topic::Time => "時鐘",
        Topic::Swap => "置換空間",
        Topic::CrashKernel => "kdump 記憶體",
        Topic::OtherKubernetes => "其他 Kubernetes",
        Topic::Ports => "連接埠",
        Topic::DiskSpace => "磁碟空間",
        Topic::DiskEncryption => "磁碟加密",
        Topic::Docker => "Docker",
        Topic::Service => "服務",
        Topic::Api => "Kubernetes API",
        Topic::InstallRecord => "安裝紀錄",
        Topic::Nodes => "節點",
        Topic::KubeletReserves => "kubelet 保留資源",
        Topic::SecretsEncryption => "Secret 加密",
        // Plan findings stay in English until zeatend#32 words them.
        Topic::Memory
        | Topic::Release
        | Topic::Server
        | Topic::Firewall
        | Topic::HighAvailability
        | Topic::Storage
        | Topic::JoinToken => super::en::topic(topic),
    }
}

fn and_list(items: &[String]) -> String {
    zh_list(items, "和")
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm for each finding, so a finding without words does not compile"
)]
fn finding(finding: &Finding) -> String {
    match finding {
        Finding::RootOk => "以 root 身分執行".to_owned(),
        Finding::RootNo => "用 root 身分執行 apply（加上 sudo）".to_owned(),
        Finding::SystemdOk => "init 系統是 systemd".to_owned(),
        Finding::SystemdNo => {
            "Kubernetes 以 systemd 服務執行，但這台主機沒有使用 systemd".to_owned()
        }
        Finding::TimeOk => "時鐘已同步".to_owned(),
        Finding::TimeDrift => "時鐘沒有同步，時間一偏移，憑證和權杖就會失效（執行 \
                               timedatectl set-ntp true）"
            .to_owned(),
        Finding::TimeUnknown => "無法向 timedatectl 確認時鐘是否同步".to_owned(),
        Finding::SwapOff => "置換空間已關閉".to_owned(),
        Finding::SwapOn => "置換空間已開啟，kubelet 要求關閉（執行 swapoff -a，並從 /etc/fstab \
                            移除置換空間的設定）"
            .to_owned(),
        Finding::SwapUnknown => "無法讀取 /proc/swaps".to_owned(),
        Finding::CrashNone => "核心沒有為 kdump 保留記憶體".to_owned(),
        Finding::CrashKept { mib } => format!(
            "核心為 kdump 保留了 {mib} MiB 記憶體，執行 sudo apt purge kdump-tools && sudo \
             update-grub 再重新開機就能釋放"
        ),
        Finding::OthersNone => "沒有找到".to_owned(),
        Finding::OthersFound { found } => {
            let named: Vec<String> = found
                .iter()
                .map(|l| format!("{} 的 {}", l.distribution, l.path))
                .collect();
            format!("找到 {}，如果仍在執行，先解除安裝", and_list(&named))
        }
        Finding::PortsFree { ports } => {
            let numbers: Vec<String> = ports.iter().map(u16::to_string).collect();
            format!("{} 都沒有程式使用", and_list(&numbers))
        }
        Finding::PortsBusy { ports } => {
            let named: Vec<String> = ports
                .iter()
                .map(|p| format!("{}（{}）", p.port, port_use(p.purpose)))
                .collect();
            format!(
                "已有程式使用 {}，用 `ss -ltnp` 找出是哪個程式",
                named.join("、")
            )
        }
        Finding::DiskUnknown { path } => format!("無法讀取 {path} 的可用空間"),
        Finding::DiskLow {
            free_tenths_gib,
            path,
            needed_gib,
        } => format!(
            "{path} 只剩 {} GiB 可用空間，叢集的映像檔和資料至少需要 {needed_gib} GiB",
            gib(*free_tenths_gib)
        ),
        Finding::DiskTight {
            free_tenths_gib,
            path,
            roomy_gib,
        } => format!(
            "{path} 有 {} GiB 可用空間，有 {roomy_gib} GiB 以上才能替映像檔、日誌和磁碟區預留空間",
            gib(*free_tenths_gib)
        ),
        Finding::DiskOk {
            free_tenths_gib,
            path,
        } => format!("{path} 有 {} GiB 可用空間", gib(*free_tenths_gib)),
        Finding::CryptYes { paths } => {
            let named: Vec<String> = paths.iter().map(|p| (*p).to_owned()).collect();
            let all = if paths.len() == 1 { "" } else { "都" };
            format!("{} {all}在 dm-crypt 裝置上", and_list(&named))
        }
        Finding::CryptRequiredPlain { path } => {
            format!("設定了 host.requireEncryptedDisk，但 {path} 不在加密裝置上")
        }
        Finding::CryptRequiredUnknown { path } => {
            format!("設定了 host.requireEncryptedDisk，但無法判斷 {path} 是否加密")
        }
        Finding::CryptNo { path } => format!(
            "{path} 不在加密磁碟上，磁碟區和資料庫都會以未加密的形式儲存。大多數 VPS \
             方案無法事後加密，要存放敏感資料，就選擇供應商會加密磁碟的方案。"
        ),
        Finding::CryptUnknown { path } => format!("無法判斷 {path} 是否加密"),
        Finding::DockerNone => "沒有 Docker，iptables 的 FORWARD 原則也不是 DROP".to_owned(),
        Finding::DockerFound { found } => format!(
            "這台主機上有 Docker（{}），Docker 會把 iptables 的 FORWARD 原則設為 DROP，Pod \
             就連不上網際網路。先停止並移除 Docker。",
            found.join("、")
        ),
        Finding::ForwardDrop => "iptables 的 FORWARD 原則是 DROP，Pod 連不上網際網路。找出設定這項\
                                 原則的程式（例如 Docker、ufw）再修改。"
            .to_owned(),
        Finding::NotInstalled => {
            "這台主機沒有安裝 K3s、RKE2 或 KubeSolo，用 `sudo zeatend apply` 安裝".to_owned()
        }
        Finding::ServiceRunning { unit } => format!("{unit} 正在執行"),
        Finding::ServiceStopped { unit } => format!(
            "{unit} 沒有執行，用 `sudo journalctl -u {unit} -n 100` 查看原因，再用 `sudo \
             systemctl restart {unit}` 重新啟動"
        ),
        Finding::ApiReady => "API 伺服器有回應 /readyz".to_owned(),
        Finding::ApiDown { unit } => format!(
            "API 伺服器 30 秒內沒有回應 /readyz，用 `sudo journalctl -u {unit} -n 100` 查看原因"
        ),
        Finding::RecordOk { writer } => format!("安裝紀錄由 {writer} 寫入"),
        Finding::RecordMissing { path } => format!(
            "{path} 沒有安裝紀錄，這個節點不是 zeatend 安裝的，或是 apply 沒有執行完。執行 `sudo \
             zeatend apply`。"
        ),
        Finding::RecordNewer { zeatend_version } => format!(
            "安裝紀錄由 zeatend {zeatend_version} 寫入，這裡的是 zeatend {ZEATEND_VERSION}。改用 \
             zeatend {zeatend_version} 或更新的版本。"
        ),
        Finding::NodesReady { names } => format!("{} {}已就緒", and_list(names), all(names)),
        Finding::NodesNotReady { names, command } => {
            format!("{} 尚未就緒，用 `{command}` 查看原因", and_list(names))
        }
        Finding::NodesNone { unit } => format!(
            "叢集沒有任何節點，用 `sudo journalctl -u {unit} -n 100` 查看 kubelet 為什麼沒有註冊"
        ),
        Finding::NodesUnknown { reason } => format!("無法讀取節點：{reason}"),
        Finding::ReservesSet { names } => format!(
            "{} 的 kubelet {}替系統保留了足夠的記憶體",
            and_list(names),
            all(names)
        ),
        Finding::ReservesLow {
            nodes,
            needed_mib,
            distribution,
        } => {
            let kept: Vec<String> = nodes
                .iter()
                .map(|k| format!("{} 只有 {} MiB", k.name, k.mib))
                .collect();
            format!(
                "kubelet 不讓 Pod 使用的記憶體：{}，少於系統閒置時需要的 {needed_mib} MiB，Pod \
                 可能用掉系統和 kubelet 需要的記憶體（沒有設定 system-reserved 或 \
                 kube-reserved）。{}。",
                kept.join("、"),
                reserves_fix(*distribution)
            )
        }
        Finding::ReservesUnknown { name } => {
            format!("節點 {name} 沒有回報記憶體容量或可分配的記憶體")
        }
        Finding::SecretsEncrypted { provider } => {
            format!("API 伺服器以加密形式儲存 Secret（{provider}）")
        }
        Finding::SecretsPlain { distribution } => format!(
            "API 伺服器以未加密形式儲存 Secret，用 `sudo {} secrets-encrypt status` 查看狀態",
            distribution.distribution().id()
        ),
        // Plan findings stay in English until zeatend#32 words them.
        Finding::MemoryLow { .. }
        | Finding::ReleaseUnpinned { .. }
        | Finding::ServerUnreachable { .. }
        | Finding::ServerFar { .. }
        | Finding::ServerTooFar { .. }
        | Finding::SameRegion
        | Finding::ServerPortsClosed { .. }
        | Finding::JoinPorts { .. }
        | Finding::UdpUnchecked { .. }
        | Finding::ServersHa
        | Finding::LocalStorage
        | Finding::TokenUnpinned => super::en::finding(finding),
    }
}

/// `都` before the verb, for a list of more than one item.
const fn all(names: &[String]) -> &'static str {
    if names.len() == 1 { "" } else { "都" }
}

/// How to set the kubelet's reserves on the nodes of `distribution`.
const fn reserves_fix(distribution: Distribution) -> &'static str {
    match distribution {
        Distribution::KubeSolo => "zeatend 目前還不會替 KubeSolo 設定（zeabur-oss/zeatend#4）",
        Distribution::K3s | Distribution::Rke2 => "在這些節點上執行 `sudo zeatend apply` 來設定",
    }
}

const fn port_use(port_use: PortUse) -> &'static str {
    match port_use {
        PortUse::KubernetesApi => "Kubernetes API",
        PortUse::K3sApiAndSupervisor => "Kubernetes API 和 K3s supervisor",
        PortUse::Rke2Supervisor => "RKE2 supervisor",
        PortUse::Kubelet => "kubelet",
        PortUse::EtcdClient => "etcd 用戶端",
        PortUse::EtcdPeer => "etcd 節點間通訊",
        PortUse::ServiceLbHttp => {
            "HTTP ingress，ServiceLB 在每個節點上替 LoadBalancer Service 占用這個連接埠"
        }
        PortUse::ServiceLbHttps => {
            "HTTPS ingress，ServiceLB 在每個節點上替 LoadBalancer Service 占用這個連接埠"
        }
        PortUse::KubeSoloHttp => {
            "HTTP ingress，KubeSolo 把這台主機的位址配置給 LoadBalancer Service"
        }
        PortUse::KubeSoloHttps => {
            "HTTPS ingress，KubeSolo 把這台主機的位址配置給 LoadBalancer Service"
        }
        PortUse::KubeSoloWebhook => "KubeSolo 指派節點的 webhook",
        PortUse::FlannelVxlan => "flannel VXLAN",
        PortUse::CanalVxlan => "Canal VXLAN",
    }
}
