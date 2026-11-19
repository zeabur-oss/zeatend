//! `apply` in a transient systemd unit, so the install goes on when the SSH session ends.
//!
//! The unit runs `zeatend apply --foreground --json`, and the journal keeps the progress it
//! prints. When this session has a reviewed plan, the unit gets it as `--plan <file>` and applies
//! that plan, not one it makes later. This process, and any later `apply`, follows the unit's
//! journal and shows the same progress. Besides that file there is no state of our own: systemd
//! knows whether the unit runs, and the journal holds its output.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use zeatend_host::journal::{self, Followed};
use zeatend_host::progress::Event;
use zeatend_host::{files, random, systemd};

use crate::{InstallError, apply, output};

/// The start of the name of an install unit; the rest is its id and `.service`.
pub(crate) const PREFIX: &str = "zeatend-install-";

/// Where the plan of each install unit goes, readable by root only.
const PLAN_DIR: &str = "/run/zeatend";

/// The environment variables the unit gets from this process. A unit does not get the
/// session's environment: downloads need the proxy, and the locale and `TERM` choose the language
/// of the progress (see [`crate::i18n`]).
const PASSED_ENV: [&str; 14] = [
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "TERM",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "all_proxy",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
];

/// Follows the install unit that runs, or starts one and follows it. The unit applies the plan in
/// the file `plan`, or without one the plan it makes.
pub(crate) async fn apply(config: &Path, plan: Option<&Path>) -> Result<(), InstallError> {
    let running = systemd::running_services(&format!("{PREFIX}*.service")).await?;
    let unit = if let Some(unit) = running.into_iter().next() {
        output::note(&(crate::i18n::messages().following_unit)(&unit));
        unit
    } else {
        let plan = match plan {
            Some(path) => {
                let text = std::fs::read(path).map_err(InstallError::io(path))?;
                apply::parse_reviewed(path, &text)?;
                Some(text)
            }
            None => None,
        };
        let unit = start(config, plan.as_deref(), None).await?;
        output::note(&(crate::i18n::messages().started_unit)(&unit));
        unit
    };
    follow(&unit).await
}

/// The command line the unit runs.
pub(crate) fn command<'a>(
    zeatend: &'a str,
    config: &'a str,
    plan: Option<&'a str>,
) -> Vec<&'a str> {
    let mut command = vec![
        zeatend,
        "--config",
        config,
        "apply",
        "--foreground",
        "--json",
    ];
    command.extend(plan.into_iter().flat_map(|plan| ["--plan", plan]));
    command
}

/// Starts `zeatend apply` in a new unit and returns the unit's name. The unit applies `plan`, the
/// JSON of a plan, or without one the plan it makes. `language`, a `LANGUAGE` value, replaces the
/// session's, so the unit reports in the language the screens show.
pub(crate) async fn start(
    config: &Path,
    plan: Option<&[u8]>,
    language: Option<&str>,
) -> Result<String, InstallError> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // `journalctl --unit` shows every run of a unit name, so no two installs share one.
    let id = format!("{PREFIX}{secs}-{}", random::hex(6)?);
    let unit = format!("{id}.service");
    let plan_file = match plan {
        Some(plan) => {
            let path = PathBuf::from(PLAN_DIR).join(format!("{id}.plan.json"));
            files::make_dir(Path::new(PLAN_DIR), 0o700)?;
            files::write(&path, 0o600, plan)?;
            Some(path)
        }
        None => None,
    };
    let zeatend = std::env::current_exe().map_err(InstallError::CurrentExe)?;
    // Relative paths in the configuration, such as `tokenFile`, stay relative to this directory.
    let dir = std::env::current_dir().map_err(InstallError::WorkingDir)?;
    let config = dir.join(config);
    let utf8 = |path: &Path| {
        path.to_str()
            .map(str::to_owned)
            .ok_or_else(|| InstallError::NotUtf8(path.to_owned()))
    };
    let (zeatend, config, dir) = (utf8(&zeatend)?, utf8(&config)?, utf8(&dir)?);
    let plan_file = plan_file.as_deref().map(utf8).transpose()?;
    let mut env: Vec<String> = PASSED_ENV
        .iter()
        .filter(|name| language.is_none() || **name != "LANGUAGE")
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| format!("{name}={value}"))
        })
        .collect();
    env.extend(language.map(|language| format!("LANGUAGE={language}")));
    systemd::run_transient(&systemd::Transient {
        unit: &unit,
        description: "zeatend apply",
        dir: &dir,
        env: &env,
        command: &command(&zeatend, &config, plan_file.as_deref()),
    })
    .await?;
    Ok(unit)
}

/// Shows the progress of `unit` from its first line until it ends.
async fn follow(unit: &str) -> Result<(), InstallError> {
    let mut last = None;
    let followed = tokio::select! {
        followed = journal::follow(unit, |text| show(text, &mut last)) => followed?,
        _ = tokio::signal::ctrl_c() => return Err(InstallError::Detached(unit.to_owned())),
    };
    match (last, followed) {
        (Some(Event::Failed { error }), _) => Err(InstallError::UnitFailed(error)),
        (Some(_), _) => Ok(()),
        (None, Followed::Stopped | Followed::Ended) => {
            Err(InstallError::UnitEnded(unit.to_owned()))
        }
    }
}

/// Shows one line of the unit's output, and stops at the last event. A line that is not an event,
/// such as a panic message, shows as a note.
fn show(text: &str, last: &mut Option<Event>) -> ControlFlow<()> {
    match Event::parse(text) {
        Some(event) if event.is_last() => {
            *last = Some(event);
            ControlFlow::Break(())
        }
        Some(event) => {
            output::emit(&event);
            ControlFlow::Continue(())
        }
        None => {
            output::note(text);
            ControlFlow::Continue(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_event_ends_the_follow() {
        let mut last = None;
        assert_eq!(
            show(r#"{"event":"step","text":"Checking the host"}"#, &mut last),
            ControlFlow::Continue(())
        );
        assert_eq!(show("not an event", &mut last), ControlFlow::Continue(()));
        assert_eq!(last, None);
        assert_eq!(
            show(r#"{"event":"failed","error":"no"}"#, &mut last),
            ControlFlow::Break(())
        );
        assert_eq!(
            last,
            Some(Event::Failed {
                error: "no".to_owned()
            })
        );
    }
}
