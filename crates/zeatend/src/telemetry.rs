//! The experience program (docs/reference/telemetry.mdx): an event when a plan is made, when host
//! checks fail, and when `apply` starts and ends. Each is sent in the background, once, and a
//! failed request changes nothing else.

use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;
use zeatend_host::telemetry::{self as host, ENDPOINT, PROJECT_KEY};
use zeatend_plan::cluster::Distribution;
use zeatend_plan::config::Role;
use zeatend_plan::finding::Finding;
use zeatend_plan::telemetry::{
    Action, Command, Event, InstallationId, Outcome, Run, Step, Switch, Via,
};

use crate::{i18n, output};

/// The file that holds the installation ID, next to the configuration file.
const ID_FILE: &str = "/etc/zeatend/installation-id";

/// What this process knows about the program.
#[derive(Debug)]
struct State {
    via: Via,
    no_telemetry: bool,
    /// The `telemetry` key of the configuration file.
    config: Option<bool>,
    /// The run of `apply` after its host checks passed, and when.
    started: Option<(Run, Instant)>,
    /// The step `apply` is in.
    step: Option<Step>,
    /// The installation ID, made at most once per process.
    id: Option<InstallationId>,
    /// Requests that may still run.
    sending: Vec<JoinHandle<()>>,
}

static STATE: Mutex<State> = Mutex::new(State {
    via: Via::Cli,
    no_telemetry: false,
    config: None,
    started: None,
    step: None,
    id: None,
    sending: Vec::new(),
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Sets how zeatend runs and whether `--no-telemetry` was given.
pub(crate) fn set_up(via: Via, no_telemetry: bool) {
    let mut state = state();
    state.via = via;
    state.no_telemetry = no_telemetry;
}

/// Takes the `telemetry` key of the configuration file.
pub(crate) fn use_config(telemetry: Option<bool>) {
    state().config = telemetry;
}

/// What happens to an event now.
pub(crate) fn switch() -> Switch {
    let (config, no_telemetry) = {
        let state = state();
        (state.config, state.no_telemetry)
    };
    let var = |name| std::env::var(name).ok();
    Switch::resolve(
        config,
        no_telemetry,
        var("DO_NOT_TRACK").as_deref(),
        var("ZEATEND_TELEMETRY").as_deref(),
    )
}

/// Whether events are sent or printed.
pub(crate) fn is_on() -> bool {
    switch() != Switch::Off
}

/// Says that the program is on and how to turn it off.
pub(crate) fn announce() {
    if is_on() {
        output::note(i18n::messages().telemetry_on);
    }
}

/// The run of this process for a host that becomes `distribution` and `role` by `action`.
pub(crate) fn run(distribution: Distribution, role: Role, action: Action) -> Run {
    Run::new(distribution, role, action, state().via)
}

/// A plan was made, or could not be.
pub(crate) fn planned<T, E>(run: Run, plan: &Result<T, E>) {
    let result = if plan.is_ok() {
        Outcome::Ok
    } else {
        Outcome::Failed
    };
    send(&Event::Plan { run, result });
}

/// The host checks of `command` failed with `failed`.
pub(crate) fn checks_failed(run: Run, command: Command, failed: &[Finding]) {
    send(&Event::ChecksFailed {
        run,
        command,
        checks: zeatend_plan::telemetry::codes(failed),
    });
}

/// `apply` passed its host checks and starts to change the host.
pub(crate) fn started(run: Run) {
    {
        let mut state = state();
        state.started = Some((run.clone(), Instant::now()));
        state.step = None;
    }
    send(&Event::ApplyStarted { run });
}

/// `apply` reached `step`.
pub(crate) fn reached(step: Step) {
    state().step = Some(step);
}

/// `apply` ended with `result`; nothing is sent unless it started.
pub(crate) fn finished<T, E>(result: &Result<T, E>) {
    let (started, step) = {
        let mut state = state();
        (state.started.take(), state.step.take())
    };
    let Some((run, at)) = started else {
        return;
    };
    let (result, step) = if result.is_ok() {
        (Outcome::Ok, None)
    } else {
        (Outcome::Failed, step)
    };
    send(&Event::ApplyFinished {
        run,
        result,
        step,
        seconds: at.elapsed().as_secs(),
    });
}

/// Sends `event` in the background, or prints it in debug mode.
fn send(event: &Event) {
    match switch() {
        Switch::Off => {}
        Switch::Debug => {
            let id = host::peek_id(Path::new(ID_FILE));
            let distinct_id = id
                .as_ref()
                .map_or("installation-id", InstallationId::as_str);
            output::err(&format!(
                "telemetry: {}",
                event.body(PROJECT_KEY, distinct_id)
            ));
        }
        Switch::On if PROJECT_KEY.is_empty() => {}
        Switch::On => {
            let mut state = state();
            // Made here, under the lock, so the events of one process share one ID.
            let Some(id) = once(&mut state.id, Path::new(ID_FILE)) else {
                return;
            };
            let body = event.body(PROJECT_KEY, id.as_str());
            state.sending.push(tokio::spawn(async move {
                drop(host::post(ENDPOINT, body).await);
            }));
        }
    }
}

/// The ID in `slot`, or the one in the file at `path`, made first when there is none.
fn once(slot: &mut Option<InstallationId>, path: &Path) -> Option<InstallationId> {
    if slot.is_none() {
        *slot = host::id(path).ok();
    }
    slot.clone()
}

/// Waits for the requests that still run, for at most `limit`, so the process can end.
pub(crate) async fn flush(limit: Duration) {
    let sending = std::mem::take(&mut state().sending);
    drop(
        tokio::time::timeout(limit, async {
            for task in sending {
                drop(task.await);
            }
        })
        .await,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_keeps_the_id_it_made() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installation-id");
        let mut slot = None;
        let made = once(&mut slot, &path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(once(&mut slot, &path), Some(made));
    }
}
