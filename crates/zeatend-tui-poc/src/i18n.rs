//! Messages as typed Rust values, one per language, and the language and terminal checks.

/// A language the installer ships strings for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Locale {
    En,
    ZhTw,
    ZhCn,
}

/// Every language, in the order the picker lists them.
pub(crate) const LOCALES: [Locale; 3] = [Locale::En, Locale::ZhTw, Locale::ZhCn];

impl Locale {
    pub(crate) const fn tag(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhTw => "zh-TW",
            Self::ZhCn => "zh-CN",
        }
    }

    /// The name of the language in that language; never translated.
    pub(crate) const fn native_name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::ZhTw => "繁體中文",
            Self::ZhCn => "简体中文",
        }
    }

    pub(crate) const fn messages(self) -> &'static Messages {
        match self {
            Self::En => &EN,
            Self::ZhTw => &ZH_TW,
            Self::ZhCn => &ZH_CN,
        }
    }

    /// Reads a POSIX locale name such as `zh_TW.UTF-8` or a tag such as `zh-Hant`.
    pub(crate) fn from_locale(value: &str) -> Option<Self> {
        let name = value
            .split(['.', '@'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let name = name.replace('-', "_");
        if name.starts_with("zh") {
            for traditional in ["_tw", "_hk", "_mo", "_hant"] {
                if name.contains(traditional) {
                    return Some(Self::ZhTw);
                }
            }
            return Some(Self::ZhCn);
        }
        if name.starts_with("en") {
            return Some(Self::En);
        }
        None
    }

    pub(crate) fn from_tag(tag: &str) -> Option<Self> {
        LOCALES
            .into_iter()
            .find(|lang| lang.tag().eq_ignore_ascii_case(tag))
    }
}

/// Reads an environment variable the way the C library does: empty means unset.
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The language from `LC_ALL`, then `LC_MESSAGES`, then `LANG`; English when none match.
pub(crate) fn locale_from_env() -> Locale {
    for name in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(value) = env(name) {
            return Locale::from_locale(&value).unwrap_or(Locale::En);
        }
    }
    Locale::En
}

/// Why the terminal cannot show Chinese, if it cannot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoCjk {
    /// `TERM=linux`: the Linux virtual console font has 512 glyphs and no CJK.
    LinuxConsole,
    /// The character set from `LC_ALL`, `LC_CTYPE` or `LANG` is not UTF-8 (or is unset, `C`,
    /// `POSIX`).
    NotUtf8,
}

/// Checks whether the terminal can show CJK text. A terminal emulator over SSH
/// passes `TERM` and, with `SendEnv LANG LC_*`, the locale.
pub(crate) fn cjk_blocker(term: Option<&str>, ctype: Option<&str>) -> Option<NoCjk> {
    if term == Some("linux") {
        return Some(NoCjk::LinuxConsole);
    }
    let charset = ctype.unwrap_or("").to_ascii_lowercase().replace('-', "");
    if charset.contains("utf8") {
        None
    } else {
        Some(NoCjk::NotUtf8)
    }
}

pub(crate) fn cjk_blocker_from_env() -> Option<NoCjk> {
    let term = env("TERM");
    let mut ctype = None;
    for name in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Some(value) = env(name) {
            ctype = Some(value);
            break;
        }
    }
    cjk_blocker(term.as_deref(), ctype.as_deref())
}

/// Every message of the installer. A language that lacks one does not compile.
pub(crate) struct Messages {
    pub(crate) app_title: &'static str,

    pub(crate) step_server: &'static str,
    pub(crate) step_access: &'static str,
    pub(crate) step_review: &'static str,
    pub(crate) step_install: &'static str,

    pub(crate) key_choose: &'static str,
    pub(crate) key_next: &'static str,
    pub(crate) key_back: &'static str,
    pub(crate) key_field: &'static str,
    pub(crate) key_lang: &'static str,
    pub(crate) key_quit: &'static str,
    pub(crate) key_install: &'static str,

