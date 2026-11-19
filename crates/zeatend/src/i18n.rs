//! What zeatend says to people, in English, 繁體中文 and 简体中文.
//!
//! Each language is one [`Messages`] value. A field is a fixed string or a function that turns
//! typed data into a sentence with a `match`, so a language that lacks a message, or a new
//! outcome that a language does not word, does not compile. Log lines of the tools zeatend runs
//! stay as they are.

pub(crate) mod en;
mod zh_cn;
mod zh_tw;

use std::sync::LazyLock;

use unicode_width::UnicodeWidthStr as _;
use zeatend_plan::cluster::Distribution;
use zeatend_plan::finding::{Finding, Status, Topic};

/// A language zeatend has messages for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lang {
    /// English.
    En,
    /// Traditional Chinese, as written in Taiwan.
    ZhTw,
    /// Simplified Chinese, as written in mainland China.
    ZhCn,
}

impl Lang {
    /// Every language, in the order the language picker lists them.
    pub(crate) const ALL: [Self; 3] = [Self::En, Self::ZhTw, Self::ZhCn];

    /// The messages of this language.
    pub(crate) const fn messages(self) -> &'static Messages {
        match self {
            Self::En => &en::MESSAGES,
            Self::ZhTw => &zh_tw::MESSAGES,
            Self::ZhCn => &zh_cn::MESSAGES,
        }
    }

    /// The language of a POSIX locale name such as `zh_TW.UTF-8`, or of a tag such as
    /// `zh-Hant`. Chinese without a region or script is Simplified.
    fn from_locale(value: &str) -> Option<Self> {
        let name = value
            .split(['.', '@'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .replace('-', "_");
        if name == "zh" || name.starts_with("zh_") {
            let traditional = ["_tw", "_hk", "_mo", "_hant"];
            return Some(if traditional.iter().any(|t| name.contains(t)) {
                Self::ZhTw
            } else {
                Self::ZhCn
            });
        }
        (name == "en" || name.starts_with("en_")).then_some(Self::En)
    }

    /// The language that `get`, a lookup of environment variables, asks for, whether or not
    /// the terminal can draw it.
    ///
    /// The locale comes from `LC_ALL`, then `LC_MESSAGES`, then `LANG`, as the C library reads
    /// them: an empty variable counts as unset. As in GNU gettext, `LANGUAGE`, a list such as
    /// `zh_TW:en`, wins over the locale unless the locale is `C` or `POSIX`: the first entry
    /// zeatend has messages for is the language, and English if there is none.
    fn requested(get: &impl Fn(&str) -> Option<String>) -> Self {
        match first(get, ["LC_ALL", "LC_MESSAGES", "LANG"]).as_deref() {
            None | Some("C" | "POSIX") => Self::En,
            Some(locale) => get("LANGUAGE")
                .filter(|list| !list.is_empty())
                .map_or_else(
                    || Self::from_locale(locale),
                    |list| list.split(':').find_map(Self::from_locale),
                )
                .unwrap_or(Self::En),
        }
    }

    /// The language that `get` asks for, and English where the terminal cannot draw Chinese
    /// (see [`NoCjk::from_env`]).
    fn from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        if NoCjk::from_env(&get).is_some() {
            Self::En
        } else {
            Self::requested(&get)
        }
    }

    /// The name of the language in that language, which the language picker shows.
    pub(crate) const fn native_name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::ZhTw => "繁體中文",
            Self::ZhCn => "简体中文",
        }
    }

    /// The `LANGUAGE` value that makes another zeatend process use this language.
    pub(crate) const fn language_env(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhTw => "zh_TW",
            Self::ZhCn => "zh_CN",
        }
    }
}

/// The first of `names` that `get` finds and that is not empty.
fn first(get: &impl Fn(&str) -> Option<String>, names: [&str; 3]) -> Option<String> {
    names
        .into_iter()
        .find_map(|name| get(name).filter(|v| !v.is_empty()))
}

/// Why the terminal cannot draw Chinese.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoCjk {
    /// `TERM` is `linux`: the Linux console font has no CJK glyphs.
    LinuxConsole,
    /// The character set from `LC_ALL`, `LC_CTYPE` or `LANG` is not UTF-8, or none is set.
    NotUtf8,
}

