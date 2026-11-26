//! The state of the screens and what each key does. Nothing here reads or changes the host.

use std::net::IpAddr;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use zeatend_host::progress::Event;
use zeatend_plan::cluster::Distribution;
use zeatend_plan::config::Config;
use zeatend_plan::finding::Status;
use zeatend_plan::{node_name, plan};

use super::host::{DISTRIBUTIONS, Host};
use crate::i18n::{Lang, NoCjk, Screen};
use crate::unit;

/// What the event loop does after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    /// Make the plan of this configuration for the Review screen.
    Plan(Box<Config>),
    /// Write this configuration and start the install of the plan the Review screen showed.
    Install(Box<Config>, Box<Review>),
    /// Close the screens before the install starts.
    Quit,
    /// Close the screens while the install goes on.
    Leave,
    /// Close the screens after the install ended.
    Close,
}

/// A field of the Access screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldKind {
    /// The address the person's computer reaches the API server at.
    Address,
    /// A DNS name for the API server.
    Name,
    /// The node's name in Kubernetes.
    Node,
}

/// A one-line text field. `cursor` counts characters.
#[derive(Debug, Clone)]
pub(crate) struct Field {
    /// Which field.
    pub(crate) kind: FieldKind,
    /// The text.
    pub(crate) value: String,
    /// Where the cursor is, in characters from the start.
    pub(crate) cursor: usize,
}

impl Field {
    fn new(kind: FieldKind, value: &str) -> Self {
        Self {
            kind,
            value: value.to_owned(),
            cursor: value.chars().count(),
        }
    }

    /// The text before the cursor.
    pub(crate) fn before_cursor(&self) -> String {
        self.value.chars().take(self.cursor).collect()
    }

    fn byte_index(&self) -> usize {
        self.value
            .char_indices()
            .nth(self.cursor)
            .map_or(self.value.len(), |(i, _)| i)
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "a key the screen does not use does nothing"
    )]
    fn on_key(&mut self, code: KeyCode) {
        let len = self.value.chars().count();
        match code {
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = len,
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let at = self.byte_index();
                self.value.remove(at);
            }
            KeyCode::Delete if self.cursor < len => {
                let at = self.byte_index();
                self.value.remove(at);
            }
            KeyCode::Char(c) if !c.is_control() => {
                let at = self.byte_index();
                self.value.insert(at, c);
                self.cursor += 1;
            }
            _ => {}
        }
    }

    fn text(&self) -> &str {
        self.value.trim()
    }
}

/// The plan the Review screen shows, which the install starts with unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Review {
    /// The distribution and its release, such as `K3s v1.37.2+k3s1`.
    pub(crate) release: String,
    /// The plan, with what it found on the host.
    pub(crate) plan: plan::Plan,
}

impl Review {
    /// The review of `plan`, made from `config`.
    pub(crate) fn new(config: &Config, plan: plan::Plan) -> Self {
        Self {
            release: format!("{} {}", config.distribution.title(), plan.release.version()),
            plan,
        }
    }
}

/// The choices of the first three screens.
#[derive(Debug, Clone)]
pub(crate) struct Choices {
    /// This host.
    pub(crate) host: Host,
    /// The configuration the screens start from.
    pub(crate) config: Config,
    /// Where the configuration goes.
    pub(crate) config_path: String,
    /// Whether that file exists, so its other settings stay.
    pub(crate) config_exists: bool,
    /// The distribution.
    pub(crate) distribution: Distribution,
    /// The fields of the Access screen.
    pub(crate) fields: [Field; 3],
    /// The field with focus.
    pub(crate) focus: usize,
    /// The field whose text is not valid.
    pub(crate) problem: Option<FieldKind>,
    /// The node name when the field is empty.
    pub(crate) default_node: String,
    /// The plan of the Review screen, or why there is none.
    pub(crate) review: Option<Result<Review, String>>,
}

