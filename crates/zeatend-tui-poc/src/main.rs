//! Throwaway proof of concept: the Zeatend install screens in four languages, with mock data.
//!
//! `zeatend-tui-poc` runs the screens in this terminal. `--lang <tag>` overrides the locale,
//! `--ascii` acts as if the terminal cannot show CJK, and `--shots <dir>` writes every screen in
//! every language as SVG instead.

mod app;
mod i18n;
mod mock;
mod svg;
mod text;
mod ui;

use std::path::Path;
use std::time::{Duration, Instant};

use app::{App, Screen};
use i18n::{LANGS, Lang, NoCjk};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

fn main() -> std::io::Result<()> {
    let mut lang = i18n::lang_from_env();
    let mut no_cjk = i18n::cjk_blocker_from_env();
    let mut shots = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lang" => {
                lang = args
                    .next()
                    .and_then(|t| Lang::from_tag(&t))
                    .unwrap_or(Lang::En);
            }
            "--ascii" => no_cjk = Some(NoCjk::NotUtf8),
            "--shots" => shots = args.next(),
            _ => {
                eprintln!(
                    "usage: zeatend-tui-poc [--lang en|zh-TW|zh-CN|ja] [--ascii] [--shots <dir>]"
                );
                std::process::exit(2);
            }
        }
    }
    if let Some(dir) = shots {
        return write_shots(Path::new(&dir));
    }
    run(App::new(lang, no_cjk))
}

fn run(mut app: App) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let mut last_tick = Instant::now();
    let result = loop {
        if let Err(err) = terminal.draw(|frame| {
            ui::draw(frame, &app);
        }) {
            break Err(err);
        }
        match event::poll(Duration::from_millis(100)) {
            Ok(true) => match event::read() {
                Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => app.on_key(key),
                Ok(_) => {}
                Err(err) => break Err(err),
            },
            Ok(false) => {}
            Err(err) => break Err(err),
        }
        // The fake install runs ten times faster than the clock.
        if last_tick.elapsed() >= Duration::from_millis(100) {
            app.tick(1);
            last_tick = Instant::now();
        }
        if app.quit {
            break Ok(());
        }
    };
    ratatui::restore();
    result
}

fn press(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
}

const fn sample_name(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Avery Chen",
        Lang::ZhTw => "陳怡君",
        Lang::ZhCn => "王小明",
        Lang::Ja => "山田花子",
    }
}

const fn long_name(lang: Lang) -> &'static str {
    match lang {
        Lang::En => {
            "Avery Chen, platform team, Taipei office, on call this week, ask in the team chat first"
        }
        Lang::ZhTw => {
            "陳怡君（台北辦公室平台組，這週輪值，有事請先在群組裡找我，緊急狀況再打電話）"
        }
        Lang::ZhCn => "王小明（北京办公室平台组，这周值班，有事请先在群里找我，紧急情况再打电话）",
        Lang::Ja => {
            "山田花子（東京オフィスのプラットフォームチーム、今週の当番、急ぎの用件は電話で）"
        }
    }
}

/// Every scenario the shots cover, built by sending the same keys a person would.
fn scenario(name: &str, lang: Lang, no_cjk: Option<NoCjk>) -> App {
    let mut app = App::new(lang, no_cjk);
    match name {
        "server" => {}
        "language" => press(&mut app, KeyCode::F(2)),
        "access" | "review" | "review-long" | "install" | "install-done" => {
            press(&mut app, KeyCode::Enter);
            press(&mut app, KeyCode::Tab);
            press(&mut app, KeyCode::Tab);
            if name == "review-long" {
                type_text(&mut app, long_name(lang));
            } else {
                type_text(&mut app, sample_name(lang));
            }
            if name != "access" {
                press(&mut app, KeyCode::Enter);
            }
            if name.starts_with("install") {
                press(&mut app, KeyCode::Enter);
                app.tick(if name == "install-done" {
                    mock::total_seconds()
                } else {
                    60
                });
            }
        }
        _ => unreachable!("unknown scenario {name}"),
    }
    debug_assert!(name != "access" || app.screen == Screen::Access);
    app
}

fn shoot(app: &App, cols: u16, rows: u16, theme: svg::Theme) -> String {
    let Ok(mut terminal) = Terminal::new(TestBackend::new(cols, rows));
    let mut cursor = None;
    let Ok(_) = terminal.draw(|frame| cursor = ui::draw(frame, app));
    svg::render(
        terminal.backend().buffer(),
        cursor,
        theme,
        app.catalog.lang().tag(),
    )
}

fn write_shots(dir: &Path) -> std::io::Result<()> {
    const SCENARIOS: [&str; 7] = [
        "language",
        "server",
        "access",
        "review",
        "review-long",
        "install",
        "install-done",
    ];
    for (cols, rows) in [(80, 24), (120, 36)] {
        let size_dir = dir.join(format!("{cols}x{rows}"));
        std::fs::create_dir_all(&size_dir)?;
        for lang in LANGS {
            for name in SCENARIOS {
                let app = scenario(name, lang, None);
                let file = size_dir.join(format!("{}-{name}.svg", lang.tag()));
                std::fs::write(file, shoot(&app, cols, rows, svg::Theme::Auto))?;
            }
        }
    }
    let themes_dir = dir.join("themes");
    std::fs::create_dir_all(&themes_dir)?;
    for (theme, suffix) in [(svg::Theme::Light, "light"), (svg::Theme::Dark, "dark")] {
        for lang in LANGS {
            let app = scenario("install", lang, None);
            let file = themes_dir.join(format!("{}-install-{suffix}.svg", lang.tag()));
            std::fs::write(file, shoot(&app, 80, 24, theme))?;
        }
    }
    let console_dir = dir.join("console");
    std::fs::create_dir_all(&console_dir)?;
    for name in ["server", "language", "install"] {
        let app = scenario(name, Lang::ZhTw, Some(NoCjk::LinuxConsole));
        let file = console_dir.join(format!("{name}.svg"));
        std::fs::write(file, shoot(&app, 80, 24, svg::Theme::Dark))?;
    }
    Ok(())
}