impl NoCjk {
    /// Why the terminal that `get`, a lookup of environment variables, describes cannot draw
    /// Chinese; `None` when it can.
    pub(crate) fn from_env(get: &impl Fn(&str) -> Option<String>) -> Option<Self> {
        if get("TERM").as_deref() == Some("linux") {
            return Some(Self::LinuxConsole);
        }
        let charset = first(get, ["LC_ALL", "LC_CTYPE", "LANG"])
            .unwrap_or_default()
            .to_ascii_lowercase()
            .replace('-', "");
        (!charset.contains("utf8")).then_some(Self::NotUtf8)
    }
}

/// The language this process's environment asks for, and why the terminal cannot draw Chinese.
pub(crate) fn from_process_env() -> (Lang, Option<NoCjk>) {
    let get = |name: &str| std::env::var(name).ok();
    (Lang::requested(&get), NoCjk::from_env(&get))
}

/// The language of this process, read from its environment once.
static CURRENT: LazyLock<Lang> = LazyLock::new(|| Lang::from_env(|name| std::env::var(name).ok()));

/// The messages in the language of this process.
pub(crate) fn messages() -> &'static Messages {
    CURRENT.messages()
}

/// Every message of zeatend. A language that lacks one does not compile.
#[derive(Debug)]
pub(crate) struct Messages {
    /// The step that runs the preflight checks.
    pub(crate) checking_host: &'static str,
    /// After the checks of `plan`, when one failed.
    pub(crate) apply_stops: &'static str,
    /// The error of `apply` when a check failed.
    pub(crate) host_not_ready: &'static str,
    /// `doctor` without root, which cannot read the cluster's files.
    pub(crate) doctor_needs_root: &'static str,
    /// `apply` found no configuration file and takes the settings zeabur-installer wrote: the
    /// path of the configuration and of zeabur-installer's file.
    pub(crate) no_config_installer: fn(&str, &str) -> String,
    /// `apply` found no configuration file at the path and installs the defaults.
    pub(crate) no_config_defaults: fn(&str) -> String,
    /// `apply` follows the install that already runs in the unit.
    pub(crate) following_unit: fn(&str) -> String,
    /// `apply` started the install in the unit.
    pub(crate) started_unit: fn(&str) -> String,
    /// The unit of the cluster starts for the first time.
    pub(crate) starting_unit: fn(&str) -> String,
    /// The unit of the cluster restarts for a new configuration.
    pub(crate) restarting_unit: fn(&str) -> String,
    /// The step that waits until the node is Ready.
    pub(crate) waiting_for_node: fn(&str) -> String,
    /// The last step of an install.
    pub(crate) done: &'static str,
    /// What to do with a cluster once its server runs.
    pub(crate) ready: fn(&Ready<'_>) -> String,
    /// What to do once a node joined a cluster.
    pub(crate) joined: fn(&Joined<'_>) -> String,
    /// How a check came out, four cells wide.
    pub(crate) status: fn(Status) -> &'static str,
    /// What a check looked at.
    pub(crate) topic: fn(Topic) -> &'static str,
    /// What a check found and what to do about it.
    pub(crate) finding: fn(&Finding) -> String,
    /// The install screens.
    pub(crate) screens: &'static Screens,
}

/// A cluster that runs, for [`Messages::ready`].
#[derive(Debug)]
pub(crate) struct Ready<'a> {
    /// The distribution and its version, such as `K3s v1.37.2+k3s1`.
    pub(crate) cluster: &'a str,
    /// The node's name.
    pub(crate) node: &'a str,
    /// The command that runs kubectl as cluster admin on the server, without its arguments.
    pub(crate) kubectl: &'a str,
    /// The kubeconfig the person copies to their own computer.
    pub(crate) kubeconfig: &'a str,
    /// The address the person's computer reaches the server at, when one is known.
    pub(crate) host: Option<&'a str>,
    /// The `server:` URL in the kubeconfig and the URL to put in its place, when the file names
    /// the server by an address only the server itself reaches.
    pub(crate) rewrite: Option<(&'a str, &'a str)>,
    /// Whether `zeatend node add` adds nodes to the cluster.
    pub(crate) add_nodes: bool,
}

/// A node that joined a cluster, for [`Messages::joined`].
#[derive(Debug)]
pub(crate) struct Joined<'a> {
    /// The node's name.
    pub(crate) node: &'a str,
    /// Whether the node is a server, not a worker.
    pub(crate) server: bool,
    /// The command that runs kubectl as cluster admin on a server, without its arguments.
    pub(crate) kubectl: &'a str,
}