impl Choices {
    /// The choices that start from `config` on `host`.
    pub(crate) fn new(host: Host, config: Config, config_path: String, exists: bool) -> Self {
        let is_ip = |s: &&String| s.parse::<IpAddr>().is_ok();
        let address = config
            .host
            .tls_san
            .iter()
            .find(is_ip)
            .or_else(|| host.addresses.first())
            .cloned()
            .unwrap_or_default();
        let name = config
            .host
            .tls_san
            .iter()
            .find(|s| !is_ip(s))
            .cloned()
            .unwrap_or_default();
        let node = config.host.node_name.clone().unwrap_or_default();
        let default_node = node_name::choose(None, &host.facts, &[]).unwrap_or_default();
        Self {
            distribution: config.distribution,
            fields: [
                Field::new(FieldKind::Address, &address),
                Field::new(FieldKind::Name, &name),
                Field::new(FieldKind::Node, &node),
            ],
            focus: 0,
            problem: None,
            default_node,
            review: None,
            host,
            config,
            config_path,
            config_exists: exists,
        }
    }

    /// The text of a field.
    pub(crate) fn value(&self, kind: FieldKind) -> &str {
        self.fields
            .iter()
            .find(|f| f.kind == kind)
            .map_or("", Field::text)
    }

    /// Whether a host check of the chosen distribution failed.
    pub(crate) fn blocked(&self) -> bool {
        self.host
            .checks(self.distribution)
            .iter()
            .any(|c| c.status == Status::Fail)
    }

    /// The first field whose text is not valid.
    fn check(&self) -> Option<FieldKind> {
        let address = self.value(FieldKind::Address);
        let name = self.value(FieldKind::Name);
        let node = self.value(FieldKind::Node);
        if address.parse::<IpAddr>().is_err() {
            Some(FieldKind::Address)
        } else if !name.is_empty() && !node_name::is_valid(&name.to_ascii_lowercase()) {
            Some(FieldKind::Name)
        } else if !node.is_empty() && !node_name::is_valid(node) {
            Some(FieldKind::Node)
        } else {
            None
        }
    }

    /// The configuration the choices make. A release override of another distribution goes, and so
    /// does a component in `disable` that the distribution does not package. The API certificate
    /// keeps the names it had and adds the address, unless it is the host's own, and the DNS name.
    pub(crate) fn config(&self) -> Config {
        let mut config = self.config.clone();
        config.distribution = self.distribution;
        if self.distribution != self.config.distribution {
            config.version = None;
        }
        config
            .disable
            .retain(|c| self.distribution.packaged(*c).is_some());
        let address = self.value(FieldKind::Address);
        let extra = [
            (!self.host.addresses.iter().any(|a| a == address)).then_some(address),
            Some(self.value(FieldKind::Name)),
        ];
        for value in extra.into_iter().flatten() {
            if !value.is_empty() && !config.host.tls_san.iter().any(|s| s == value) {
                config.host.tls_san.push(value.to_owned());
            }
        }
        let node = self.value(FieldKind::Node);
        config.host.node_name = (!node.is_empty()).then(|| node.to_owned());
        config
    }
}

/// A download in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Download {
    /// Bytes received.
    pub(crate) done: u64,
    /// The size, when known.
    pub(crate) total: Option<u64>,
    /// Bytes per second.
    pub(crate) rate: u64,
}

/// How the install ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum End {
    /// It finished.
    Done,
    /// It stopped with this error.
    Failed(String),
}

/// The most log lines the Install screen keeps.
const LOG_LINES: usize = 200;

/// The progress of the install unit, from its output.
#[derive(Debug, Clone, Default)]
pub(crate) struct Progress {
    /// The unit's name.
    pub(crate) unit: String,
    /// When the unit started, in seconds since the Unix epoch, from its name.
    started: Option<u64>,
    /// How long the install has run, in seconds.
    pub(crate) elapsed: u64,
    /// The steps so far; the last one runs.
    pub(crate) steps: Vec<String>,
    /// The details of the steps, the latest last.
    pub(crate) log: Vec<String>,
    /// The download in progress.
    pub(crate) download: Option<Download>,
    /// The result lines since the last step.
    pub(crate) result: Vec<String>,
    /// How the install ended.
    pub(crate) end: Option<End>,
}

