//! The pictures of the screens in `docs/screens`, drawn from the same keys a person presses on a
//! made-up host. `ZEATEND_WRITE_SCREENS=1 cargo nextest run shots` writes them again; otherwise
//! the test fails when a picture differs from what the screens draw.

use std::path::{Path, PathBuf};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use zeatend_host::preflight::{Check, Finding};
use zeatend_host::progress::Event;
use zeatend_plan::cluster::Distribution;
use zeatend_plan::config::{Component, Config};
use zeatend_plan::plan::Facts;

use super::app::{Action, App, Choices};
use super::host::Host;
use super::{draw, svg};
use crate::i18n::{Lang, NoCjk, Screen};

const UNIT: &str = "zeatend-install-1794991689-3fa2c1.service";

fn facts() -> Facts {
    Facts {
        arch: "amd64".to_owned(),
        hostname: "edge-1".to_owned(),
        node_name: None,
        host_hex: "3fa2c1".to_owned(),
        memory_mib: 8 * 1024,
        node_ips: vec!["10.0.0.5".to_owned()],
        record: None,
        kubernetes: None,
        apparmor_restricts_unconfined: false,
        k3s_own_network: false,
        installer_webhook: false,
    }
}

fn host() -> Host {
    let checks = vec![
        Check::from(Finding::RootOk),
        Check::from(Finding::SwapOff),
        Check::from(Finding::CryptNo {
            path: "/var/lib/rancher",
        }),
    ];
    Host {
        hostname: "edge-1".to_owned(),
        system: "Ubuntu 26.04 LTS".to_owned(),
        cpus: 4,
        memory_mib: 8 * 1024,
        addresses: vec!["10.0.0.5".to_owned()],
        checks: vec![
            (Distribution::KubeSolo, checks.clone()),
            (Distribution::K3s, checks.clone()),
            (Distribution::Rke2, checks),
        ],
        facts: facts(),
    }
}

fn press(app: &mut App, code: KeyCode) -> Option<Action> {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
}

/// The lines the install unit writes in `lang`, up to a download half done; with `done`, to the
/// end.
fn install_lines(lang: Lang, done: bool) -> Vec<Event> {
    let step = |text: &str| Event::Step {
        text: text.to_owned(),
    };
    let note = |text: &str| Event::Note {
        text: text.to_owned(),
    };
    let mut events = vec![
        step(lang.messages().checking_host),
        note(&lang.messages().check_line(&Finding::SwapOff)),
        step("K3s v1.37.2+k3s1 (k3s)"),
        note("https://github.com/k3s-io/k3s/releases/download/v1.37.2%2Bk3s1/k3s"),
        Event::Download {
            done: 40 * 1024 * 1024,
            total: Some(74 * 1024 * 1024),
            rate: 12 * 1024 * 1024,
        },
    ];
    if done {
        events.extend([
            Event::DownloadEnd,
            step("Waiting for node edge-1 to be Ready"),
            step("Done"),
            Event::Line {
                text: "K3s v1.37.2+k3s1 runs on edge-1. See the nodes with:\n  sudo \
                       /usr/local/bin/kubectl --kubeconfig /etc/rancher/k3s/k3s.yaml get nodes \
                       -o wide"
                    .to_owned(),
            },
            Event::Done,
        ]);
    }
    events
}

/// The screens after the keys of `scenario`.
fn scenario(name: &str, lang: Lang, no_cjk: Option<NoCjk>) -> App {
    let choices = Choices::new(
        host(),
        Config::default(),
        "/etc/zeatend/config.yaml".to_owned(),
        false,
    );
    let mut app = App::new(lang, no_cjk, choices);
    match name {
        "server" => {}
        "language" => {
            press(&mut app, KeyCode::F(2));
        }
        "access" | "review" | "install" | "done" => {
            press(&mut app, KeyCode::Enter);
            press(&mut app, KeyCode::Tab);
            type_text(&mut app, "k8s.example.com");
            if name != "access" {
                press(&mut app, KeyCode::Enter);
            }
            if name == "install" || name == "done" {
                let action = press(&mut app, KeyCode::Enter);
                assert!(matches!(action, Some(Action::Install(_))), "{action:?}");
                app.start(UNIT);
                app.progress.tick(1_794_991_689 + 125);
                for event in install_lines(app.lang, name == "done") {
                    app.progress.on_event(event);
                }
            }
        }
        _ => unreachable!("no scenario {name}"),
    }
    app
}

