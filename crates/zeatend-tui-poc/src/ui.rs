//! Drawing. Colours are the 16 ANSI colours plus bold, dim and reverse, so the terminal's
//! own light or dark palette decides the actual colours.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Cell, Clear, Paragraph, Row, Table, Wrap};

use crate::app::{App, Msg, SCREENS, Screen};
use crate::i18n::{LOCALES, Locale, duration, group_thousands};
use crate::mock;
use crate::text::{pad, truncate, width};

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

fn block<'a>(app: &App, title: impl std::fmt::Display) -> Block<'a> {
    let set = if app.no_cjk.is_some() {
        ASCII_BORDER
    } else {
        border::PLAIN
    };
    Block::bordered()
        .border_set(set)
        .title(Span::styled(format!(" {title} "), bold()))
}

const fn ok_mark(app: &App) -> &'static str {
    if app.no_cjk.is_some() { "+" } else { "✓" }
}

/// Draws the app and returns where the text cursor is, if a field has focus.
pub(crate) fn draw(frame: &mut Frame<'_>, app: &App) -> Option<Position> {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_header(frame, app, header);
    let mut cursor = None;
    match app.screen {
        Screen::Server => draw_server(frame, app, body),
        Screen::Access => cursor = draw_access(frame, app, body),
        Screen::Review => draw_review(frame, app, body),
        Screen::Install => draw_install(frame, app, body),
    }
    draw_footer(frame, app, footer);
    if let Some(row) = app.picker {
        draw_picker(frame, app, row);
    }
    if let Some(position) = cursor {
        frame.set_cursor_position(position);
    }
    cursor
}

fn draw_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let m = app.locale.messages();
    frame.render_widget(
        Paragraph::new(Span::styled(format!(" {}", m.app_title), bold())),
        area,
    );
    let mut steps = Vec::new();
    for (i, screen) in SCREENS.iter().enumerate() {
        let label = format!(" {} {} ", i + 1, screen.title(m));
        if *screen == app.screen {
            steps.push(Span::styled(
                label,
                bold().fg(Color::Cyan).add_modifier(Modifier::REVERSED),
            ));
        } else {
            steps.push(Span::styled(label, dim()));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(steps)).alignment(Alignment::Right),
        area,
    );
}

fn draw_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let keys: &[(&str, Msg)] = if app.picker.is_some() {
        &[
            ("Up/Down", |m| m.key_choose),
            ("Enter", |m| m.key_next),
            ("Esc", |m| m.key_back),
        ]
    } else {
        match app.screen {
            Screen::Server => &[
                ("Up/Down", |m| m.key_choose),
                ("Enter", |m| m.key_next),
                ("F2", |m| m.key_lang),
                ("Ctrl+C", |m| m.key_quit),
            ],
            Screen::Access => &[
                ("Tab", |m| m.key_field),
                ("Enter", |m| m.key_next),
                ("Esc", |m| m.key_back),
                ("F2", |m| m.key_lang),
                ("Ctrl+C", |m| m.key_quit),
            ],
            Screen::Review => &[
                ("Enter", |m| m.key_install),
                ("Esc", |m| m.key_back),
                ("F2", |m| m.key_lang),
                ("Ctrl+C", |m| m.key_quit),
            ],
            Screen::Install => &[("F2", |m| m.key_lang), ("Ctrl+C", |m| m.key_quit)],
        }
    };
    let mut spans = vec![Span::raw(" ")];
    for (key, label) in keys {
        spans.push(Span::styled(*key, bold()));
        spans.push(Span::raw(format!(" {}   ", label(app.locale.messages()))));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_server(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let m = app.locale.messages();
    let outer = block(app, m.server_title);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let mut lines = Vec::new();
    if app.no_cjk.is_some() {
        lines.push(Line::styled(
            m.fallback_note,
            Style::new().fg(Color::Yellow),
        ));
        lines.push(Line::raw(""));
    }

    let info = [
        (m.server_host, mock::HOSTNAME.to_owned()),
        (m.server_system, mock::SYSTEM.to_owned()),
        (m.server_cpu, (m.cpu_cores)(mock::CPU_CORES)),
        (m.server_memory, (m.memory_gb)(mock::MEMORY_GB)),
        (
            m.server_disk,
            (m.disk_free)(mock::DISK_FREE_GB, mock::DISK_TOTAL_GB),
        ),
        (m.server_address, (m.addr_private)(mock::PRIVATE_ADDR)),
    ];
    let mut label_width = 0;
    for (label, _) in &info {
        label_width = label_width.max(width(label));
    }
    for (label, value) in info {
        lines.push(Line::from(vec![
            Span::styled(format!(" {}  ", pad(label, label_width)), dim()),
            Span::raw(value),
        ]));
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled(format!(" {}", m.distro_title), bold()));
    for (i, (name, _)) in mock::DISTROS.iter().enumerate() {
        if i == app.distro {
            lines.push(Line::styled(format!(" (*) {name}"), bold().fg(Color::Cyan)));
        } else {
            lines.push(Line::raw(format!(" ( ) {name}")));
        }
    }
    let description = (mock::DISTROS[app.distro].1)(m);
    lines.push(Line::styled(format!("     {description}"), dim()));

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(format!(" {}  ", m.checks_title), bold()),
        Span::styled((m.checks_summary)(1), Style::new().fg(Color::Yellow)),
    ]));
    lines.push(Line::from(vec![
        Span::styled(format!(" {} ", ok_mark(app)), Style::new().fg(Color::Green)),
        Span::raw(m.check_ports),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            " ! ",
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        Span::raw(m.check_swap),
    ]));

    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        inner,
    );
}