impl Progress {
    /// The progress of `unit`, before its first line.
    pub(crate) fn new(unit: &str) -> Self {
        let started = unit
            .strip_prefix(unit::PREFIX)
            .and_then(|rest| rest.split(['-', '.']).next())
            .and_then(|secs| secs.parse().ok());
        Self {
            unit: unit.to_owned(),
            started,
            ..Self::default()
        }
    }

    /// Takes one line of the unit's output. A line that is not an event goes to the log.
    pub(crate) fn on_line(&mut self, text: &str) {
        match Event::parse(text) {
            Some(event) => self.on_event(event),
            None => self.push_log(text),
        }
    }

    /// Takes one event.
    pub(crate) fn on_event(&mut self, event: Event) {
        match event {
            Event::Step { text } => {
                self.steps.push(text);
                self.result.clear();
                self.download = None;
            }
            Event::Note { text } => self.push_log(&text),
            Event::Line { text } => self.result.extend(text.lines().map(str::to_owned)),
            Event::Download { done, total, rate } => {
                self.download = Some(Download { done, total, rate });
            }
            Event::DownloadEnd => self.download = None,
            Event::Done => self.end = Some(End::Done),
            Event::Failed { error } => self.end = Some(End::Failed(error)),
        }
    }

    /// The unit ended without a last event.
    pub(crate) fn ended(&mut self) {
        if self.end.is_none() {
            let unit = &self.unit;
            self.end = Some(End::Failed(format!(
                "{unit} ended without a result; see `journalctl -u {unit}`"
            )));
        }
    }

    /// Sets how long the install has run, at `now` seconds since the Unix epoch.
    pub(crate) const fn tick(&mut self, now: u64) {
        if self.end.is_none()
            && let Some(started) = self.started
        {
            self.elapsed = now.saturating_sub(started);
        }
    }

    fn push_log(&mut self, text: &str) {
        self.log.push(text.to_owned());
        if self.log.len() > LOG_LINES {
            self.log.remove(0);
        }
    }
}

/// The screens.
#[derive(Debug, Clone)]
pub(crate) struct App {
    /// The language.
    pub(crate) lang: Lang,
    /// Why the terminal cannot draw Chinese; then the screens stay in English with ASCII borders.
    pub(crate) no_cjk: Option<NoCjk>,
    /// The screen shown.
    pub(crate) screen: Screen,
    /// The highlighted row while the language picker is open.
    pub(crate) picker: Option<usize>,
    /// The choices; `None` when the screens follow an install that already runs.
    pub(crate) choices: Option<Choices>,
    /// The install.
    pub(crate) progress: Progress,
}

impl App {
    /// The screens from the Server screen, in `lang` unless the terminal cannot draw it.
    pub(crate) fn new(lang: Lang, no_cjk: Option<NoCjk>, choices: Choices) -> Self {
        Self {
            lang: if no_cjk.is_some() { Lang::En } else { lang },
            no_cjk,
            screen: Screen::Server,
            picker: None,
            choices: Some(choices),
            progress: Progress::default(),
        }
    }

    /// The Install screen, following `unit`, which already runs.
    pub(crate) fn follow(lang: Lang, no_cjk: Option<NoCjk>, unit: &str) -> Self {
        Self {
            lang: if no_cjk.is_some() { Lang::En } else { lang },
            no_cjk,
            screen: Screen::Install,
            picker: None,
            choices: None,
            progress: Progress::new(unit),
        }
    }

    /// Whether the install screen shows an install that has not ended.
    pub(crate) fn installing(&self) -> bool {
        self.screen == Screen::Install && self.progress.end.is_none()
    }