/// The screens drawn at `cols` by `rows`, as SVG.
fn shoot(app: &App, cols: u16, rows: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    let mut cursor = None;
    terminal
        .draw(|frame| cursor = draw::draw(frame, app))
        .unwrap();
    let tag = match app.lang {
        Lang::En => "en",
        Lang::ZhTw => "zh-TW",
        Lang::ZhCn => "zh-CN",
    };
    svg::render(terminal.backend().buffer(), cursor, tag)
}

fn docs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screens")
}

/// Every picture: its path under `docs/screens` and its SVG.
fn pictures() -> Vec<(String, String)> {
    const SCENARIOS: [&str; 6] = ["server", "language", "access", "review", "install", "done"];
    let mut pictures = Vec::new();
    for (cols, rows) in [(80, 24), (120, 36)] {
        for lang in Lang::ALL {
            for name in SCENARIOS {
                let app = scenario(name, lang, None);
                pictures.push((
                    format!("{cols}x{rows}/{}-{name}.svg", lang.language_env()),
                    shoot(&app, cols, rows),
                ));
            }
        }
    }
    for name in ["server", "language", "install"] {
        let app = scenario(name, Lang::ZhTw, Some(NoCjk::LinuxConsole));
        pictures.push((format!("console/{name}.svg"), shoot(&app, 80, 24)));
    }
    pictures
}

#[test]
fn the_pictures_in_docs_are_what_the_screens_draw() {
    let write = std::env::var_os("ZEATEND_WRITE_SCREENS").is_some();
    let mut stale = Vec::new();
    for (path, svg) in pictures() {
        let file = docs().join(&path);
        if write {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, svg).unwrap();
        } else if std::fs::read_to_string(&file).ok().as_deref() != Some(svg.as_str()) {
            stale.push(path);
        }
    }
    assert!(
        stale.is_empty(),
        "run ZEATEND_WRITE_SCREENS=1 cargo nextest run shots; stale: {stale:?}"
    );
}

/// The text of the screens drawn at `cols` by `rows`, one line for each row.
fn screen_text(app: &App, cols: u16, rows: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    terminal
        .draw(|frame| {
            draw::draw(frame, app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..rows {
        let mut x = 0;
        while x < cols {
            let symbol = buffer[(x, y)].symbol();
            text.push_str(symbol);
            x += u16::try_from(super::text::width(symbol).max(1)).unwrap();
        }
        text.push('\n');
    }
    text
}

#[test]
fn a_terminal_without_cjk_stays_in_english_with_ascii_borders() {
    let mut app = scenario("server", Lang::ZhTw, Some(NoCjk::LinuxConsole));
    assert_eq!(app.lang, Lang::En);
    press(&mut app, KeyCode::F(2));
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.lang, Lang::En, "the picker does not choose Chinese");
    for name in ["server", "language", "install"] {
        let app = scenario(name, Lang::ZhTw, Some(NoCjk::NotUtf8));
        let text = screen_text(&app, 80, 24);
        assert!(text.is_ascii(), "{name}:\n{text}");
        if name == "server" {
            assert!(text.contains("This terminal is not set to UTF-8"), "{text}");
        }
    }
}

#[test]
fn every_screen_shows_its_last_part_at_80_by_24_in_every_language() {
    for lang in Lang::ALL {
        let w = lang.messages().screens;
        for (name, last) in [
            ("server", w.checks_title),
            ("access", w.field_node),
            ("review", w.review_next),
            ("install", w.log_title),
            ("done", w.result_title),
        ] {
            let text = screen_text(&scenario(name, lang, None), 80, 24);
            assert!(text.contains(last), "{lang:?} {name}:\n{text}");
        }
    }
}

#[test]
fn the_screens_go_back_and_check_what_was_typed() {
    let mut app = scenario("access", Lang::En, None);
    assert_eq!(app.screen, Screen::Access);
    press(&mut app, KeyCode::BackTab);
    for _ in 0..10 {
        press(&mut app, KeyCode::Backspace);
    }
    type_text(&mut app, "not an address");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Access);
    let choices = app.choices.as_ref().unwrap();
    assert_eq!(choices.problem, Some(super::app::FieldKind::Address));

    let mut app = scenario("review", Lang::En, None);
    let config = app.choices.as_ref().unwrap().config();
    assert_eq!(config.distribution, Distribution::K3s);
    // The host's own address is on the certificate already.
    assert_eq!(config.host.tls_san, ["k8s.example.com"]);
    assert_eq!(config.host.node_name, None);
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Server);
    press(&mut app, KeyCode::Up);
    assert_eq!(
        app.choices.as_ref().unwrap().distribution,
        Distribution::KubeSolo
    );
}

