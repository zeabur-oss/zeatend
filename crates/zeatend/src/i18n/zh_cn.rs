//! Simplified Chinese, as written in mainland China.

use zeatend_host::preflight::{Finding, Status, Topic};
use zeatend_plan::cluster::{Distribution, PortUse};
use zeatend_plan::record::ZEATEND_VERSION;

use super::{Messages, gib, zh_list};

pub(super) const MESSAGES: Messages = Messages {
    checking_host: "正在检查主机",
    apply_stops: "修复失败的项目之前，apply 不会继续运行。",
    host_not_ready: "主机还没准备好，修复失败的项目后再运行一次 apply",
    doctor_needs_root: "用 root 身份运行 doctor（加上 sudo），检查需要读取集群的文件和凭据",
    status,
    topic,
    finding,
};

const fn status(status: Status) -> &'static str {
    match status {
        Status::Pass => "通过",
        Status::Warn => "注意",
        Status::Fail => "失败",
    }
}

const fn topic(topic: Topic) -> &'static str {
    match topic {
        Topic::Root => "root 权限",
        Topic::Systemd => "systemd",
        Topic::Time => "时钟",
        Topic::Swap => "交换空间",
        Topic::CrashKernel => "kdump 内存",
        Topic::OtherKubernetes => "其他 Kubernetes",
        Topic::Ports => "端口",
        Topic::DiskSpace => "磁盘空间",
        Topic::DiskEncryption => "磁盘加密",
        Topic::Docker => "Docker",
        Topic::Service => "服务",
        Topic::Api => "Kubernetes API",
        Topic::InstallRecord => "安装记录",
        Topic::Nodes => "节点",
        Topic::KubeletReserves => "kubelet 预留资源",
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
        Finding::RootOk => "以 root 身份运行".to_owned(),
        Finding::RootNo => "用 root 身份运行 apply（加上 sudo）".to_owned(),
        Finding::SystemdOk => "init 系统是 systemd".to_owned(),
        Finding::SystemdNo => {
            "Kubernetes 以 systemd 服务运行，但这台主机没有使用 systemd".to_owned()
        }
        Finding::TimeOk => "时钟已同步".to_owned(),
        Finding::TimeDrift => "时钟没有同步，时间一偏移，证书和令牌就会失效（运行 \
                               timedatectl set-ntp true）"
            .to_owned(),
        Finding::TimeUnknown => "无法用 timedatectl 确认时钟是否同步".to_owned(),
        Finding::SwapOff => "交换空间已关闭".to_owned(),
        Finding::SwapOn => "交换空间已开启，kubelet 要求关闭（运行 swapoff -a，并从 /etc/fstab \
                            删除交换空间的配置）"
            .to_owned(),
        Finding::SwapUnknown => "无法读取 /proc/swaps".to_owned(),
        Finding::CrashNone => "内核没有为 kdump 预留内存".to_owned(),
        Finding::CrashKept { mib } => format!(
            "内核为 kdump 预留了 {mib} MiB 内存，运行 sudo apt purge kdump-tools && sudo \
             update-grub 再重启就能释放"
        ),
        Finding::OthersNone => "没有找到".to_owned(),
        Finding::OthersFound { found } => {
            let named: Vec<String> = found
                .iter()
                .map(|l| format!("{} 的 {}", l.distribution, l.path))
                .collect();
            format!("找到 {}，如果仍在运行，先卸载", and_list(&named))
        }
        Finding::PortsFree { ports } => {
            let numbers: Vec<String> = ports.iter().map(u16::to_string).collect();
            format!("{} 都没有程序占用", and_list(&numbers))
        }
        Finding::PortsBusy { ports } => {
            let named: Vec<String> = ports
                .iter()
                .map(|p| format!("{}（{}）", p.port, port_use(p.purpose)))
                .collect();
            format!(
                "已有程序占用 {}，用 `ss -ltnp` 查看是哪个程序",
                named.join("、")
            )
        }
        Finding::DiskUnknown { path } => format!("无法读取 {path} 的可用空间"),
        Finding::DiskLow {
            free_tenths_gib,
            path,
            needed_gib,
        } => format!(
            "{path} 只剩 {} GiB 可用空间，集群的镜像和数据至少需要 {needed_gib} GiB",
            gib(*free_tenths_gib)
        ),
        Finding::DiskTight {
            free_tenths_gib,
            path,
            roomy_gib,
        } => format!(
            "{path} 有 {} GiB 可用空间，有 {roomy_gib} GiB 以上才能为镜像、日志和存储卷预留空间",
            gib(*free_tenths_gib)
        ),
        Finding::DiskOk {
            free_tenths_gib,
            path,
        } => format!("{path} 有 {} GiB 可用空间", gib(*free_tenths_gib)),
        Finding::CryptYes { paths } => {
            let named: Vec<String> = paths.iter().map(|p| (*p).to_owned()).collect();
            let all = if paths.len() == 1 { "" } else { "都" };
            format!("{} {all}在 dm-crypt 设备上", and_list(&named))
        }
        Finding::CryptRequiredPlain { path } => {
            format!("设置了 host.requireEncryptedDisk，但 {path} 不在加密设备上")
        }
        Finding::CryptRequiredUnknown { path } => {
            format!("设置了 host.requireEncryptedDisk，但无法判断 {path} 是否加密")
        }
        Finding::CryptNo { path } => format!(
            "{path} 不在加密磁盘上，存储卷和数据库都会以未加密的形式保存。大多数 VPS \
             套餐无法事后加密，要存放敏感数据，就选择服务商会加密磁盘的套餐"
        ),
        Finding::CryptUnknown { path } => format!("无法判断 {path} 是否加密"),
        Finding::DockerNone => "没有 Docker，iptables 的 FORWARD 策略也不是 DROP".to_owned(),
        Finding::DockerFound { found } => format!(
            "这台主机上有 Docker（{}），Docker 会把 iptables 的 FORWARD 策略设为 DROP，Pod \
             就连不上互联网。先停止并卸载 Docker",
            found.join("、")
        ),
        Finding::ForwardDrop => "iptables 的 FORWARD 策略是 DROP，Pod 连不上互联网。找出设置这项\
                                 策略的程序（例如 Docker、ufw）再修改"
            .to_owned(),
        Finding::NotInstalled => {
            "这台主机没有安装 K3s、RKE2 或 KubeSolo，用 `sudo zeatend apply` 安装".to_owned()
        }
        Finding::ServiceRunning { unit } => format!("{unit} 正在运行"),
        Finding::ServiceStopped { unit } => format!(
            "{unit} 没有运行，用 `sudo journalctl -u {unit} -n 100` 查看原因，再用 `sudo \
             systemctl restart {unit}` 重新启动"
        ),
        Finding::ApiReady => "API 服务器有响应 /readyz".to_owned(),
        Finding::ApiDown { unit } => format!(
            "API 服务器 30 秒内没有响应 /readyz，用 `sudo journalctl -u {unit} -n 100` 查看原因"
        ),
        Finding::RecordOk { zeatend_version } => {
            format!("安装记录由 zeatend {zeatend_version} 写入")
        }
        Finding::RecordMissing { path } => format!(
            "{path} 没有安装记录，这个节点不是 zeatend 安装的，或者 apply 没有运行完。运行 `sudo \
             zeatend apply`"
        ),
        Finding::RecordNewer { zeatend_version } => format!(
            "安装记录由 zeatend {zeatend_version} 写入，这里的是 zeatend {ZEATEND_VERSION}。改用 \
             zeatend {zeatend_version} 或更新的版本"
        ),
        Finding::NodesReady { names } => format!("{} {}已就绪", and_list(names), all(names)),
        Finding::NodesNotReady { names, command } => {
            format!("{} 尚未就绪，用 `{command}` 查看原因", and_list(names))
        }
        Finding::NodesUnknown { reason } => format!("无法读取节点：{reason}"),
        Finding::ReservesSet { names } => format!(
            "{} 的 kubelet {}为系统预留了足够的内存",
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
                "kubelet 不让 Pod 使用的内存：{}，少于系统空闲时需要的 {needed_mib} MiB，Pod \
                 可能占用系统和 kubelet 需要的内存（没有设置 system-reserved 或 \
                 kube-reserved）。{}",
                kept.join("、"),
                reserves_fix(*distribution)
            )
        }
        Finding::ReservesUnknown { name } => {
            format!("节点 {name} 没有报告内存容量或可分配的内存")
        }
        Finding::SecretsEncrypted { provider } => {
            format!("API 服务器以加密形式存储 Secret（{provider}）")
        }
        Finding::SecretsPlain { distribution } => format!(
            "API 服务器以未加密形式存储 Secret，用 `sudo {} secrets-encrypt status` 查看状态",
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
        Distribution::KubeSolo => "zeatend 目前还不会为 KubeSolo 设置（zeabur-oss/zeatend#4）",
        Distribution::K3s | Distribution::Rke2 => "在这些节点上运行 `sudo zeatend apply` 来设置",
    }
}

