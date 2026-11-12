//! String catalogs (Fluent) and the language and terminal checks.

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};

/// A language the installer ships strings for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lang {
    En,
    ZhTw,
    ZhCn,
    Ja,
}

/// Every language, in the order the picker lists them.
pub(crate) const LANGS: [Lang; 4] = [Lang::En, Lang::ZhTw, Lang::ZhCn, Lang::Ja];

impl Lang {
    pub(crate) const fn tag(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhTw => "zh-TW",
            Self::ZhCn => "zh-CN",
            Self::Ja => "ja",
        }
    }

    /// The name of the language in that language; never translated.
    pub(crate) const fn native_name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::ZhTw => "繁體中文",
            Self::ZhCn => "简体中文",
            Self::Ja => "日本語",
        }
    }

    const fn source(self) -> &'static str {
        match self {
            Self::En => include_str!("../locales/en.ftl"),
            Self::ZhTw => include_str!("../locales/zh-TW.ftl"),
            Self::ZhCn => include_str!("../locales/zh-CN.ftl"),
            Self::Ja => include_str!("../locales/ja.ftl"),
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
        if name.starts_with("ja") {
            return Some(Self::Ja);
        }
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
        LANGS
            .into_iter()
            .find(|lang| lang.tag().eq_ignore_ascii_case(tag))
    }
}

/// Reads an environment variable the way the C library does: empty means unset.
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The language from `LC_ALL`, then `LC_MESSAGES`, then `LANG`; English when none match.
pub(crate) fn lang_from_env() -> Lang {
    for name in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(value) = env(name) {
            return Lang::from_locale(&value).unwrap_or(Lang::En);
        }
    }
    Lang::En
}

/// Why the terminal cannot show Chinese or Japanese, if it cannot.
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

/// The strings of one language, with English for any message it lacks.
pub(crate) struct Catalog {
    lang: Lang,
    bundle: FluentBundle<FluentResource>,
    english: FluentBundle<FluentResource>,
}

impl std::fmt::Debug for Catalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Catalog").field("lang", &self.lang).finish()
    }
}

fn bundle(lang: Lang) -> FluentBundle<FluentResource> {
    let langid = lang.tag().parse().expect("language tags are valid");
    let mut bundle = FluentBundle::new(vec![langid]);
    // Fluent wraps placeables in U+2068/U+2069 by default; terminals draw them as a cell or
    // garbage.
    bundle.set_use_isolating(false);
    let resource = FluentResource::try_new(lang.source().to_owned())
        .unwrap_or_else(|(_, errors)| panic!("{}.ftl does not parse: {errors:?}", lang.tag()));
    bundle
        .add_resource(resource)
        .unwrap_or_else(|errors| panic!("{}.ftl has duplicate ids: {errors:?}", lang.tag()));
    bundle
}

impl Catalog {
    pub(crate) fn new(lang: Lang) -> Self {
        Self {
            lang,
            bundle: bundle(lang),
            english: bundle(Lang::En),
        }
    }

    pub(crate) const fn lang(&self) -> Lang {
        self.lang
    }

    pub(crate) fn t(&self, id: &str) -> String {
        self.tr(id, &[])
    }

    pub(crate) fn tr(&self, id: &str, args: &[(&str, FluentValue<'_>)]) -> String {
        let mut fluent_args = FluentArgs::new();
        for (name, value) in args {
            fluent_args.set(*name, value.clone());
        }
        for bundle in [&self.bundle, &self.english] {
            let Some(pattern) = bundle.get_message(id).and_then(|m| m.value()) else {
                continue;
            };
            let mut errors = Vec::new();
            return bundle
                .format_pattern(pattern, Some(&fluent_args), &mut errors)
                .into_owned();
        }
        id.to_owned()
    }
}

/// A number with comma thousands separators. All four languages group the same way in software UI.
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

pub(crate) fn duration(catalog: &Catalog, seconds: u32) -> String {
    let minutes = seconds / 60;
    let rest = if minutes > 0 {
        format!("{:02}", seconds % 60)
    } else {
        (seconds % 60).to_string()
    };
    catalog.tr(
        "duration",
        &[
            ("m", FluentValue::from(minutes)),
            ("s", FluentValue::from(rest)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_names_map_to_languages() {
        assert_eq!(Lang::from_locale("zh_TW.UTF-8"), Some(Lang::ZhTw));
        assert_eq!(Lang::from_locale("zh_HK.UTF-8"), Some(Lang::ZhTw));
        assert_eq!(Lang::from_locale("zh_CN.utf8"), Some(Lang::ZhCn));
        assert_eq!(Lang::from_locale("ja_JP.UTF-8"), Some(Lang::Ja));
        assert_eq!(Lang::from_locale("C.UTF-8"), None);
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
            cjk_blocker(Some("xterm-256color"), Some("ja_JP.UTF-8")),
            None
        );
    }

    #[test]
    fn every_language_has_every_english_message() {
        let english = FluentResource::try_new(Lang::En.source().to_owned()).unwrap();
        for lang in LANGS {
            let catalog = Catalog::new(lang);
            for entry in english.entries() {
                if let fluent_syntax::ast::Entry::Message(message) = entry {
                    let id = message.id.name;
                    assert!(catalog.bundle.has_message(id), "{} lacks {id}", lang.tag());
                }
            }
        }
    }

    #[test]
    fn plurals_follow_each_language() {
        let en = Catalog::new(Lang::En);
        assert_eq!(en.tr("cpu-cores", &[("n", 1.into())]), "1 core");
        assert_eq!(en.tr("cpu-cores", &[("n", 4.into())]), "4 cores");
        let ja = Catalog::new(Lang::Ja);
        assert_eq!(ja.tr("cpu-cores", &[("n", 1.into())]), "1 コア");
        assert_eq!(duration(&ja, 125), "2分05秒");
        assert_eq!(duration(&Catalog::new(Lang::ZhTw), 9), "9 秒");
        assert_eq!(group_thousands(1_210), "1,210");
    }
}
