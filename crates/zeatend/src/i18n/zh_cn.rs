//! Simplified Chinese, as written in mainland China.

use zeatend_plan::cluster::{Distribution, PortUse};
use zeatend_plan::finding::{Finding, Status, Topic};
use zeatend_plan::longhorn::{MODULES_FILE, MULTIPATH_FILE, Need};
use zeatend_plan::precondition::Stale;
use zeatend_plan::record::ZEATEND_VERSION;
use zeatend_plan::steps::Step;

use super::{Joined, Key, Messages, Ready, Screen, Screens, gib, zh_list};

pub(super) const MESSAGES: Messages = Messages {
    checking_host: "正在检查主机",
    apply_stops: "修复失败的项目之前，apply 不会继续运行。",
    host_not_ready: "主机还没准备好，修复失败的项目后再运行一次 apply",
    not_plan: |path, reason| {
        format!("没有做任何更改：{path} 不是 `zeatend plan --json` 输出的计划（{reason}）")
    },
    stale,
    doctor_needs_root: "用 root 身份运行 doctor（加上 sudo），检查需要读取集群的文件和凭据",
    no_config_installer,
    no_config_defaults,
    following_unit,
    started_unit,
    telemetry_on: "zeatend 会发送版本、发行版和安装是否成功，帮助改进 Zeatend。加上 \
                   --no-telemetry 或设置 DO_NOT_TRACK=1 即可关闭。会发送什么：\
                   https://oss.zeabur.com/zh-CN/docs/zeatend/reference/telemetry/",
    starting_unit,
    restarting_unit,
    waiting_for_node,
    step,
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
        Screen::Server => "服务器",
        Screen::Access => "访问方式",
        Screen::Review => "确认",
        Screen::Install => "安装",
    },
    key: |key| match key {
        Key::Choose => "选择",
        Key::Next => "继续",
        Key::Back => "上一步",
        Key::Field => "下一项",
        Key::Language => "语言",
        Key::Quit => "退出",
        Key::Install => "开始安装",
        Key::Leave => "关闭界面，安装会继续",
        Key::Close => "关闭",
    },

    server_title: "这台服务器",
    host_name: "主机名",
    system: "系统",
    cpu: "CPU",
    memory: "内存",
    addresses: "地址",
    cpus: |n| format!("{n} 核 CPU"),
    private_address: |address| format!("{address}（内网）"),
    no_address: "没有",
    distribution_title: "要安装哪种 Kubernetes",
    distribution_about: |distribution| match distribution {
        Distribution::KubeSolo => "只有一个节点，最省资源，不能加入工作节点。",
        Distribution::K3s => "轻量，参照 RKE2 的 CIS 配置加固，以后可以加入工作节点。",
        Distribution::Rke2 => "使用 RKE2 的 CIS 配置，适合三台服务器组成的控制平面。",
    },
    checks_title: "主机检查",
    checks_summary: |failed, warned| match (failed, warned) {
        (0, 0) => "全部通过".to_owned(),
        (0, w) => format!("{w} 项要注意"),
        (f, 0) => format!("{f} 项失败"),
        (f, w) => format!("{f} 项失败，{w} 项要注意"),
    },
    checks_stop: "修复失败的项目，再运行一次 sudo zeatend。",

    access_title: "你的电脑怎么连到集群",
    access_private: "这台服务器只有内网地址。你的电脑用哪个地址连到它？",
    access_public: "你的电脑用这个地址连到 API 服务器。如果用的是别的地址，请修改。",
    field_address: "地址",
    field_address_hint: "API 服务器的证书会包含这个地址，kubectl 才能连接。",
    field_name: "DNS 名称（可不填）",
    field_name_hint: "指向这台服务器的名称，证书也会包含它。",
    field_node: "节点名称（可不填）",
    field_node_hint: |name| format!("不填的话，节点名称是 {name}。"),
    bad_address: "请输入 IP 地址，例如 203.0.113.7。",
    bad_name: "请输入 DNS 名称，例如 k8s.example.com。",
    bad_node: "请用小写英文字母、数字、- 和 .，开头和结尾要是字母或数字。",

    review_title: "安装前确认",
    review_release: "Kubernetes",
    review_address: "地址",
    review_name: "DNS 名称",
    review_no_name: "没有填，kubectl 用地址连接",
    review_node: "节点名称",
    review_config: "配置文件",
    review_next: "接下来会做什么",
    next_write: |path| format!("zeatend 把这些选择写入 {path}。"),
    next_update: |path| format!("zeatend 把这些选择保存到 {path}，其他配置不变。"),
    next_unit: "安装在 systemd 单元中运行，SSH 连接断开也会继续。",
    next_follow: "再运行一次 sudo zeatend 就能继续查看进度。",
    next_telemetry: "安装时会发送版本、发行版和安装是否成功，帮助改进 Zeatend。要关闭，\
                     先退出，再运行 sudo zeatend --no-telemetry。",

    install_title: "正在安装",
    install_unit: |unit| format!("在 {unit} 中运行"),
    install_elapsed: |time| format!("已用时 {time}"),
    install_starting: "正在开始安装",
    install_done: "集群已经可以使用。",
    install_failed: "安装停止了。",
    duration: |minutes, seconds| {
        if minutes == 0 {
            format!("{seconds} 秒")
        } else {
            format!("{minutes} 分 {seconds} 秒")
        }
    },
    log_title: "日志",
    result_title: "接下来",
    result_more: "按 Enter 关闭界面后，终端会打印出完整内容。",

    language_title: "语言",
    language_hint: "选了马上切换。",
    language_needs_utf8: "终端需要支持 UTF-8",

    reading_host: "正在读取这台主机的信息",
    left: |unit| format!("安装会在 {unit} 中继续。运行 `sudo zeatend` 可以再次查看进度。"),
    no_terminal: "zeatend 要在终端中打开界面。没有终端的话，请写好 \
                  /etc/zeatend/config.yaml，再运行：\n  sudo zeatend plan\n  sudo zeatend apply",
    needs_root: "用 root 身份运行 zeatend：sudo zeatend",
    needs_systemd: "这台主机没有使用 systemd，无法打开界面。请写好 /etc/zeatend/config.yaml，\
                    再运行 sudo zeatend apply --foreground",
    joins: |path| format!("{path} 会让这台主机加入集群，请运行 sudo zeatend apply"),
};

