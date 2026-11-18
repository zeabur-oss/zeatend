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
use zeatend_host::preflight::{Finding, Status, Topic};

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
    /// Every language.
    #[cfg(test)]
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

    /// The language that `get`, a lookup of environment variables, asks for.
    ///
    /// The language comes from `LC_ALL`, then `LC_MESSAGES`, then `LANG`, as the C library reads
    /// them: an empty variable counts as unset. Chinese needs a terminal that draws it: the
    /// character set from `LC_ALL`, `LC_CTYPE` or `LANG` must be UTF-8, and `TERM` must not be
    /// `linux` (the Linux console font has no CJK glyphs). Otherwise the language is English.
    fn from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        let first = |names: [&str; 3]| {
            names
                .into_iter()
                .find_map(|name| get(name).filter(|v| !v.is_empty()))
        };
        let lang = first(["LC_ALL", "LC_MESSAGES", "LANG"])
            .and_then(|value| Self::from_locale(&value))
            .unwrap_or(Self::En);
        if lang == Self::En {
            return lang;
        }
        let charset = first(["LC_ALL", "LC_CTYPE", "LANG"])
            .unwrap_or_default()
            .to_ascii_lowercase()
            .replace('-', "");
        let console = get("TERM").as_deref() == Some("linux");
        if charset.contains("utf8") && !console {
            lang
        } else {
            Self::En
        }
    }
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
    /// How a check came out, four cells wide.
    pub(crate) status: fn(Status) -> &'static str,
    /// What a check looked at.
    pub(crate) topic: fn(Topic) -> &'static str,
    /// What a check found and what to do about it.
    pub(crate) finding: fn(&Finding) -> String,
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

/// Tenths of a GiB as GiB with one decimal.
fn gib(tenths: u64) -> String {
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
    use zeatend_host::preflight::BusyPort;
    use zeatend_plan::cluster::PortUse;

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
}