/// Rows a wrapped paragraph needs; CJK breaks between any two characters, so this is close.
fn wrapped_rows(s: &str, cells: u16) -> u16 {
    let cells = usize::from(cells.max(1));
    u16::try_from(width(s).div_ceil(cells).max(1)).unwrap_or(u16::MAX)
}

fn draw_access(frame: &mut Frame<'_>, app: &App, area: Rect) -> Option<Position> {
    let mut cursor = None;
    let m = app.locale.messages();
    let outer = block(app, m.access_title);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let note = m.access_private_note;
    let mut constraints = vec![Constraint::Length(wrapped_rows(note, inner.width - 2) + 1)];
    for _ in &app.fields {
        constraints.push(Constraint::Length(3));
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Fill(1));
    let rows = Layout::vertical(constraints).split(inner);

    frame.render_widget(
        Paragraph::new(note)
            .wrap(Wrap { trim: false })
            .block(Block::new().padding(ratatui::widgets::Padding::horizontal(1))),
        rows[0],
    );

    for (i, field) in app.fields.iter().enumerate() {
        let focused = i == app.focus;
        let box_area = rows[1 + i * 2].inner(ratatui::layout::Margin::new(1, 0));
        let mut field_block = block(app, (field.label)(m));
        if focused {
            field_block = field_block.border_style(Style::new().fg(Color::Cyan));
        } else {
            field_block = field_block.border_style(dim());
        }
        let text_area = field_block.inner(box_area);
        frame.render_widget(field_block, box_area);

        // Scroll left so the cursor stays inside the box.
        let mut before: Vec<char> = field.before_cursor().chars().collect();
        let mut skipped = 0;
        while width(&before.iter().collect::<String>()) + 1 > usize::from(text_area.width) {
            before.remove(0);
            skipped += 1;
        }
        let visible: String = field.value.chars().skip(skipped).collect();
        frame.render_widget(Paragraph::new(visible), text_area);
        if focused && app.picker.is_none() {
            let x =
                text_area.x + u16::try_from(width(&before.iter().collect::<String>())).unwrap_or(0);
            cursor = Some(Position::new(x, text_area.y));
        }

        if let Some(hint) = field.hint {
            let hint = truncate(hint(m), usize::from(inner.width - 2));
            frame.render_widget(
                Paragraph::new(Span::styled(format!("  {hint}"), dim())),
                rows[2 + i * 2],
            );
        }
    }
    cursor
}

fn draw_review(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let m = app.locale.messages();
    let outer = block(app, m.review_title);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let labels = [
        m.review_distro,
        m.review_address,
        m.review_domain,
        m.review_name,
    ];
    let mut label_width = 0;
    for label in &labels {
        label_width = label_width.max(width(label));
    }
    let value_width = usize::from(inner.width).saturating_sub(label_width + 4);

    let domain = &app.fields[1].value;
    let domain_cell = if domain.is_empty() {
        Cell::from(Span::styled(m.review_no_domain, dim()))
    } else {
        Cell::from(truncate(domain, value_width))
    };
    let values = [
        Cell::from(mock::DISTROS[app.distro].0),
        Cell::from(truncate(&app.fields[0].value, value_width)),
        domain_cell,
        Cell::from(truncate(&app.fields[2].value, value_width)),
    ];
    let mut rows = Vec::new();
    for (label, value) in labels.into_iter().zip(values) {
        rows.push(Row::new(vec![
            Cell::from(Span::styled(label, dim())),
            value,
        ]));
    }
    let table = Table::new(
        rows,
        [
            Constraint::Length(u16::try_from(label_width).unwrap_or(10)),
            Constraint::Fill(1),
        ],
    )
    .column_spacing(2);

    let [table_area, _, plan_area] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(inner.inner(ratatui::layout::Margin::new(1, 0)));
    frame.render_widget(table, table_area);

    let plan = vec![
        Line::styled(m.review_plan_title, bold()),
        Line::raw(format!("- {}", (m.plan_download)(mock::DOWNLOAD_GB))),
        Line::raw(format!("- {}", (m.plan_disk)(mock::DISK_USE_GB))),
        Line::raw(format!("- {}", (m.plan_time)(mock::total_seconds() / 60))),
        Line::raw(format!("- {}", m.plan_restart)),
    ];
    frame.render_widget(Paragraph::new(plan).wrap(Wrap { trim: false }), plan_area);
}