    pub(crate) server_title: &'static str,
    pub(crate) server_host: &'static str,
    pub(crate) server_system: &'static str,
    pub(crate) server_cpu: &'static str,
    pub(crate) server_memory: &'static str,
    pub(crate) server_disk: &'static str,
    pub(crate) server_address: &'static str,

    pub(crate) cpu_cores: fn(u32) -> String,

    pub(crate) memory_gb: fn(u32) -> String,

    pub(crate) disk_free: fn(&str, &str) -> String,

    pub(crate) addr_private: fn(&str) -> String,

    pub(crate) distro_title: &'static str,
    pub(crate) distro_kubesolo: &'static str,
    pub(crate) distro_k3s: &'static str,
    pub(crate) distro_rke2: &'static str,

    pub(crate) checks_title: &'static str,
    pub(crate) checks_summary: fn(u32) -> String,

    pub(crate) check_ports: &'static str,
    pub(crate) check_swap: &'static str,

    pub(crate) access_title: &'static str,
    pub(crate) access_private_note: &'static str,

    pub(crate) field_address: &'static str,
    pub(crate) field_domain: &'static str,
    pub(crate) field_domain_hint: &'static str,
    pub(crate) field_name: &'static str,
    pub(crate) field_name_hint: &'static str,

    pub(crate) review_title: &'static str,
    pub(crate) review_distro: &'static str,
    pub(crate) review_address: &'static str,
    pub(crate) review_domain: &'static str,
    pub(crate) review_name: &'static str,
    pub(crate) review_no_domain: &'static str,
    pub(crate) review_plan_title: &'static str,

    pub(crate) plan_download: fn(&str) -> String,
    pub(crate) plan_disk: fn(&str) -> String,
    pub(crate) plan_time: fn(u32) -> String,
    pub(crate) plan_restart: &'static str,

    pub(crate) install_title: &'static str,
    pub(crate) install_elapsed: fn(&str) -> String,
    pub(crate) install_left: fn(&str) -> String,
    pub(crate) install_done: fn(&str) -> String,
    pub(crate) install_downloaded: fn(&str, &str) -> String,

    pub(crate) duration: fn(u32, &str) -> String,

    pub(crate) install_step_check: &'static str,
    pub(crate) install_step_download: &'static str,
    pub(crate) install_step_k3s: &'static str,
    pub(crate) install_step_network: &'static str,
    pub(crate) install_step_ready: &'static str,

    pub(crate) log_title: &'static str,

    pub(crate) lang_title: &'static str,
    pub(crate) lang_hint: &'static str,
    pub(crate) lang_needs_utf8: &'static str,

    pub(crate) fallback_note: &'static str,
}

