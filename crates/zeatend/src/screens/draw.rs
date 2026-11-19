//! Drawing the screens. Colours are the 16 ANSI colours with bold, dim and reverse, so the
//! terminal's own light or dark palette decides how they look.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use zeatend_plan::finding::{Check, Status};

use super::app::{App, Choices, End, FieldKind, Progress};
use super::host::{DISTRIBUTIONS, is_private};
use super::text::{pad, truncate, width, wrap};
use crate::i18n::{self, Key, Lang, Messages, NoCjk, Screen, Screens};
use crate::output::size;

/// The screens in the order of the bar at the top.
const SCREENS: [Screen; 4] = [
    Screen::Server,
    Screen::Access,
    Screen::Review,
    Screen::Install,
];

/// Borders for a terminal that cannot draw box-drawing characters, such as the Linux console.
const ASCII_BORDER: border::Set<'static> = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

const fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

const fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

const fn colour(color: Color) -> Style {
    Style::new().fg(color)
}

/// Why the screens are in English. It is in English because the terminal cannot draw Chinese.
pub(crate) const fn fallback(reason: NoCjk) -> &'static str {
    match reason {
        NoCjk::LinuxConsole => "The Linux console cannot show Chinese, so zeatend uses English.",
        NoCjk::NotUtf8 => "This terminal is not set to UTF-8, so zeatend uses English.",
    }
}

/// What a drawing needs: the words and the glyphs the terminal can draw.
struct Pen<'a> {
    messages: &'a Messages,
    words: &'a Screens,
    ascii: bool,
}

impl Pen<'_> {
    fn block(&self, title: &str) -> Block<'static> {
        Block::bordered()
            .border_set(if self.ascii {
                ASCII_BORDER
            } else {
                border::PLAIN
            })
            .title(Span::styled(format!(" {title} "), bold()))
    }

    const fn ok(&self) -> &'static str {
        if self.ascii { "+" } else { "✓" }
    }
}