/// The words of the install screens.
#[derive(Debug)]
pub(crate) struct Screens {
    /// The name of each screen, in the bar at the top.
    pub(crate) screen: fn(Screen) -> &'static str,
    /// What a key does, in the bar at the bottom.
    pub(crate) key: fn(Key) -> &'static str,

    /// The title of the Server screen.
    pub(crate) server_title: &'static str,
    /// The labels of the host's facts.
    pub(crate) host_name: &'static str,
    /// The operating system.
    pub(crate) system: &'static str,
    /// CPUs.
    pub(crate) cpu: &'static str,
    /// Memory.
    pub(crate) memory: &'static str,
    /// The host's addresses.
    pub(crate) addresses: &'static str,
    /// A number of CPUs.
    pub(crate) cpus: fn(usize) -> String,
    /// An address that only a private network reaches.
    pub(crate) private_address: fn(&str) -> String,
    /// The host has no global address.
    pub(crate) no_address: &'static str,
    /// Above the choice of distribution.
    pub(crate) distribution_title: &'static str,
    /// What each distribution is for.
    pub(crate) distribution_about: fn(Distribution) -> &'static str,
    /// Above the host checks.
    pub(crate) checks_title: &'static str,
    /// How many checks failed and warned.
    pub(crate) checks_summary: fn(usize, usize) -> String,
    /// A check failed, so the screens do not go on.
    pub(crate) checks_stop: &'static str,

    /// The title of the Access screen.
    pub(crate) access_title: &'static str,
    /// The host has only private addresses.
    pub(crate) access_private: &'static str,
    /// The host has a public address.
    pub(crate) access_public: &'static str,
    /// The address the person's computer reaches the API server at.
    pub(crate) field_address: &'static str,
    /// Under the address.
    pub(crate) field_address_hint: &'static str,
    /// A DNS name for the API server.
    pub(crate) field_name: &'static str,
    /// Under the DNS name.
    pub(crate) field_name_hint: &'static str,
    /// The node's name in Kubernetes.
    pub(crate) field_node: &'static str,
    /// Under the node name, with the name it gets when the field is empty.
    pub(crate) field_node_hint: fn(&str) -> String,
    /// The address is not an IP address.
    pub(crate) bad_address: &'static str,
    /// The DNS name is not one.
    pub(crate) bad_name: &'static str,
    /// The node name is not one.
    pub(crate) bad_node: &'static str,

    /// The title of the Review screen.
    pub(crate) review_title: &'static str,
    /// The release.
    pub(crate) review_release: &'static str,
    /// The address.
    pub(crate) review_address: &'static str,
    /// The DNS name.
    pub(crate) review_name: &'static str,
    /// No DNS name.
    pub(crate) review_no_name: &'static str,
    /// The node name.
    pub(crate) review_node: &'static str,
    /// The configuration file.
    pub(crate) review_config: &'static str,
    /// Above what happens next.
    pub(crate) review_next: &'static str,
    /// The choices go into a new configuration file.
    pub(crate) next_write: fn(&str) -> String,
    /// The choices replace those in the configuration file; its other settings stay.
    pub(crate) next_update: fn(&str) -> String,
    /// The install runs in a systemd unit.
    pub(crate) next_unit: &'static str,
    /// How to follow the install again.
    pub(crate) next_follow: &'static str,

    /// The title of the Install screen.
    pub(crate) install_title: &'static str,
    /// The unit the install runs in.
    pub(crate) install_unit: fn(&str) -> String,
    /// How long the install has run.
    pub(crate) install_elapsed: fn(&str) -> String,
    /// Before the first step of the install.
    pub(crate) install_starting: &'static str,
    /// The install finished.
    pub(crate) install_done: &'static str,
    /// The install stopped with an error.
    pub(crate) install_failed: &'static str,
    /// Minutes and seconds; the seconds come with two digits after a minute.
    pub(crate) duration: fn(u64, &str) -> String,
    /// The title of the log box.
    pub(crate) log_title: &'static str,
    /// The title of the box with what to run next.
    pub(crate) result_title: &'static str,
    /// Under the result when it does not fit: the screens print all of it when they close.
    pub(crate) result_more: &'static str,