const EN: Messages = Messages {
    app_title: "Zeatend",

    step_server: "Server",
    step_access: "Access",
    step_review: "Review",
    step_install: "Install",

    key_choose: "choose",
    key_next: "continue",
    key_back: "back",
    key_field: "next field",
    key_lang: "language",
    key_quit: "quit",
    key_install: "install",

    server_title: "This server",
    server_host: "Hostname",
    server_system: "System",
    server_cpu: "CPU",
    server_memory: "Memory",
    server_disk: "Disk",
    server_address: "Address",

    cpu_cores: |n| {
        if n == 1 {
            "1 core".to_owned()
        } else {
            format!("{n} cores")
        }
    },

    memory_gb: |gb| format!("{gb} GB"),

    disk_free: |free, total| format!("{free} GB free of {total} GB"),

    addr_private: |addr| format!("{addr} (private)"),

    distro_title: "Which Kubernetes to install",
    distro_kubesolo: "Single node and the lightest. For small servers with 1 to 2 GB of memory.",
    distro_k3s: "Light and widely used. You can add more servers later.",
    distro_rke2: "Hardened for security rules such as CIS. Needs at least 4 GB of memory.",

    checks_title: "Checks",
    checks_summary: |n| match n {
        0 => "Everything looks good".to_owned(),
        1 => "1 thing to know".to_owned(),
        _ => format!("{n} things to know"),
    },

    check_ports: "Ports 80, 443 and 6443 are free",
    check_swap: "Swap is on. Kubernetes runs better without it, so the installer turns it off.",

    access_title: "How your computer reaches the cluster",
    access_private_note: "This server only sees a private address. Which address does your computer use to reach it?",

    field_address: "Address",
    field_domain: "Name for the address (optional)",
    field_domain_hint: "Added to the API certificate, so kubectl can use the name.",
    field_name: "Cluster name",
    field_name_hint: "Shown as the context name in your kubeconfig.",

    review_title: "Check before installing",
    review_distro: "Kubernetes",
    review_address: "Address",
    review_domain: "Domain",
    review_name: "Cluster name",
    review_no_domain: "None, kubectl uses the address",
    review_plan_title: "What happens next",

    plan_download: |gb| format!("Downloads about {gb} GB"),
    plan_disk: |gb| format!("Uses about {gb} GB of disk"),
    plan_time: |min| {
        if min == 1 {
            "Takes about 1 minute".to_owned()
        } else {
            format!("Takes about {min} minutes")
        }
    },
    plan_restart: "The server does not restart.",

    install_title: "Installing",
    install_elapsed: |time| format!("Elapsed {time}"),
    install_left: |time| format!("About {time} left"),
    install_done: |path| format!("The cluster is ready. kubeconfig saved to {path}."),
    install_downloaded: |done, total| format!("{done} of {total} MB"),

    duration: |m, s| {
        if m == 0 {
            format!("{s} s")
        } else {
            format!("{m} min {s} s")
        }
    },

    install_step_check: "Check the server",
    install_step_download: "Download K3s and images",
    install_step_k3s: "Start K3s",
    install_step_network: "Set up the network",
    install_step_ready: "Wait for the node to be Ready",

    log_title: "Log",

    lang_title: "Language",
    lang_hint: "Changes right away.",
    lang_needs_utf8: "needs a UTF-8 terminal",

    fallback_note: "This terminal cannot show Chinese, so Zeatend uses English.",
};

