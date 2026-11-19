//! The install screens: `zeatend` with no command, in English, 繁體中文 or 简体中文.
//!
//! Four screens lead to an install: Server (the host, the distribution and the host checks),
//! Access (the address and names of the API server), Review (the plan) and Install. The choices
//! go into the configuration file, and the install runs in a systemd unit as `zeatend apply`
//! does, so it goes on when the SSH session ends. When an install unit already runs, the screens
//! open on the Install screen and follow it.

mod app;
mod draw;
mod host;
#[cfg(test)]
mod shots;
#[cfg(test)]
mod svg;
mod text;

use std::io::IsTerminal as _;
use std::ops::ControlFlow;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyEvent, KeyEventKind};
use tokio::sync::mpsc::{self, UnboundedSender};
use zeatend_host::progress::Event;
use zeatend_host::{files, journal, preflight, systemd};
use zeatend_plan::config::{Config, Document};

use self::app::{Action, App, Choices, End};
use crate::i18n::{self, Lang, Screens};
use crate::{InstallError, load, output, unit};

/// Opens the screens, and prints the result when they close.
pub(crate) async fn run(config: &Path) -> ExitCode {
    let (lang, no_cjk) = i18n::from_process_env();
    let lang = if no_cjk.is_some() { Lang::En } else { lang };
    let words = lang.messages().screens;
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        output::err(words.no_terminal);
        return ExitCode::from(2);
    }
    let app = match prepare(config, lang, no_cjk, words).await {
        Ok(app) => app,
        Err(error) => {
            output::err(&format!("error: {error}"));
            return ExitCode::FAILURE;
        }
    };
    match show(app, config).await {
        Ok((app, action)) => finish(&app, &action),
        Err(error) => {
            output::err(&format!("error: {error}"));
            ExitCode::FAILURE
        }
    }
}

/// The screens to open: the Install screen when an install unit runs, else the Server screen.
async fn prepare(
    config: &Path,
    lang: Lang,
    no_cjk: Option<i18n::NoCjk>,
    words: &Screens,
) -> Result<App, InstallError> {
    if !preflight::is_root() {
        return Err(InstallError::ScreensNeedRoot(lang));
    }
    if !systemd::booted() {
        return Err(InstallError::ScreensNeedSystemd(lang));
    }
    let running = systemd::running_services(&format!("{}*.service", unit::PREFIX)).await?;
    if let Some(unit) = running.first() {
        return Ok(App::follow(lang, no_cjk, unit));
    }
    let (document, source) = load::document_or_defaults(config)?;
    let Document::Install(install) = document else {
        let path = match source {
            load::Source::Installer => zeatend_plan::installer::CONFIG_FILE.to_owned(),
            load::Source::File | load::Source::Defaults => config.display().to_string(),
        };
        return Err(InstallError::ScreensJoin { lang, path });
    };
    output::err(words.reading_host);
    let host = host::read(install.host.require_encrypted_disk).await?;
    let choices = Choices::new(host, install, config.display().to_string(), config.exists());
    Ok(App::new(lang, no_cjk, choices))
}

/// Shows the screens until they close; returns them and how they closed.
async fn show(mut app: App, config: &Path) -> Result<(App, Action), InstallError> {
    let mut terminal = ratatui::try_init().map_err(InstallError::Terminal)?;
    let action = event_loop(&mut terminal, &mut app, config).await;
    ratatui::restore();
    Ok((app, action?))
}

/// A key press, or the terminal changed size.
#[derive(Debug)]
enum Input {
    Key(KeyEvent),
    Resize,
}

async fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    config: &Path,
) -> Result<Action, InstallError> {
    let (input_tx, mut inputs) = mpsc::unbounded_channel();
    // crossterm reads the terminal with blocking calls, so a thread of its own does.
    std::thread::spawn(move || read_input(&input_tx));
    let (line_tx, mut lines) = mpsc::unbounded_channel();
    if !app.progress.unit.is_empty() {
        follow(app.progress.unit.clone(), line_tx.clone());
    }
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        app.progress.tick(now());
        terminal
            .draw(|frame| {
                draw::draw(frame, app);
            })
            .map_err(InstallError::Terminal)?;
        tokio::select! {
            input = inputs.recv() => {
                let Some(input) = input else {
                    return Ok(Action::Quit);
                };
                let Input::Key(key) = input else {
                    continue;
                };
                match app.on_key(key) {
                    None => {}
                    Some(Action::Install(choices)) => {
                        match install(config, &choices, app.lang).await {
                            Ok(unit) => {
                                app.start(&unit);
                                follow(unit, line_tx.clone());
                            }
                            Err(error) => app.not_started(error.to_string()),
                        }
                    }
                    Some(action) => return Ok(action),
                }
            }
            Some(line) = lines.recv() => match line {
                Some(text) => app.progress.on_line(&text),
                None => app.progress.ended(),
            },
            _ = tick.tick() => {}
        }
    }
}

/// Sends each key press and resize to `tx` until the screens close.
fn read_input(tx: &UnboundedSender<Input>) {
    while let Ok(read) = event::read() {
        let input = match read {
            TermEvent::Key(key) if key.kind == KeyEventKind::Press => Input::Key(key),
            TermEvent::Resize(..) => Input::Resize,
            TermEvent::Key(_)
            | TermEvent::FocusGained
            | TermEvent::FocusLost
            | TermEvent::Mouse(_)
            | TermEvent::Paste(_) => continue,
        };
        if tx.send(input).is_err() {
            break;
        }
    }
}

/// Sends each line `unit` wrote to `tx`, then `None` when it ended.
fn follow(unit: String, tx: UnboundedSender<Option<String>>) {
    tokio::spawn(async move {
        let followed = journal::follow(&unit, |text| {
            let last = Event::parse(text).is_some_and(|e| e.is_last());
            if tx.send(Some(text.to_owned())).is_err() || last {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .await;
        if let Err(error) = followed {
            drop(tx.send(Some(error.to_string())));
        }
        drop(tx.send(None));
    });
}

/// Writes the configuration to `path` and starts the install unit, which reports in `lang`.
async fn install(path: &Path, config: &Config, lang: Lang) -> Result<String, InstallError> {
    let yaml = serde_saphyr::to_string(config).map_err(|e| InstallError::Serialize {
        what: "configuration",
        reason: e.to_string(),
    })?;
    files::write(path, 0o600, yaml.as_bytes())?;
    unit::start(path, Some(lang.language_env())).await
}

/// Seconds since the Unix epoch.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Prints what the closed screens leave to the terminal, and the exit code.
fn finish(app: &App, action: &Action) -> ExitCode {
    let words = app.lang.messages().screens;
    match (action, &app.progress.end) {
        (Action::Leave, _) => {
            output::err(&(words.left)(&app.progress.unit));
            // 128 + SIGINT, as a shell reports Ctrl-C.
            ExitCode::from(130)
        }
        (Action::Close, Some(End::Done)) => {
            for line in &app.progress.result {
                output::stdout(line);
            }
            ExitCode::SUCCESS
        }
        (Action::Close, Some(End::Failed(error))) => {
            output::err(&format!("error: {error}"));
            ExitCode::FAILURE
        }
        (Action::Quit | Action::Install(_) | Action::Close, _) => ExitCode::from(130),
    }
}