/// `text` wrapped to `cells`, the first line after `first`, the others after as many spaces.
fn hanging(first: Span<'static>, text: &str, style: Style, cells: usize) -> Vec<Line<'static>> {
    let indent = width(&first.content);
    let mut first = Some(first);
    wrap(text, cells.saturating_sub(indent))
        .into_iter()
        .map(|part| {
            let lead = first
                .take()
                .unwrap_or_else(|| Span::raw(" ".repeat(indent)));
            Line::from(vec![lead, Span::styled(part, style)])
        })
        .collect()
}

/// Draws the screens and returns where the text cursor is, when a field has focus.
pub(crate) fn draw(frame: &mut Frame<'_>, app: &App) -> Option<Position> {
    let messages = app.lang.messages();
    let pen = Pen {
        messages,
        words: messages.screens,
        ascii: app.no_cjk.is_some(),
    };
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    draw_header(frame, &pen, app.screen, header);
    let mut cursor = None;
    match (app.screen, &app.choices) {
        (Screen::Server, Some(choices)) => draw_server(frame, &pen, app.no_cjk, choices, body),
        (Screen::Access, Some(choices)) => cursor = draw_access(frame, &pen, choices, body),
        (Screen::Review, Some(choices)) => draw_review(frame, &pen, choices, body),
        (Screen::Install, _) => draw_install(frame, &pen, &app.progress, body),
        (Screen::Server | Screen::Access | Screen::Review, None) => {}
    }
    draw_footer(frame, &pen, app, footer);
    if let Some(row) = app.picker {
        cursor = None;
        draw_picker(frame, &pen, app, row);
    }
    if let Some(position) = cursor {
        frame.set_cursor_position(position);
    }
    cursor
}

fn draw_header(frame: &mut Frame<'_>, pen: &Pen<'_>, screen: Screen, area: Rect) {
    frame.render_widget(Paragraph::new(Span::styled(" zeatend", bold())), area);
    let mut steps = Vec::new();
    for (i, step) in SCREENS.iter().enumerate() {
        let label = format!(" {} {} ", i + 1, (pen.words.screen)(*step));
        steps.push(if *step == screen {
            Span::styled(
                label,
                bold().fg(Color::Cyan).add_modifier(Modifier::REVERSED),
            )
        } else {
            Span::styled(label, dim())
        });
    }
    frame.render_widget(
        Paragraph::new(Line::from(steps)).alignment(Alignment::Right),
        area,
    );
}

fn draw_footer(frame: &mut Frame<'_>, pen: &Pen<'_>, app: &App, area: Rect) {
    let keys: &[(&str, Key)] = if app.picker.is_some() {
        &[
            ("Up/Down", Key::Choose),
            ("Enter", Key::Next),
            ("Esc", Key::Back),
        ]
    } else {
        match app.screen {
            Screen::Server => &[
                ("Up/Down", Key::Choose),
                ("Enter", Key::Next),
                ("F2", Key::Language),
                ("Ctrl+C", Key::Quit),
            ],
            Screen::Access => &[
                ("Tab", Key::Field),
                ("Enter", Key::Next),
                ("Esc", Key::Back),
                ("F2", Key::Language),
                ("Ctrl+C", Key::Quit),
            ],
            Screen::Review => &[
                ("Enter", Key::Install),
                ("Esc", Key::Back),
                ("F2", Key::Language),
                ("Ctrl+C", Key::Quit),
            ],
            Screen::Install if app.installing() => &[("F2", Key::Language), ("Ctrl+C", Key::Leave)],
            Screen::Install => &[("Enter", Key::Close), ("F2", Key::Language)],
        }
    };
    let mut spans = vec![Span::raw(" ")];
    for (key, what) in keys {
        spans.push(Span::styled(*key, bold()));
        spans.push(Span::raw(format!(" {}   ", (pen.words.key)(*what))));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_server(
    frame: &mut Frame<'_>,
    pen: &Pen<'_>,
    no_cjk: Option<NoCjk>,
    choices: &Choices,
    area: Rect,
) {
    let w = pen.words;
    let outer = pen.block(w.server_title);
    let inner = outer.inner(area).inner(Margin::new(1, 0));
    frame.render_widget(outer, area);
    let cells = usize::from(inner.width);
    let host = &choices.host;

    let mut lines = Vec::new();
    if let Some(reason) = no_cjk {
        lines.extend(hanging(
            Span::raw(""),
            fallback(reason),
            colour(Color::Yellow),
            cells,
        ));
    }
    let addresses: Vec<String> = host
        .addresses
        .iter()
        .map(|a| {
            if is_private(a) {
                (w.private_address)(a)
            } else {
                a.clone()
            }
        })
        .collect();
    let facts = [
        (w.host_name, host.hostname.clone()),
        (w.system, host.system.clone()),
        (w.cpu, (w.cpus)(host.cpus)),
        (
            w.memory,
            format!("{} GiB", i18n::gib(host.memory_mib * 10 / 1024)),
        ),
        (
            w.addresses,
            if addresses.is_empty() {
                w.no_address.to_owned()
            } else {
                addresses.join(", ")
            },
        ),
    ];
    let label_width = facts.iter().map(|(l, _)| width(l)).max().unwrap_or(0) + 2;
    for (label, value) in facts {
        lines.push(Line::from(vec![
            Span::styled(pad(label, label_width), dim()),
            Span::raw(truncate(&value, cells.saturating_sub(label_width))),
        ]));
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled(w.distribution_title, bold()));
    for distribution in DISTRIBUTIONS {
        lines.push(if distribution == choices.distribution {
            Line::styled(
                format!("(*) {}", distribution.title()),
                bold().fg(Color::Cyan),
            )
        } else {
            Line::raw(format!("( ) {}", distribution.title()))
        });
    }
    lines.extend(hanging(
        Span::raw("    "),
        (w.distribution_about)(choices.distribution),
        dim(),
        cells,
    ));

    let rows = usize::from(inner.height).saturating_sub(lines.len());
    lines.extend(check_lines(
        pen,
        host.checks(choices.distribution),
        cells,
        rows,
    ));
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The summary of the host checks, and the ones that did not pass.
/// The summary of the host checks and the ones that did not pass, in `rows` rows when it can:
/// a check takes one line, cut short, when the wrapped ones would not fit.
fn check_lines(pen: &Pen<'_>, checks: &[Check], cells: usize, rows: usize) -> Vec<Line<'static>> {
    let w = pen.words;
    let m = pen.messages;
    let failed = checks.iter().filter(|c| c.status == Status::Fail).count();
    let warned = checks.iter().filter(|c| c.status == Status::Warn).count();
    let summary_style = match (failed, warned) {
        (0, 0) => colour(Color::Green),
        (0, _) => colour(Color::Yellow),
        _ => colour(Color::Red),
    };
    let mut lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled(format!("{}  ", w.checks_title), bold()),
            Span::styled((w.checks_summary)(failed, warned), summary_style),
        ]),
    ];
    if failed > 0 {
        lines.extend(hanging(
            Span::raw(""),
            w.checks_stop,
            colour(Color::Red),
            cells,
        ));
    }
    let shown: Vec<(&'static str, Style, String)> = checks
        .iter()
        .filter(|c| c.status != Status::Pass)
        .map(|check| {
            let (mark, style) = if check.status == Status::Fail {
                ("x ", colour(Color::Red).add_modifier(Modifier::BOLD))
            } else {
                ("! ", colour(Color::Yellow).add_modifier(Modifier::BOLD))
            };
            let text = format!("{}: {}", (m.topic)(check.name), (m.finding)(&check.finding));
            (mark, style, text)
        })
        .collect();
    let wrapped: Vec<Line<'static>> = shown
        .iter()
        .flat_map(|(mark, style, text)| {
            hanging(Span::styled(*mark, *style), text, Style::new(), cells)
        })
        .collect();
    if lines.len() + wrapped.len() <= rows {
        lines.extend(wrapped);
    } else {
        for (mark, style, text) in shown {
            lines.push(Line::from(vec![
                Span::styled(mark, style),
                Span::raw(truncate(&text, cells.saturating_sub(2))),
            ]));
        }
    }
    lines
}

const fn field_label(w: &Screens, kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Address => w.field_address,
        FieldKind::Name => w.field_name,
        FieldKind::Node => w.field_node,
    }
}

fn field_hint(w: &Screens, choices: &Choices, kind: FieldKind) -> String {
    match kind {
        FieldKind::Address => w.field_address_hint.to_owned(),
        FieldKind::Name => w.field_name_hint.to_owned(),
        FieldKind::Node => (w.field_node_hint)(&choices.default_node),
    }
}

const fn field_problem(w: &Screens, kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Address => w.bad_address,
        FieldKind::Name => w.bad_name,
        FieldKind::Node => w.bad_node,
    }
}

fn draw_access(
    frame: &mut Frame<'_>,
    pen: &Pen<'_>,
    choices: &Choices,
    area: Rect,
) -> Option<Position> {
    let w = pen.words;
    let outer = pen.block(w.access_title);
    let inner = outer.inner(area).inner(Margin::new(1, 0));
    frame.render_widget(outer, area);
    let cells = usize::from(inner.width);

    let private = choices.host.addresses.iter().all(|a| is_private(a));
    let note = wrap(
        if private {
            w.access_private
        } else {
            w.access_public
        },
        cells,
    );
    let note_rows = u16::try_from(note.len()).unwrap_or(1);
    let mut constraints = vec![Constraint::Length(note_rows + 1)];
    for _ in &choices.fields {
        constraints.push(Constraint::Length(3));
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Fill(1));
    let rows = Layout::vertical(constraints).split(inner);
    let mut rows = rows.iter().copied();
    if let Some(note_area) = rows.next() {
        frame.render_widget(
            Paragraph::new(note.into_iter().map(Line::raw).collect::<Vec<_>>()),
            note_area,
        );
    }

    let mut cursor = None;
    for (i, field) in choices.fields.iter().enumerate() {
        let (Some(box_area), Some(hint_area)) = (rows.next(), rows.next()) else {
            break;
        };
        let focused = i == choices.focus;
        let field_block = pen
            .block(field_label(w, field.kind))
            .border_style(if focused { colour(Color::Cyan) } else { dim() });
        let text_area = field_block.inner(box_area);
        frame.render_widget(field_block, box_area);

        // Scroll left so the cursor stays inside the box.
        let room = usize::from(text_area.width).saturating_sub(1);
        let mut before: Vec<char> = field.before_cursor().chars().collect();
        let mut skipped = 0;
        while width(&before.iter().collect::<String>()) > room && !before.is_empty() {
            before.remove(0);
            skipped += 1;
        }
        let visible: String = field.value.chars().skip(skipped).collect();
        frame.render_widget(
            Paragraph::new(truncate(&visible, usize::from(text_area.width))),
            text_area,
        );
        if focused {
            let x = u16::try_from(width(&before.iter().collect::<String>())).unwrap_or(0);
            cursor = Some(Position::new(text_area.x + x, text_area.y));
        }

        let (hint, style) = if choices.problem == Some(field.kind) {
            (field_problem(w, field.kind).to_owned(), colour(Color::Red))
        } else {
            (field_hint(w, choices, field.kind), dim())
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {}", truncate(&hint, cells.saturating_sub(1))),
                style,
            )),
            hint_area,
        );
    }
    cursor
}