#[test]
fn another_distribution_drops_the_release_override() {
    let config = Config {
        k3s_version: Some("v1.37.1+k3s1".to_owned()),
        ..Config::default()
    };
    let choices = Choices::new(host(), config, "/etc/zeatend/config.yaml".to_owned(), true);
    let mut app = App::new(Lang::En, None, choices);
    press(&mut app, KeyCode::Down);
    let config = app.choices.as_ref().unwrap().config();
    assert_eq!(config.distribution, Distribution::Rke2);
    assert_eq!(config.k3s_version, None);
    assert!(config.check().is_ok());
    press(&mut app, KeyCode::Up);
    let config = app.choices.as_ref().unwrap().config();
    assert_eq!(config.k3s_version.as_deref(), Some("v1.37.1+k3s1"));
}

#[test]
fn another_distribution_drops_the_components_it_does_not_package() {
    let config = Config {
        disable: vec![Component::Traefik, Component::LocalStorage],
        ..Config::default()
    };
    let mut choices = Choices::new(host(), config, "/etc/zeatend/config.yaml".to_owned(), true);
    for (distribution, disable) in [
        (Distribution::Rke2, vec![Component::Traefik]),
        (Distribution::KubeSolo, vec![]),
        (
            Distribution::K3s,
            vec![Component::Traefik, Component::LocalStorage],
        ),
    ] {
        choices.distribution = distribution;
        let config = choices.config();
        assert_eq!(config.disable, disable, "{distribution:?}");
        assert!(config.check().is_ok(), "{distribution:?}");
    }
}

#[test]
fn a_failed_check_keeps_the_server_screen() {
    let mut app = scenario("server", Lang::En, None);
    if let Some(choices) = &mut app.choices {
        choices.host.checks = vec![(Distribution::K3s, vec![Check::from(Finding::SystemdNo)])];
    }
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Server);
}

#[test]
fn ctrl_c_leaves_a_running_install_and_closes_an_ended_one() {
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    let mut app = scenario("install", Lang::En, None);
    assert_eq!(app.on_key(ctrl_c), Some(Action::Leave));
    let mut app = scenario("done", Lang::En, None);
    assert_eq!(app.on_key(ctrl_c), Some(Action::Close));
    assert_eq!(press(&mut app, KeyCode::Enter), Some(Action::Close));
    assert_eq!(
        app.progress.result.first().map(String::as_str),
        Some("K3s v1.37.2+k3s1 runs on edge-1. See the nodes with:")
    );
    assert_eq!(app.progress.elapsed, 125);
}