fn draw_install(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let m = app.locale.messages();
    let outer = block(app, m.install_title);
    let inner = outer.inner(area).inner(ratatui::layout::Margin::new(1, 0));
    frame.render_widget(outer, area);

    let [steps_area, _, gauge_area, time_area, _, log_area] = Layout::vertical([
        Constraint::Length(u16::try_from(mock::STEPS.len()).unwrap_or(6)),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(inner);

    let progress = mock::progress(app.elapsed);
    let mut names = Vec::new();
    let mut name_width = 0;
    for (label, _) in mock::STEPS {
        let name = label(m);
        name_width = name_width.max(width(name));
        names.push(name);
    }
    let mut lines = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let seconds = mock::STEPS[i].1;
        let (mark, style, detail) = if i < progress.step {
            (
                ok_mark(app),
                Style::new().fg(Color::Green),
                duration(m, seconds),
            )
        } else if i == progress.step {
            let detail = if i == 1 {
                (m.install_downloaded)(
                    &group_thousands(mock::downloaded_mb(progress.into_step)),
                    &group_thousands(mock::DOWNLOAD_MB),
                )
            } else {
                duration(m, progress.into_step)
            };
            (">", bold().fg(Color::Cyan), detail)
        } else {
            (" ", dim(), String::new())
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{mark} "), style),
            Span::styled(
                pad(name, name_width + 3),
                if i == progress.step {
                    bold()
                } else {
                    Style::new()
                },
            ),
            Span::styled(detail, dim()),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), steps_area);

    let total = mock::total_seconds();
    let percent = u16::try_from(app.elapsed * 100 / total).unwrap_or(100);
    // Coloured spaces instead of block glyphs: U+2588 is ambiguous width and doubles in some CJK
    // terminals.
    let label = format!(" {percent}%");
    let bar_width = usize::from(gauge_area.width).saturating_sub(width(&label));
    let filled = bar_width * usize::from(percent) / 100;
    let bar = Line::from(vec![
        Span::styled(" ".repeat(filled), Style::new().bg(Color::Cyan)),
        Span::styled("-".repeat(bar_width - filled), dim()),
        Span::styled(label, bold()),
    ]);
    frame.render_widget(Paragraph::new(bar), gauge_area);

    let time_line = if progress.step >= mock::STEPS.len() {
        Line::styled(
            (m.install_done)("~/.kube/zeatend-k3s.yaml"),
            bold().fg(Color::Green),
        )
    } else {
        Line::from(vec![
            Span::raw((m.install_elapsed)(&duration(m, app.elapsed))),
            Span::raw("    "),
            Span::styled((m.install_left)(&duration(m, total - app.elapsed)), dim()),
        ])
    };
    frame.render_widget(Paragraph::new(time_line), time_area);

    let log_block = block(app, m.log_title).border_style(dim());
    let log_inner = log_block.inner(log_area);
    frame.render_widget(log_block, log_area);
    let all = mock::log_until(app.elapsed);
    let shown = usize::from(log_inner.height);
    let mut log_lines = Vec::new();
    for line in all.iter().skip(all.len().saturating_sub(shown)) {
        log_lines.push(Line::styled(
            truncate(line, usize::from(log_inner.width)),
            dim(),
        ));
    }
    frame.render_widget(Paragraph::new(log_lines), log_inner);
}

fn draw_picker(frame: &mut Frame<'_>, app: &App, row: usize) {
    let m = app.locale.messages();
    let area = frame.area();
    let w = 46.min(area.width);
    let h = u16::try_from(LOCALES.len()).unwrap_or(4) + 4;
    let popup = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    );
    frame.render_widget(Clear, popup);
    let outer = block(app, m.lang_title).border_style(Style::new().fg(Color::Cyan));
    let inner = outer.inner(popup).inner(ratatui::layout::Margin::new(1, 0));
    frame.render_widget(outer, popup);

    let mut lines = Vec::new();
    for (i, lang) in LOCALES.iter().enumerate() {
        let current = *lang == app.locale;
        let marker = if current { "(*)" } else { "( )" };
        let blocked = app.no_cjk.is_some() && *lang != Locale::En;
        // The console cannot draw the native names, so show the tag there.
        let name = if blocked {
            lang.tag().to_owned()
        } else {
            lang.native_name().to_owned()
        };
        let mut style = Style::new();
        if blocked {
            style = dim();
        }
        if i == row {
            style = style.add_modifier(Modifier::REVERSED);
        }
        let mut text = format!("{marker} {name}");
        if blocked {
            text.push_str(&format!("  {}", m.lang_needs_utf8));
        }
        lines.push(Line::styled(pad(&text, usize::from(inner.width)), style));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(m.lang_hint, dim()));
    frame.render_widget(Paragraph::new(lines), inner);
}