fn draw_review(frame: &mut Frame<'_>, pen: &Pen<'_>, choices: &Choices, area: Rect) {
    let w = pen.words;
    let outer = pen.block(w.review_title);
    let inner = outer.inner(area).inner(Margin::new(1, 0));
    frame.render_widget(outer, area);
    let cells = usize::from(inner.width);

    let review = choices.review.as_ref();
    let ok = review.and_then(|r| r.as_ref().ok());
    let name = choices.value(FieldKind::Name);
    let rows = [
        (
            w.review_release,
            ok.map_or_else(
                || choices.distribution.title().to_owned(),
                |r| r.release.clone(),
            ),
            Style::new(),
        ),
        (
            w.review_address,
            choices.value(FieldKind::Address).to_owned(),
            Style::new(),
        ),
        if name.is_empty() {
            (w.review_name, w.review_no_name.to_owned(), dim())
        } else {
            (w.review_name, name.to_owned(), Style::new())
        },
        (
            w.review_node,
            ok.map_or_else(
                || choices.value(FieldKind::Node).to_owned(),
                |r| r.node_name.clone(),
            ),
            Style::new(),
        ),
        (w.review_config, choices.config_path.clone(), Style::new()),
    ];
    let label_width = rows.iter().map(|(l, ..)| width(l)).max().unwrap_or(0) + 2;
    let mut lines = Vec::new();
    for (label, value, style) in rows {
        lines.extend(hanging(
            Span::styled(pad(label, label_width), dim()),
            &value,
            style,
            cells,
        ));
    }
    lines.push(Line::raw(""));
    if let Some(Err(error)) = review {
        lines.extend(hanging(Span::raw(""), error, colour(Color::Red), cells));
    } else {
        lines.push(Line::styled(w.review_next, bold()));
        let save = if choices.config_exists {
            (w.next_update)(&choices.config_path)
        } else {
            (w.next_write)(&choices.config_path)
        };
        for item in [save.as_str(), w.next_unit, w.next_follow] {
            lines.extend(hanging(Span::raw("- "), item, Style::new(), cells));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

#[expect(
    clippy::too_many_lines,
    reason = "the parts of one screen, top to bottom"
)]
fn draw_install(frame: &mut Frame<'_>, pen: &Pen<'_>, progress: &Progress, area: Rect) {
    let w = pen.words;
    let outer = pen.block(w.install_title);
    let inner = outer.inner(area).inner(Margin::new(1, 0));
    frame.render_widget(outer, area);
    let cells = usize::from(inner.width);

    let mut lines = Vec::new();
    let elapsed = (w.install_elapsed)(&i18n::duration(w, progress.elapsed));
    let mut top = vec![Span::raw(elapsed)];
    if !progress.unit.is_empty() {
        top.push(Span::styled(
            format!("   {}", (w.install_unit)(&progress.unit)),
            dim(),
        ));
    }
    lines.push(Line::from(top));
    lines.push(Line::raw(""));

    let last = progress.steps.len().saturating_sub(1);
    let running = progress.end.is_none();
    if progress.steps.is_empty() && running {
        lines.push(Line::styled(
            format!("> {}", w.install_starting),
            bold().fg(Color::Cyan),
        ));
    }
    for (i, step) in progress.steps.iter().enumerate() {
        let text = truncate(step, cells.saturating_sub(2));
        lines.push(if i == last && running {
            Line::styled(format!("> {text}"), bold().fg(Color::Cyan))
        } else if i == last && matches!(progress.end, Some(End::Failed(_))) {
            Line::from(vec![
                Span::styled("x ", colour(Color::Red).add_modifier(Modifier::BOLD)),
                Span::raw(text),
            ])
        } else {
            Line::from(vec![
                Span::styled(format!("{} ", pen.ok()), colour(Color::Green)),
                Span::raw(text),
            ])
        });
    }
    if let Some(download) = progress.download {
        let mut text = size(download.done);
        if let Some(total) = download.total {
            text = format!("{text} / {}", size(total));
        }
        text = format!("  {text}   {}/s", size(download.rate));
        lines.push(Line::styled(text, dim()));
        if let Some(total) = download.total.filter(|t| *t > 0) {
            // Coloured spaces, not block glyphs: U+2588 is ambiguous width and takes two cells
            // in some CJK terminals.
            let percent = (download.done.min(total) * 100 / total).min(100);
            let label = format!(" {percent}%");
            let bar = cells.saturating_sub(2 + width(&label));
            let filled = usize::try_from(percent)
                .map_or(0, |p| bar * p / 100)
                .min(bar);
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(" ".repeat(filled), Style::new().bg(Color::Cyan)),
                Span::styled("-".repeat(bar - filled), dim()),
                Span::styled(label, bold()),
            ]));
        }
    }
    match &progress.end {
        Some(End::Done) => {
            lines.push(Line::raw(""));
            lines.push(Line::styled(w.install_done, bold().fg(Color::Green)));
        }
        Some(End::Failed(error)) => {
            lines.push(Line::raw(""));
            lines.push(Line::styled(w.install_failed, bold().fg(Color::Red)));
            lines.extend(hanging(Span::raw(""), error, colour(Color::Red), cells));
        }
        None => {}
    }

    let used = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let [top_area, box_area] =
        Layout::vertical([Constraint::Length(used), Constraint::Fill(1)]).areas(inner);
    frame.render_widget(Paragraph::new(lines), top_area);
    if box_area.height < 3 {
        return;
    }

    let (title, text): (&str, &[String]) = match progress.end {
        Some(End::Done) => (w.result_title, &progress.result),
        Some(End::Failed(_)) | None => (w.log_title, &progress.log),
    };
    let block = pen.block(title).border_style(dim());
    let box_inner = block.inner(box_area);
    frame.render_widget(block, box_area);
    let box_cells = usize::from(box_inner.width);
    let rows = usize::from(box_inner.height);
    let shown: Vec<Line<'_>> = if progress.end == Some(End::Done) {
        let mut shown: Vec<Line<'_>> = text
            .iter()
            .flat_map(|line| {
                let rest = line.trim_start();
                let indent = line.len() - rest.len();
                hanging(Span::raw(" ".repeat(indent)), rest, Style::new(), box_cells)
            })
            .collect();
        if shown.len() > rows {
            shown.truncate(rows.saturating_sub(1));
            shown.push(Line::styled(truncate(w.result_more, box_cells), dim()));
        }
        shown
    } else {
        text.iter()
            .skip(text.len().saturating_sub(rows))
            .map(|line| Line::styled(truncate(line, box_cells), dim()))
            .collect()
    };
    frame.render_widget(Paragraph::new(shown), box_inner);
}