const fn port_use(port_use: PortUse) -> &'static str {
    match port_use {
        PortUse::KubernetesApi => "Kubernetes API",
        PortUse::K3sApiAndSupervisor => "Kubernetes API 和 K3s supervisor",
        PortUse::Rke2Supervisor => "RKE2 supervisor",
        PortUse::Kubelet => "kubelet",
        PortUse::EtcdClient => "etcd 客户端",
        PortUse::EtcdPeer => "etcd 节点间通信",
        PortUse::ServiceLbHttp => {
            "HTTP ingress，ServiceLB 在每个节点上为 LoadBalancer Service 占用这个端口"
        }
        PortUse::ServiceLbHttps => {
            "HTTPS ingress，ServiceLB 在每个节点上为 LoadBalancer Service 占用这个端口"
        }
        PortUse::KubeSoloHttp => {
            "HTTP ingress，KubeSolo 把这台主机的地址分配给 LoadBalancer Service"
        }
        PortUse::KubeSoloHttps => {
            "HTTPS ingress，KubeSolo 把这台主机的地址分配给 LoadBalancer Service"
        }
        PortUse::KubeSoloWebhook => "KubeSolo 分配节点的 webhook",
        PortUse::FlannelVxlan => "flannel VXLAN",
        PortUse::CanalVxlan => "Canal VXLAN",
    }
}
