//! Traditional Chinese, as written in Taiwan.

use zeatend_host::preflight::{Finding, Status, Topic};
use zeatend_plan::cluster::PortUse;

use super::{Messages, gib, zh_list};

pub(super) const MESSAGES: Messages = Messages {
    checking_host: "正在檢查主機",
    apply_stops: "修好失敗的項目之前，apply 不會繼續執行。",
    host_not_ready: "主機還沒準備好；修好失敗的項目後，再執行一次 apply",
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
    }
}

fn and_list(items: &[String]) -> String {
    zh_list(items, "和")
}

fn finding(finding: &Finding) -> String {
    match finding {
        Finding::RootOk => "以 root 身分執行".to_owned(),
        Finding::RootNo => "用 root 身分執行 apply（加上 sudo）".to_owned(),
        Finding::SystemdOk => "init 系統是 systemd".to_owned(),
        Finding::SystemdNo => {
            "Kubernetes 以 systemd 服務執行，但這台主機沒有使用 systemd".to_owned()
        }
        Finding::TimeOk => "時鐘已同步".to_owned(),
        Finding::TimeDrift => "時鐘沒有同步；時間一偏移，憑證和權杖就會失效（執行 \
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
            "核心為 kdump 保留了 {mib} MiB 記憶體；要釋放這些記憶體，執行 sudo apt purge \
             kdump-tools && sudo update-grub，再重新開機"
        ),
        Finding::OthersNone => "沒有找到".to_owned(),
        Finding::OthersFound { found } => {
            let named: Vec<String> = found
                .iter()
                .map(|l| format!("{}（{}）", l.distribution, l.path))
                .collect();
            format!(
                "找到其他發行版的檔案：{}；如果仍在執行，先解除安裝",
                named.join("、")
            )
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
                "已有程式使用 {}；用 `ss -ltnp` 找出是哪個程式",
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
            "{path} 有 {} GiB 可用空間；{roomy_gib} GiB 以上可為映像檔、日誌和磁碟區預留空間",
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
            "{path} 不在加密磁碟上，磁碟區和資料庫都會以未加密的形式儲存；大多數 VPS \
             方案無法事後加密，要存放敏感資料，就選擇供應商會加密磁碟的方案"
        ),
        Finding::CryptUnknown { path } => format!("無法判斷 {path} 是否加密"),
        Finding::DockerNone => "沒有 Docker，iptables 的 FORWARD 原則也不是 DROP".to_owned(),
        Finding::DockerFound { found } => format!(
            "這台主機上有 Docker（{}），Docker 會把 iptables 的 FORWARD 原則設為 DROP，Pod \
             就連不上網際網路；先停止並移除 Docker",
            found.join("、")
        ),
        Finding::ForwardDrop => "iptables 的 FORWARD 原則是 DROP，Pod 連不上網際網路；找出設定這項\
                                 原則的程式（例如 Docker、ufw）再修改"
            .to_owned(),
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