    /// The title of the language picker.
    pub(crate) language_title: &'static str,
    /// Under the languages.
    pub(crate) language_hint: &'static str,
    /// Next to a language the terminal cannot draw.
    pub(crate) language_needs_utf8: &'static str,

    /// Before the screens open, while zeatend reads the host.
    pub(crate) reading_host: &'static str,
    /// After the screens close while the install goes on.
    pub(crate) left: fn(&str) -> String,
    /// `zeatend` without a terminal: what to run instead.
    pub(crate) no_terminal: &'static str,
    /// `zeatend` without root.
    pub(crate) needs_root: &'static str,
    /// `zeatend` on a host without systemd.
    pub(crate) needs_systemd: &'static str,
    /// The configuration file joins a cluster, which the screens do not do.
    pub(crate) joins: fn(&str) -> String,
}

/// A screen of the install screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    /// The host and the distribution.
    Server,
    /// How the person's computer reaches the cluster.
    Access,
    /// The choices and what happens next.
    Review,
    /// The install's progress.
    Install,
}

/// What a key does on the install screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Key {
    /// Up and Down move the choice.
    Choose,
    /// Enter goes to the next screen.
    Next,
    /// Esc goes back.
    Back,
    /// Tab goes to the next field.
    Field,
    /// F2 opens the language picker.
    Language,
    /// Ctrl+C leaves the screens.
    Quit,
    /// Enter on the Review screen starts the install.
    Install,
    /// Ctrl+C while the install runs: the screens close and the install goes on.
    Leave,
    /// Enter after the install: the screens close.
    Close,
}

impl Messages {
    /// One line of the preflight results: the status, the topic and the finding.
    pub(crate) fn check_line(&self, finding: &Finding) -> String {
        format!(
            "{} {} {}",
            (self.status)(finding.status()),
            pad((self.topic)(finding.topic()), 18),
            (self.finding)(finding)
        )
    }
}

/// `text` with spaces after it up to `cells` terminal cells; a CJK character takes two.
fn pad(text: &str, cells: usize) -> String {
    let fill = cells.saturating_sub(text.width());
    format!("{text}{}", " ".repeat(fill))
}

/// Minutes and seconds in the words of `screens`.
pub(crate) fn duration(screens: &Screens, seconds: u64) -> String {
    let minutes = seconds / 60;
    let rest = if minutes > 0 {
        format!("{:02}", seconds % 60)
    } else {
        (seconds % 60).to_string()
    };
    (screens.duration)(minutes, &rest)
}

/// Tenths of a GiB as GiB with one decimal.
pub(crate) fn gib(tenths: u64) -> String {
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// A list in Chinese: `a、b 和 c`, with `and` as the last joint. A space goes between the
/// joint and a neighbour that ends or starts with a Latin letter or a digit.
fn zh_list(items: &[String], and: &str) -> String {
    let latin = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '/');
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => {
            let head = rest.join("、");
            let before = if latin(head.chars().next_back()) {
                " "
            } else {
                ""
            };
            let after = if latin(last.chars().next()) { " " } else { "" };
            format!("{head}{before}{and}{after}{last}")
        }
    }
}

#[cfg(test)]
mod tests {
    use zeatend_plan::cluster::PortUse;
    use zeatend_plan::finding::BusyPort;

    use super::*;