const ZH_TW: Messages = Messages {
    app_title: "Zeatend",

    step_server: "伺服器",
    step_access: "連線方式",
    step_review: "確認",
    step_install: "安裝",

    key_choose: "選擇",
    key_next: "繼續",
    key_back: "上一步",
    key_field: "下一欄",
    key_lang: "語言",
    key_quit: "離開",
    key_install: "開始安裝",

    server_title: "這台伺服器",
    server_host: "主機名稱",
    server_system: "系統",
    server_cpu: "CPU",
    server_memory: "記憶體",
    server_disk: "磁碟",
    server_address: "位址",

    cpu_cores: |n| format!("{n} 核心"),

    memory_gb: |gb| format!("{gb} GB"),

    disk_free: |free, total| format!("還有 {free} GB 可用，共 {total} GB"),

    addr_private: |addr| format!("{addr}（內網）"),

    distro_title: "要裝哪一種 Kubernetes",
    distro_kubesolo: "只有一台機器，最省資源。適合記憶體只有 1 到 2 GB 的小主機。",
    distro_k3s: "輕巧、用的人多，之後還能再加伺服器。",
    distro_rke2: "預先照 CIS 等資安規範設定好，記憶體至少要 4 GB。",

    checks_title: "檢查結果",
    checks_summary: |n| {
        if n == 0 {
            "一切正常".to_owned()
        } else {
            format!("有 {n} 件事要留意")
        }
    },

    check_ports: "80、443、6443 連接埠都沒被占用",
    check_swap: "這台有開 swap。Kubernetes 不開 swap 跑得比較好，安裝程式會幫你關掉。",

    access_title: "你的電腦要怎麼連到叢集",
    access_private_note: "這台伺服器只看得到內網位址。你的電腦是用哪個位址連到它？",

    field_address: "位址",
    field_domain: "位址的名稱（可不填）",
    field_domain_hint: "會加進 API 憑證，kubectl 才能用這個名稱連線。",
    field_name: "叢集名稱",
    field_name_hint: "會當成 kubeconfig 裡的 context 名稱。",

    review_title: "安裝前確認一下",
    review_distro: "Kubernetes",
    review_address: "位址",
    review_domain: "網域",
    review_name: "叢集名稱",
    review_no_domain: "沒有填，kubectl 用位址連線",
    review_plan_title: "接下來會做什麼",

    plan_download: |gb| format!("下載大約 {gb} GB"),
    plan_disk: |gb| format!("用掉大約 {gb} GB 磁碟空間"),
    plan_time: |min| format!("大約要 {min} 分鐘"),
    plan_restart: "伺服器不會重新開機。",

    install_title: "正在安裝",
    install_elapsed: |time| format!("已經過了 {time}"),
    install_left: |time| format!("大約還要 {time}"),
    install_done: |path| format!("叢集已經可以用了。kubeconfig 存在 {path}。"),
    install_downloaded: |done, total| format!("{done} / {total} MB"),

    duration: |m, s| {
        if m == 0 {
            format!("{s} 秒")
        } else {
            format!("{m} 分 {s} 秒")
        }
    },

    install_step_check: "檢查伺服器",
    install_step_download: "下載 K3s 和映像檔",
    install_step_k3s: "啟動 K3s",
    install_step_network: "設定網路",
    install_step_ready: "等節點變成 Ready",

    log_title: "紀錄",

    lang_title: "語言",
    lang_hint: "選了馬上換。",
    lang_needs_utf8: "終端機要支援 UTF-8",

    fallback_note: "這個終端機顯示不了中文，所以 Zeatend 改用英文。",
};

const ZH_CN: Messages = Messages {
    app_title: "Zeatend",

    step_server: "服务器",
    step_access: "访问方式",
    step_review: "确认",
    step_install: "安装",

    key_choose: "选择",
    key_next: "继续",
    key_back: "上一步",
    key_field: "下一项",
    key_lang: "语言",
    key_quit: "退出",
    key_install: "开始安装",

    server_title: "这台服务器",
    server_host: "主机名",
    server_system: "系统",
    server_cpu: "CPU",
    server_memory: "内存",
    server_disk: "磁盘",
    server_address: "地址",

    cpu_cores: |n| format!("{n} 核"),

    memory_gb: |gb| format!("{gb} GB"),

    disk_free: |free, total| format!("还有 {free} GB 可用，共 {total} GB"),

    addr_private: |addr| format!("{addr}（内网）"),

    distro_title: "要装哪种 Kubernetes",
    distro_kubesolo: "只有一台机器，最省资源。适合内存只有 1 到 2 GB 的小主机。",
    distro_k3s: "轻量、用的人多，以后还能再加服务器。",
    distro_rke2: "预先按 CIS 等安全规范配置好，内存至少要 4 GB。",

    checks_title: "检查结果",
    checks_summary: |n| {
        if n == 0 {
            "一切正常".to_owned()
        } else {
            format!("有 {n} 件事要注意")
        }
    },

    check_ports: "80、443、6443 端口都没被占用",
    check_swap: "这台开了 swap。Kubernetes 不开 swap 跑得更好，安装程序会帮你关掉。",

    access_title: "你的电脑要怎么连到集群",
    access_private_note: "这台服务器只能看到内网地址。你的电脑是用哪个地址连到它的？",

    field_address: "地址",
    field_domain: "地址的名称（可不填）",
    field_domain_hint: "会加进 API 证书，kubectl 才能用这个名称连接。",
    field_name: "集群名称",
    field_name_hint: "会当成 kubeconfig 里的 context 名称。",

    review_title: "安装前确认一下",
    review_distro: "Kubernetes",
    review_address: "地址",
    review_domain: "域名",
    review_name: "集群名称",
    review_no_domain: "没有填，kubectl 用地址连接",
    review_plan_title: "接下来会做什么",

    plan_download: |gb| format!("下载大约 {gb} GB"),
    plan_disk: |gb| format!("占用大约 {gb} GB 磁盘空间"),
    plan_time: |min| format!("大约要 {min} 分钟"),
    plan_restart: "服务器不会重启。",

    install_title: "正在安装",
    install_elapsed: |time| format!("已用时 {time}"),
    install_left: |time| format!("大约还要 {time}"),
    install_done: |path| format!("集群已经可以用了。kubeconfig 存在 {path}。"),
    install_downloaded: |done, total| format!("{done} / {total} MB"),

    duration: |m, s| {
        if m == 0 {
            format!("{s} 秒")
        } else {
            format!("{m} 分 {s} 秒")
        }
    },

    install_step_check: "检查服务器",
    install_step_download: "下载 K3s 和镜像",
    install_step_k3s: "启动 K3s",
    install_step_network: "配置网络",
    install_step_ready: "等节点变成 Ready",

    log_title: "日志",

    lang_title: "语言",
    lang_hint: "选了马上切换。",
    lang_needs_utf8: "终端需要支持 UTF-8",

    fallback_note: "这个终端显示不了中文，所以 Zeatend 改用英文。",
};

