//! Screen state and keys.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::i18n::{Catalog, LANGS, Lang, NoCjk};
use crate::mock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Screen {
    Server,
    Access,
    Review,
    Install,
}

pub(crate) const SCREENS: [Screen; 4] = [
    Screen::Server,
    Screen::Access,
    Screen::Review,
    Screen::Install,
];

impl Screen {
    pub(crate) const fn title_id(self) -> &'static str {
        match self {
            Self::Server => "step-server",
            Self::Access => "step-access",
            Self::Review => "step-review",
            Self::Install => "step-install",
        }
    }
}

/// A one-line text field. `cursor` counts chars, not bytes or cells.
#[derive(Debug)]
pub(crate) struct Field {
    pub(crate) label_id: &'static str,
    pub(crate) hint_id: Option<&'static str>,
    pub(crate) value: String,
    pub(crate) cursor: usize,
}

impl Field {
    fn new(label_id: &'static str, hint_id: Option<&'static str>, value: &str) -> Self {
        Self {
            label_id,
            hint_id,
            value: value.to_owned(),
            cursor: value.chars().count(),
        }
    }

    /// The text before the cursor, to place the terminal cursor (and the IME pre-edit) after it.
    pub(crate) fn before_cursor(&self) -> String {
        self.value.chars().take(self.cursor).collect()
    }

    fn insert(&mut self, c: char) {
        let at = self.byte_index();
        self.value.insert(at, c);
        self.cursor += 1;
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let at = self.byte_index();
        self.value.remove(at);
    }

    fn byte_index(&self) -> usize {
        self.value
            .char_indices()
            .nth(self.cursor)
            .map_or(self.value.len(), |(i, _)| i)
    }
}

#[derive(Debug)]
pub(crate) struct App {
    pub(crate) catalog: Catalog,
    /// Set when the terminal cannot show CJK; the app stays in English with ASCII borders.
    pub(crate) no_cjk: Option<NoCjk>,
    pub(crate) screen: Screen,
    /// The highlighted row while the language picker is open.
    pub(crate) picker: Option<usize>,
    pub(crate) distro: usize,
    pub(crate) fields: [Field; 3],
    pub(crate) focus: usize,
    /// Fake seconds since the install started.
    pub(crate) elapsed: u32,
    pub(crate) quit: bool,
}

impl App {
    pub(crate) fn new(lang: Lang, no_cjk: Option<NoCjk>) -> Self {
        let lang = if no_cjk.is_some() { Lang::En } else { lang };
        Self {
            catalog: Catalog::new(lang),
            no_cjk,
            screen: Screen::Server,
            picker: None,
            distro: 1,
            fields: [
                Field::new("field-address", None, mock::PRIVATE_ADDR),
                Field::new("field-domain", Some("field-domain-hint"), ""),
                Field::new("field-name", Some("field-name-hint"), ""),
            ],
            focus: 0,
            elapsed: 0,
            quit: false,
        }
    }

    pub(crate) fn set_lang(&mut self, lang: Lang) {
        if self.no_cjk.is_none() || lang == Lang::En {
            self.catalog = Catalog::new(lang);
        }
    }

    pub(crate) fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if let Some(row) = self.picker {
            self.on_picker_key(row, key.code);
            return;
        }
        if key.code == KeyCode::F(2) {
            let current = LANGS
                .iter()
                .position(|l| *l == self.catalog.lang())
                .unwrap_or(0);
            self.picker = Some(current);
            return;
        }
        match self.screen {
            Screen::Server => match key.code {
                KeyCode::Up => self.distro = self.distro.saturating_sub(1),
                KeyCode::Down => self.distro = (self.distro + 1).min(mock::DISTROS.len() - 1),
                KeyCode::Enter => self.screen = Screen::Access,
                _ => {}
            },
            Screen::Access => self.on_access_key(key.code),
            Screen::Review => match key.code {
                KeyCode::Esc => self.screen = Screen::Access,
                KeyCode::Enter => self.screen = Screen::Install,
                _ => {}
            },
            Screen::Install => {}
        }
    }

    fn on_picker_key(&mut self, row: usize, code: KeyCode) {
        match code {
            KeyCode::Up => self.picker = Some(row.saturating_sub(1)),
            KeyCode::Down => self.picker = Some((row + 1).min(LANGS.len() - 1)),
            KeyCode::Enter => {
                self.set_lang(LANGS[row]);
                self.picker = None;
            }
            KeyCode::Esc | KeyCode::F(2) => self.picker = None,
            _ => {}
        }
    }

    fn on_access_key(&mut self, code: KeyCode) {
        let field = &mut self.fields[self.focus];
        match code {
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % self.fields.len(),
            KeyCode::BackTab | KeyCode::Up => {
                self.focus = (self.focus + self.fields.len() - 1) % self.fields.len();
            }
            KeyCode::Left => field.cursor = field.cursor.saturating_sub(1),
            KeyCode::Right => field.cursor = (field.cursor + 1).min(field.value.chars().count()),
            KeyCode::Backspace => field.backspace(),
            KeyCode::Char(c) => field.insert(c),
            KeyCode::Esc => self.screen = Screen::Server,
            KeyCode::Enter => self.screen = Screen::Review,
            _ => {}
        }
    }

    pub(crate) fn tick(&mut self, fake_seconds: u32) {
        if self.screen == Screen::Install {
            self.elapsed = (self.elapsed + fake_seconds).min(mock::total_seconds());
        }
    }
}