    /// Takes one key press.
    pub(crate) fn on_key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(match (self.screen, &self.progress.end) {
                (Screen::Install, None) => Action::Leave,
                (Screen::Install, Some(_)) => Action::Close,
                (Screen::Server | Screen::Access | Screen::Review, _) => Action::Quit,
            });
        }
        if let Some(row) = self.picker {
            self.on_picker_key(row, key.code);
            return None;
        }
        if key.code == KeyCode::F(2) {
            self.picker = Lang::ALL.iter().position(|l| *l == self.lang);
            return None;
        }
        match self.screen {
            Screen::Server => self.on_server_key(key.code),
            Screen::Access => return self.on_access_key(key.code),
            Screen::Review => return self.on_review_key(key.code),
            Screen::Install => {
                if key.code == KeyCode::Enter && self.progress.end.is_some() {
                    return Some(Action::Close);
                }
            }
        }
        None
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "a key the screen does not use does nothing"
    )]
    fn on_picker_key(&mut self, row: usize, code: KeyCode) {
        match code {
            KeyCode::Up => self.picker = Some(row.saturating_sub(1)),
            KeyCode::Down => self.picker = Some((row + 1).min(Lang::ALL.len() - 1)),
            KeyCode::Enter => {
                if let Some(lang) = Lang::ALL.get(row).copied()
                    && (self.no_cjk.is_none() || lang == Lang::En)
                {
                    self.lang = lang;
                }
                self.picker = None;
            }
            KeyCode::Esc | KeyCode::F(2) => self.picker = None,
            _ => {}
        }
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "a key the screen does not use does nothing"
    )]
    fn on_server_key(&mut self, code: KeyCode) {
        let Some(choices) = &mut self.choices else {
            return;
        };
        let at = DISTRIBUTIONS
            .iter()
            .position(|d| *d == choices.distribution)
            .unwrap_or(1);
        let at = match code {
            KeyCode::Up => at.saturating_sub(1),
            KeyCode::Down => (at + 1).min(DISTRIBUTIONS.len() - 1),
            KeyCode::Enter if !choices.blocked() => {
                self.screen = Screen::Access;
                at
            }
            _ => at,
        };
        if let Some(distribution) = DISTRIBUTIONS.get(at) {
            choices.distribution = *distribution;
        }
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "a key the screen does not use does nothing"
    )]
    fn on_access_key(&mut self, code: KeyCode) -> Option<Action> {
        let choices = self.choices.as_mut()?;
        let count = choices.fields.len();
        match code {
            KeyCode::Tab | KeyCode::Down => choices.focus = (choices.focus + 1) % count,
            KeyCode::BackTab | KeyCode::Up => choices.focus = (choices.focus + count - 1) % count,
            KeyCode::Esc => self.screen = Screen::Server,
            KeyCode::Enter => {
                choices.problem = choices.check();
                if let Some(kind) = choices.problem {
                    choices.focus = choices
                        .fields
                        .iter()
                        .position(|f| f.kind == kind)
                        .unwrap_or(0);
                } else {
                    return Some(Action::Plan(Box::new(choices.config())));
                }
            }
            _ => {
                if let Some(field) = choices.fields.get_mut(choices.focus) {
                    field.on_key(code);
                }
            }
        }
        None
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "a key the screen does not use does nothing"
    )]
    fn on_review_key(&mut self, code: KeyCode) -> Option<Action> {
        let choices = self.choices.as_ref()?;
        match code {
            KeyCode::Esc => self.screen = Screen::Access,
            KeyCode::Enter => {
                if let Some(Ok(review)) = &choices.review {
                    return Some(Action::Install(
                        Box::new(choices.config()),
                        Box::new(review.clone()),
                    ));
                }
            }
            _ => {}
        }
        None
    }

    /// The Review screen, showing `review`, or why there is no plan.
    pub(crate) fn reviewed(&mut self, review: Result<Review, String>) {
        if let Some(choices) = &mut self.choices {
            choices.review = Some(review);
            self.screen = Screen::Review;
        }
    }

    /// The Install screen, following `unit`.
    pub(crate) fn start(&mut self, unit: &str) {
        self.screen = Screen::Install;
        self.progress = Progress::new(unit);
    }

    /// The Install screen, where the install could not start.
    pub(crate) fn not_started(&mut self, error: String) {
        self.screen = Screen::Install;
        self.progress = Progress {
            end: Some(End::Failed(error)),
            ..Progress::default()
        };
    }
}