/// A number with comma thousands separators. All three languages group the same way in software UI.
pub(crate) fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

pub(crate) fn duration(messages: &Messages, seconds: u32) -> String {
    let minutes = seconds / 60;
    let rest = if minutes > 0 {
        format!("{:02}", seconds % 60)
    } else {
        (seconds % 60).to_string()
    };
    (messages.duration)(minutes, &rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_names_map_to_languages() {
        assert_eq!(Locale::from_locale("zh_TW.UTF-8"), Some(Locale::ZhTw));
        assert_eq!(Locale::from_locale("zh_HK.UTF-8"), Some(Locale::ZhTw));
        assert_eq!(Locale::from_locale("zh_CN.utf8"), Some(Locale::ZhCn));
        assert_eq!(Locale::from_locale("ja_JP.UTF-8"), None);
        assert_eq!(Locale::from_locale("C.UTF-8"), None);
    }

    #[test]
    fn linux_console_and_non_utf8_locales_cannot_show_cjk() {
        assert_eq!(
            cjk_blocker(Some("linux"), Some("zh_TW.UTF-8")),
            Some(NoCjk::LinuxConsole)
        );
        assert_eq!(
            cjk_blocker(Some("xterm-256color"), Some("zh_TW.Big5")),
            Some(NoCjk::NotUtf8)
        );
        assert_eq!(
            cjk_blocker(Some("xterm-256color"), None),
            Some(NoCjk::NotUtf8)
        );
        assert_eq!(
            cjk_blocker(Some("xterm-256color"), Some("zh_TW.UTF-8")),
            None
        );
    }

    #[test]
    fn plurals_follow_each_language() {
        let en = Locale::En.messages();
        assert_eq!((en.cpu_cores)(1), "1 core");
        assert_eq!((en.cpu_cores)(4), "4 cores");
        assert_eq!((en.checks_summary)(0), "Everything looks good");
        assert_eq!((en.checks_summary)(1), "1 thing to know");
        assert_eq!((en.plan_time)(1), "Takes about 1 minute");
        let zh = Locale::ZhCn.messages();
        assert_eq!((zh.cpu_cores)(4), "4 核");
        assert_eq!(duration(zh, 125), "2 分 05 秒");
        assert_eq!(duration(Locale::ZhTw.messages(), 9), "9 秒");
        assert_eq!(duration(en, 125), "2 min 05 s");
        assert_eq!(group_thousands(1_210), "1,210");
    }
}