    fn env(vars: &[(&str, &str)]) -> Lang {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Lang::from_env(move |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()))
    }

    #[test]
    fn the_locale_names_the_language() {
        let cases = [
            ("zh_TW.UTF-8", Some(Lang::ZhTw)),
            ("zh_HK.UTF-8", Some(Lang::ZhTw)),
            ("zh-Hant", Some(Lang::ZhTw)),
            ("zh_CN.UTF-8", Some(Lang::ZhCn)),
            ("zh_SG.utf8", Some(Lang::ZhCn)),
            ("zh", Some(Lang::ZhCn)),
            ("en_US.UTF-8", Some(Lang::En)),
            ("C.UTF-8", None),
            ("POSIX", None),
            ("de_DE.UTF-8", None),
            ("zu_ZA.UTF-8", None),
        ];
        for (value, lang) in cases {
            assert_eq!(Lang::from_locale(value), lang, "{value}");
        }
    }

    #[test]
    fn lc_all_wins_over_lc_messages_and_lang() {
        assert_eq!(
            env(&[("LC_ALL", "zh_CN.UTF-8"), ("LANG", "zh_TW.UTF-8")]),
            Lang::ZhCn
        );
        assert_eq!(
            env(&[("LC_MESSAGES", "zh_TW.UTF-8"), ("LANG", "en_US.UTF-8")]),
            Lang::ZhTw
        );
        // An empty variable counts as unset.
        assert_eq!(env(&[("LC_ALL", ""), ("LANG", "zh_TW.UTF-8")]), Lang::ZhTw);
        assert_eq!(env(&[("LANG", "fr_FR.UTF-8")]), Lang::En);
        assert_eq!(env(&[]), Lang::En);
    }

    #[test]
    fn language_wins_over_the_locale_unless_the_locale_is_c() {
        let utf8 = ("LANG", "en_US.UTF-8");
        assert_eq!(env(&[utf8, ("LANGUAGE", "zh_TW:en")]), Lang::ZhTw);
        // Entries without messages are skipped.
        assert_eq!(env(&[utf8, ("LANGUAGE", "fr:zh_CN")]), Lang::ZhCn);
        // No entry has messages: English, not the locale's language.
        assert_eq!(
            env(&[("LANG", "zh_TW.UTF-8"), ("LANGUAGE", "fr:de")]),
            Lang::En
        );
        assert_eq!(
            env(&[("LANG", "zh_TW.UTF-8"), ("LANGUAGE", "")]),
            Lang::ZhTw
        );
        assert_eq!(env(&[("LANG", "C"), ("LANGUAGE", "zh_TW")]), Lang::En);
        assert_eq!(env(&[("LANGUAGE", "zh_TW")]), Lang::En);
        assert_eq!(
            env(&[("LC_ALL", "C.UTF-8"), ("LANGUAGE", "zh_TW")]),
            Lang::ZhTw
        );
    }

    #[test]
    fn chinese_falls_back_to_english_where_the_terminal_cannot_draw_it() {
        assert_eq!(env(&[("LANG", "zh_TW.Big5")]), Lang::En);
        assert_eq!(env(&[("LANG", "zh_CN.GB18030")]), Lang::En);
        assert_eq!(
            env(&[("LC_MESSAGES", "zh_TW.UTF-8"), ("LC_CTYPE", "C")]),
            Lang::En
        );
        assert_eq!(env(&[("LANG", "zh_TW.UTF-8"), ("TERM", "linux")]), Lang::En);
        assert_eq!(
            env(&[("LC_MESSAGES", "zh_TW.UTF-8"), ("LC_CTYPE", "C.UTF-8")]),
            Lang::ZhTw
        );
    }

    #[test]
    fn chinese_lists_space_only_next_to_latin_text() {
        let list = |items: &[&str]| {
            let items: Vec<String> = items.iter().map(|s| (*s).to_owned()).collect();
            zh_list(&items, "和")
        };
        assert_eq!(list(&[]), "");
        assert_eq!(list(&["80"]), "80");
        assert_eq!(list(&["80", "443", "6443"]), "80、443 和 6443");
        assert_eq!(list(&["/", "/var/lib"]), "/ 和 /var/lib");
        assert_eq!(
            list(&["6443（Kubernetes API）", "10250（kubelet）"]),
            "6443（Kubernetes API）和 10250（kubelet）"
        );
    }

    #[test]
    fn check_lines_line_up_in_every_language() {
        let findings = [
            Finding::RootOk,
            Finding::PortsBusy {
                ports: vec![BusyPort {
                    port: 6443,
                    purpose: PortUse::KubernetesApi,
                }],
            },
            Finding::CrashKept { mib: 256 },
        ];
        for lang in Lang::ALL {
            let m = lang.messages();
            for finding in &findings {
                let line = m.check_line(finding);
                let finding_text = (m.finding)(finding);
                let prefix = line.strip_suffix(&finding_text).unwrap();
                assert_eq!(prefix.width(), 4 + 1 + 18 + 1, "{lang:?}: {line}");
            }
        }
    }

    #[test]
    fn chinese_findings_join_lists_their_own_way() {
        let free = Finding::PortsFree {
            ports: vec![80, 443, 10250],
        };
        let crypt = Finding::CryptYes {
            paths: vec!["/", "/var/lib/rancher"],
        };
        let busy = Finding::PortsBusy {
            ports: vec![
                BusyPort {
                    port: 6443,
                    purpose: PortUse::KubernetesApi,
                },
                BusyPort {
                    port: 10250,
                    purpose: PortUse::Kubelet,
                },
            ],
        };
        let tw = Lang::ZhTw.messages().finding;
        assert_eq!(tw(&free), "80、443 和 10250 都沒有程式使用");
        assert_eq!(tw(&crypt), "/ 和 /var/lib/rancher 都在 dm-crypt 裝置上");
        assert_eq!(
            tw(&busy),
            "已有程式使用 6443（Kubernetes API）、10250（kubelet），用 `ss -ltnp` 找出是哪個程式"
        );
        let cn = Lang::ZhCn.messages().finding;
        assert_eq!(cn(&free), "80、443 和 10250 都没有程序占用");
        assert_eq!(cn(&crypt), "/ 和 /var/lib/rancher 都在 dm-crypt 设备上");
    }

    #[test]
    fn a_finding_of_two_sentences_ends_with_a_full_stop() {
        let findings = [
            Finding::CryptNo {
                path: "/var/lib/rancher",
            },
            Finding::DockerFound {
                found: vec!["docker.service".to_owned()],
            },
            Finding::ForwardDrop,
            Finding::RecordMissing {
                path: "/var/lib/zeatend".to_owned(),
            },
            Finding::RecordNewer {
                zeatend_version: "9.9.9".to_owned(),
            },
            Finding::ReservesLow {
                nodes: vec![zeatend_plan::finding::Kept {
                    name: "edge-1".to_owned(),
                    mib: 0,
                }],
                needed_mib: 512,
                distribution: Distribution::K3s,
            },
        ];
        for lang in Lang::ALL {
            let finding = lang.messages().finding;
            for f in &findings {
                let text = finding(f);
                let stop = if lang == Lang::En { ". " } else { "。" };
                assert!(text.contains(stop) && text.ends_with(stop.trim()), "{text}");
            }
        }
    }

    #[test]
    fn the_ready_text_gives_each_command_with_the_address_of_the_server() {
        let k3s = Ready {
            cluster: "K3s v1.37.2+k3s1",
            node: "edge-1",
            kubectl: "sudo kubectl --kubeconfig /k3s.yaml",
            kubeconfig: "/k3s.yaml",
            host: Some("203.0.113.7"),
            rewrite: Some(("https://127.0.0.1:6443", "https://203.0.113.7:6443")),
            add_nodes: true,
        };
        let solo = Ready {
            host: None,
            rewrite: None,
            add_nodes: false,
            ..k3s
        };
        for lang in Lang::ALL {
            let ready = lang.messages().ready;
            let text = ready(&k3s);
            for command in [
                "\n  sudo kubectl --kubeconfig /k3s.yaml get nodes -o wide\n",
                "@203.0.113.7 sudo cat /k3s.yaml > ~/.kube/edge-1.yaml)\n  sed -i.bak \
                 's#https://127.0.0.1:6443#https://203.0.113.7:6443#' ~/.kube/edge-1.yaml\n  \
                 KUBECONFIG=~/.kube/edge-1.yaml kubectl get nodes\n",
                "`sudo zeatend node add`",
                "\n  mkdir -p ~/.kube\n  (umask 077; ssh ",
            ] {
                assert!(text.contains(command), "{text}");
            }
            let text = ready(&solo);
            assert!(text.contains("host.tlsSan"), "{text}");
            assert!(
                !text.contains("sed ") && !text.contains("node add"),
                "{text}"
            );
        }
    }
}
