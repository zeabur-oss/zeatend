//! Traditional Chinese, as written in Taiwan.

use zeatend_host::preflight::{Finding, Status, Topic};
use zeatend_plan::cluster::{Distribution, PortUse};
use zeatend_plan::record::ZEATEND_VERSION;

use super::{Messages, gib, zh_list};

pub(super) const MESSAGES: Messages = Messages {
    checking_host: "正在檢查主機",
    apply_stops: "修好失敗的項目之前，apply 不會繼續執行。",
    host_not_ready: "主機還沒準備好，修好失敗的項目後再執行一次 apply",
    doctor_needs_root: "用 root 身分執行 doctor（加上 sudo），檢查需要讀取叢集的檔案和憑證",
    status,
    topic,
    finding,
};

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
             方案無法事後加密，要存放敏感資料，就選擇供應商會加密磁碟的方案"
        ),
        Finding::CryptUnknown { path } => format!("無法判斷 {path} 是否加密"),
        Finding::DockerNone => "沒有 Docker，iptables 的 FORWARD 原則也不是 DROP".to_owned(),
        Finding::DockerFound { found } => format!(
            "這台主機上有 Docker（{}），Docker 會把 iptables 的 FORWARD 原則設為 DROP，Pod \
             就連不上網際網路。先停止並移除 Docker",
            found.join("、")
        ),
        Finding::ForwardDrop => "iptables 的 FORWARD 原則是 DROP，Pod 連不上網際網路。找出設定這項\
                                 原則的程式（例如 Docker、ufw）再修改"
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
        Finding::RecordOk { zeatend_version } => {
            format!("安裝紀錄由 zeatend {zeatend_version} 寫入")
        }
        Finding::RecordMissing { path } => format!(
            "{path} 沒有安裝紀錄，這個節點不是 zeatend 安裝的，或是 apply 沒有執行完。執行 `sudo \
             zeatend apply`"
        ),
        Finding::RecordNewer { zeatend_version } => format!(
            "安裝紀錄由 zeatend {zeatend_version} 寫入，這裡的是 zeatend {ZEATEND_VERSION}。改用 \
             zeatend {zeatend_version} 或更新的版本"
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
                 kube-reserved）。{}",
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
            distribution.id()
        ),
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
