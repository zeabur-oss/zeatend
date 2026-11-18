//! `apply` in a transient systemd unit, so the install goes on when the SSH session ends.
//!
//! The unit runs `zeatend apply --foreground --json`, and the journal keeps the progress it
//! prints. This process, and any later `apply`, follows the unit's journal and shows the same
//! progress. There is no state of our own: systemd knows whether the unit runs, and the journal
//! holds its output.

use std::ops::ControlFlow;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use zeatend_host::journal::{self, Followed};
use zeatend_host::progress::Event;
use zeatend_host::systemd;

use crate::{InstallError, output};

/// The start of the name of an install unit; the rest is its id and `.service`.
const PREFIX: &str = "zeatend-install-";

/// The environment variables the unit gets from this process. A unit starts with an empty
/// environment, and downloads need the proxy.
const PASSED_ENV: [&str; 8] = [
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "all_proxy",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
];

/// Follows the install unit that runs, or starts one and follows it.
pub(crate) async fn apply(config: &Path) -> Result<(), InstallError> {
    let running = systemd::running_services(&format!("{PREFIX}*.service")).await?;
    let unit = if let Some(unit) = running.into_iter().next() {
        output::note(&format!("An install runs in {unit}; following it."));
        unit
    } else {
        start(config).await?
    };
    follow(&unit).await
}

/// The command line the unit runs.
pub(crate) const fn command<'a>(zeatend: &'a str, config: &'a str) -> [&'a str; 6] {
    [
        zeatend,
        "--config",
        config,
        "apply",
        "--foreground",
        "--json",
    ]
}

/// Starts `zeatend apply` in a new unit and returns the unit's name.
async fn start(config: &Path) -> Result<String, InstallError> {
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let unit = format!("{PREFIX}{id}.service");
    let zeatend = std::env::current_exe()
        .map_err(|e| InstallError::Message(format!("finding the zeatend binary: {e}")))?;
    // The unit starts in `/`.
    let config = std::path::absolute(config)
        .map_err(|e| InstallError::Message(format!("{}: {e}", config.display())))?;
    let utf8 = |path: &Path| {
        path.to_str()
            .map(str::to_owned)
            .ok_or_else(|| InstallError::Message(format!("{}: not UTF-8", path.display())))
    };
    let (zeatend, config) = (utf8(&zeatend)?, utf8(&config)?);
    let env: Vec<String> = PASSED_ENV
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| format!("{name}={value}"))
        })
        .collect();
    systemd::run_transient(&unit, "zeatend apply", &env, &command(&zeatend, &config)).await?;
    output::note(&format!(
        "The install runs in {unit}. If this session ends, run `sudo zeatend apply` to follow it again."
    ));
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
        (Some(Event::Failed { error }), _) => Err(InstallError::Message(error)),
        (Some(_), _) => Ok(()),
        (None, Followed::Stopped | Followed::Ended) => Err(InstallError::Message(format!(
            "{unit} ended without a result; see `journalctl -u {unit}`"
        ))),
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