const DONE: &str = "完成";

fn no_config_installer(config: &str, installer: &str) -> String {
    format!("找不到配置文件 {config}，改用 zeabur-installer 写在 {installer} 的发行版和主机设置。")
}

fn no_config_defaults(config: &str) -> String {
    format!("找不到配置文件 {config}，改用默认值：在这台主机上安装 K3s。")
}

fn following_unit(unit: &str) -> String {
    format!("{unit} 已经在安装，接着显示它的进度。")
}

fn started_unit(unit: &str) -> String {
    format!("安装在 {unit} 里运行。这个会话结束后，运行 `sudo zeatend apply` 就能再看到进度。")
}

fn starting_unit(unit: &str) -> String {
    format!("正在启动 {unit}，第一次启动要几分钟")
}

fn restarting_unit(unit: &str) -> String {
    format!("正在重启 {unit}，应用新的配置")
}

fn waiting_for_node(node: &str) -> String {
    format!("节点 {node}")
}

const fn step(step: Step) -> &'static str {
    match step {
        Step::CheckHost => "正在检查主机",
        Step::PrepareLonghorn => "正在为 Longhorn 准备这个节点",
        Step::Download => "正在下载发行版",
        Step::WriteFiles => "正在写入配置文件",
        Step::PrepareHost => "正在准备主机",
        Step::InstallRelease => "正在安装发行版",
        Step::StartService => "正在启动服务",
        Step::WaitReady => "正在等待节点就绪",
        Step::WaitRemoved => "正在等待已关闭的组件移除完毕",
        Step::ProtectCertificates => "正在把证书改为仅 root 可读",
        Step::WaitAddons => "正在等待 CoreDNS 和 local-path 存储启动",
        Step::WriteRecord => "正在写入安装记录",
        Step::WriteKubeconfig => "正在写入 kubeconfig",
    }
}

fn ready(r: &Ready<'_>) -> String {
    let host = r.host.unwrap_or("<服务器地址>");
    let node = r.node;
    let sed = r.rewrite.map_or_else(String::new, |(from, to)| {
        format!("  sed -i.bak 's#{from}#{to}#' ~/.kube/{node}.yaml\n")
    });
    let mut text = format!(
        "{} 已经在 {node} 上运行。\n\n在这台服务器上列出节点：\n  {} get nodes -o wide\n\n\
         在你自己的电脑上使用这个集群：\n  \
         mkdir -p ~/.kube\n  (umask 077; ssh <用户名>@{host} sudo cat {} > ~/.kube/{node}.yaml)\n{sed}  KUBECONFIG=~/.kube/{node}.yaml kubectl get nodes\n\n\
         拿到这个文件的人就是集群管理员，要像 SSH 密钥一样保管好。",
        r.cluster, r.kubectl, r.kubeconfig
    );
    if r.rewrite.is_none() {
        text.push_str(
            "如果你的电脑要用其他地址连到服务器，改掉文件里的 `server:`，并把这个地址加进 \
             host.tlsSan。",
        );
    }
    if r.add_nodes {
        text.push_str(
            "\n\n要加入节点，在这台服务器上运行 `sudo zeatend node add`，它会打印出新主机要运行的命令。",
        );
    }
    text
}

fn joined(j: &Joined<'_>) -> String {
    let role = if j.server {
        "服务器"
    } else {
        "工作节点"
    };
    format!(
        "{} 已经作为{role}加入集群。在服务器上列出节点：\n  {} get nodes -o wide",
        j.node, j.kubectl
    )
}