fn draw_picker(frame: &mut Frame<'_>, pen: &Pen<'_>, app: &App, row: usize) {
    let w = pen.words;
    let area = frame.area();
    let popup_width = 46.min(area.width);
    let height = u16::try_from(Lang::ALL.len()).unwrap_or(3) + 4;
    let popup = Rect::new(
        area.x + (area.width - popup_width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        popup_width,
        height.min(area.height),
    );
    frame.render_widget(Clear, popup);
    let outer = pen
        .block(w.language_title)
        .border_style(colour(Color::Cyan));
    let inner = outer.inner(popup).inner(Margin::new(1, 0));
    frame.render_widget(outer, popup);

    let cells = usize::from(inner.width);
    let mut lines = Vec::new();
    for (i, lang) in Lang::ALL.iter().enumerate() {
        let marker = if *lang == app.lang { "(*)" } else { "( )" };
        let blocked = app.no_cjk.is_some() && *lang != Lang::En;
        // A terminal that cannot draw Chinese shows the language's code instead of its name.
        let text = if blocked {
            format!(
                "{marker} {}  {}",
                lang.language_env(),
                w.language_needs_utf8
            )
        } else {
            format!("{marker} {}", lang.native_name())
        };
        let mut style = if blocked { dim() } else { Style::new() };
        if i == row {
            style = style.add_modifier(Modifier::REVERSED);
        }
        lines.push(Line::styled(pad(&truncate(&text, cells), cells), style));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(w.language_hint, dim()));
    frame.render_widget(Paragraph::new(lines), inner);
}