fn stale(stale: &Stale) -> String {
    let why = match stale {
        Stale::Writes => {
            "配置文件、这台主机或 zeatend 发生了变化，现在 apply 要做的事和文件里的计划不一样"
                .to_owned()
        }
        Stale::File(path) => format!("生成计划之后，{path} 被改过了"),
        Stale::Release { reviewed, now } => format!(
            "生成计划时安装的版本是 {}，现在是 {}",
            reviewed.as_deref().unwrap_or("（没有）"),
            now.as_deref().unwrap_or("（没有）")
        ),
        Stale::Record => "生成计划之后，安装记录发生了变化".to_owned(),
    };
    format!("没有做任何更改：{why}。请重新运行 `zeatend plan --json`，看过新的计划再应用。")
}

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
        Topic::Storage => "存储",
        Topic::Downloads => "下载",
        Topic::Mirrors => "镜像源",
        Topic::NodeName => "节点名称",
        // Plan findings stay in English until zeatend#32 words them.
        Topic::Memory
        | Topic::Release
        | Topic::Server
        | Topic::Firewall
        | Topic::HighAvailability
        | Topic::JoinToken => super::en::topic(topic),
    }
}

/// Something a node lacks for Longhorn.
fn need(need: &Need) -> String {
    match need {
        Need::Package { program, apt, dnf } => {
            format!("{program}（软件包 {apt}，RHEL 上是 {dnf}）")
        }
        Need::ModulesFile => format!("开机时加载模块的 {MODULES_FILE}"),
        Need::Module { module } => format!("{module} 模块"),
        Need::Iscsid => "已启用并在运行的 iscsid".to_owned(),
        Need::MultipathBlacklist => format!("让 multipathd 避开 Longhorn 磁盘的 {MULTIPATH_FILE}"),
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
             套餐无法事后加密，要存放敏感数据，就选择服务商会加密磁盘的套餐。"
        ),
        Finding::CryptUnknown { path } => format!("无法判断 {path} 是否加密"),
        Finding::DockerNone => "没有 Docker，iptables 的 FORWARD 策略也不是 DROP".to_owned(),
        Finding::DockerFound { found } => format!(
            "这台主机上有 Docker（{}），Docker 会把 iptables 的 FORWARD 策略设为 DROP，Pod \
             就连不上互联网。先停止并卸载 Docker。",
            found.join("、")
        ),
        Finding::ForwardDrop => "iptables 的 FORWARD 策略是 DROP，Pod 连不上互联网。找出设置这项\
                                 策略的程序（例如 Docker、ufw）再修改。"
            .to_owned(),
        Finding::NotInstalled => {
            "这台主机没有安装 K3s、RKE2 或 KubeSolo，用 `sudo zeatend apply` 安装".to_owned()
        }
        Finding::DownloadsOk { url } => format!("{url} 可以连接"),
        Finding::DownloadsNo { url, reason } => format!(
            "{url} 连不上（{reason}），按 \
             https://oss.zeabur.com/zh-CN/docs/zeatend/get-started/without-github/ 在配置文件里设置 proxy.https 代理"
        ),
        Finding::MirrorOk { endpoint } => format!("镜像源 {endpoint} 可以连接"),
        Finding::MirrorNo { endpoint, reason } => {
            format!(
                "镜像源 {endpoint} 连不上（{reason}），镜像会改从下一个镜像源或镜像仓库本身拉取"
            )
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
        Finding::RecordOk { writer } => format!("安装记录由 {writer} 写入"),
        Finding::RecordMissing { path } => format!(
            "{path} 没有安装记录，这个节点不是 zeatend 安装的，或者 apply 没有运行完。运行 `sudo \
             zeatend apply`。"
        ),
        Finding::RecordNewer { zeatend_version } => format!(
            "安装记录由 zeatend {zeatend_version} 写入，这里的是 zeatend {ZEATEND_VERSION}。改用 \
             zeatend {zeatend_version} 或更新的版本。"
        ),
        Finding::NodesReady { names } => format!("{} {}已就绪", and_list(names), all(names)),
        Finding::NodesNotReady { names, command } => {
            format!("{} 尚未就绪，用 `{command}` 查看原因", and_list(names))
        }
        Finding::NodesNone { unit } => format!(
            "集群没有任何节点，用 `sudo journalctl -u {unit} -n 100` 查看 kubelet 为什么没有注册"
        ),
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
                 kube-reserved）。{}。",
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
            distribution.distribution().id()
        ),
        Finding::LonghornReady => {
            "这个节点已有 Longhorn 需要的 iSCSI initiator、NFS 客户端、内核模块和 iscsid".to_owned()
        }
        Finding::LonghornMissing { needs } => format!(
            "这个节点缺少{}，Longhorn 无法在这里连接卷。请运行 `sudo zeatend apply`",
            and_list(&needs.iter().map(need).collect::<Vec<_>>())
        ),
        Finding::NodeNameLocalhost {
            distribution: Distribution::KubeSolo,
        } => "节点会命名为 localhost，从这个名称看不出警告说的是哪台机器。KubeSolo 用主机名作为节点名称，请在安装前修改主机名：`sudo hostnamectl set-hostname <名称>`。".to_owned(),
        Finding::NodeNameLocalhost { .. } => "节点会命名为 localhost，从这个名称看不出警告说的是哪台机器。用 host.nodeName 或“访问方式”页面的“节点名称”字段另取名称。".to_owned(),
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
