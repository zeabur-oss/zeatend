//! Reading a unit's output from the systemd journal.

use std::ops::ControlFlow;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::process::Command;

use crate::{HostError, cmd, systemd};

/// How often [`follow`] asks whether the unit still runs.
const POLL: Duration = Duration::from_secs(2);
/// How long [`follow`] waits for a unit that has not started yet.
const START_WAIT: Duration = Duration::from_secs(30);

/// One entry of `journalctl --output=json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The position in the journal, for `--after-cursor`.
    pub cursor: String,
    /// The line the unit's process wrote, or `None` for an entry that another process, such as
    /// systemd, logged about the unit.
    pub output: Option<String>,
}

impl Entry {
    /// Parses one line of `journalctl --unit=<unit> --output=json`. Returns `None` when the line
    /// is not a journal entry.
    ///
    /// `journalctl --unit` also shows what systemd logs about the unit, such as "Started …";
    /// only entries whose `_SYSTEMD_UNIT` is `unit` are the unit's output. `MESSAGE` is a string,
    /// or an array of bytes when it is not valid UTF-8 or holds control characters
    /// (systemd.journal-fields(7), "JSON format" in journalctl(1)).
    #[must_use]
    pub fn parse(line: &str, unit: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(line).ok()?;
        let cursor = value.get("__CURSOR")?.as_str()?.to_owned();
        let from_unit = value.get("_SYSTEMD_UNIT").and_then(Value::as_str) == Some(unit);
        let output = from_unit.then(|| message(value.get("MESSAGE")));
        Some(Self { cursor, output })
    }
}

/// The text of a `MESSAGE` field.
fn message(field: Option<&Value>) -> String {
    match field {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(bytes)) => {
            let bytes: Vec<u8> = bytes
                .iter()
                .filter_map(Value::as_u64)
                .filter_map(|b| u8::try_from(b).ok())
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }
        Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_)) | None => {
            String::new()
        }
    }
}

/// How [`follow`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Followed {
    /// The callback returned [`ControlFlow::Break`].
    Stopped,
    /// The unit no longer runs, and every line it wrote was passed on.
    Ended,
}

/// Passes each line the unit wrote, from its first one, to `on_line` as it arrives, with
/// `journalctl --unit=<unit> --follow --output=json --all`.
///
/// `journalctl --follow` does not end with the unit, so this asks `systemctl is-active` every two
/// seconds. When the unit no longer runs, it reads the rest of the journal after the last entry
/// it saw and returns. A unit that never runs counts as ended after 30 seconds.
///
/// Dropping the future stops `journalctl`; the unit goes on.
///
/// # Errors
///
/// When `journalctl` cannot start or fails.
pub async fn follow(
    unit: &str,
    mut on_line: impl FnMut(&str) -> ControlFlow<()>,
) -> Result<Followed, HostError> {
    let fail = |reason: String| HostError::Command {
        command: format!("journalctl --unit={unit} --follow"),
        reason,
    };
    let mut child = Command::new("journalctl")
        .args([
            &format!("--unit={unit}"),
            "--follow",
            "--output=json",
            // Without it, a field longer than 4096 bytes reads as `null`.
            "--all",
            "--lines=all",
            "--no-pager",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| fail(e.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| fail("no stdout".to_owned()))?;
    let mut lines = BufReader::new(stdout).lines();
    let mut cursor = None;
    let mut poll = tokio::time::interval(POLL);
    let since = Instant::now();
    let mut seen_running = false;
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { break };
                if pass_on(&line, unit, &mut cursor, &mut on_line).is_break() {
                    return Ok(Followed::Stopped);
                }
            }
            _ = poll.tick() => {
                if systemd::is_running(unit).await {
                    seen_running = true;
                } else if seen_running || since.elapsed() > START_WAIT {
                    break;
                }
            }
        }
    }
    drop(child);

    // What the unit wrote after the last line read above.
    let unit_arg = format!("--unit={unit}");
    let mut args = vec![unit_arg.as_str(), "--output=json", "--all", "--no-pager"];
    let after = cursor.as_ref().map(|c| format!("--after-cursor={c}"));
    args.extend(after.as_deref());
    let rest = cmd::run("journalctl", &args, None).await?;
    for line in rest.lines() {
        if pass_on(line, unit, &mut cursor, &mut on_line).is_break() {
            return Ok(Followed::Stopped);
        }
    }
    Ok(Followed::Ended)
}

/// Passes the unit's output in one journal line to `on_line` and keeps its cursor.
fn pass_on(
    line: &str,
    unit: &str,
    cursor: &mut Option<String>,
    on_line: &mut impl FnMut(&str) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let Some(entry) = Entry::parse(line, unit) else {
        return ControlFlow::Continue(());
    };
    *cursor = Some(entry.cursor);
    entry
        .output
        .map_or(ControlFlow::Continue(()), |text| on_line(&text))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::progress::Event;

    /// `journalctl --unit=… --output=json` of a K3s install on Ubuntu 26.04 with systemd 259, with
    /// what systemd logged about the unit around it.
    const DONE: &str = include_str!("../fixtures/journal-done.json");
    const DONE_UNIT: &str = "zeatend-install-1794991689.service";
    /// The same of an install whose configuration does not parse.
    const FAILED: &str = include_str!("../fixtures/journal-failed.json");
    const FAILED_UNIT: &str = "zeatend-install-1794991748.service";

    /// The events in a journal, and the lines of the unit that are not events.
    fn events(journal: &str, unit: &str) -> (Vec<Event>, Vec<String>) {
        let (mut events, mut other) = (Vec::new(), Vec::new());
        for line in journal.lines() {
            let entry = Entry::parse(line, unit).unwrap();
            assert!(entry.cursor.starts_with("s="), "{}", entry.cursor);
            if let Some(text) = entry.output {
                match Event::parse(&text) {
                    Some(event) => events.push(event),
                    None => other.push(text),
                }
            }
        }
        (events, other)
    }

    #[test]
    fn an_install_reads_back_as_its_events() {
        let (events, other) = events(DONE, DONE_UNIT);
        assert_eq!(other, Vec::<String>::new());
        // systemd's "Started", "Deactivated" and "Consumed" lines are not output.
        assert_eq!(DONE.lines().count(), events.len() + 3);
        let steps: Vec<&str> = events
            .iter()
            .filter_map(|e| {
                if let Event::Step { text } = e {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            steps,
            ["Checking the host", "K3s v1.37.2+k3s1 (k3s)", "Done"]
        );
        assert!(events.contains(&Event::Download {
            done: 73_990_306,
            total: Some(73_990_306),
            rate: 36_774_505
        }));
        assert_eq!(
            events.iter().filter(|e| **e == Event::DownloadEnd).count(),
            3
        );
        assert_eq!(events.last(), Some(&Event::Done));
        assert_eq!(events.iter().filter(|e| e.is_last()).count(), 1);
    }

    #[test]
    fn a_failed_install_ends_with_its_error() {
        let (events, other) = events(FAILED, FAILED_UNIT);
        assert_eq!(other, Vec::<String>::new());
        let [Event::Failed { error }] = events.as_slice() else {
            panic!("{events:?}");
        };
        assert!(
            error.starts_with("configuration: error: line 1 column 15: unknown variant `k9s`"),
            "{error}"
        );
        assert!(error.contains('\n'), "{error}");
    }

    #[test]
    fn lines_of_another_unit_are_not_output() {
        for line in DONE.lines() {
            assert_eq!(Entry::parse(line, FAILED_UNIT).unwrap().output, None);
        }
    }

    #[test]
    fn a_message_can_be_bytes_or_missing() {
        let entry = |message: Value| {
            let line =
                json!({"__CURSOR": "s=1;i=2", "_SYSTEMD_UNIT": DONE_UNIT, "MESSAGE": message});
            Entry::parse(&line.to_string(), DONE_UNIT)
                .unwrap()
                .output
                .unwrap()
        };
        // "café \x1b" and an invalid byte: journalctl prints such a message as an array.
        assert_eq!(
            entry(json!([99, 97, 102, 195, 169, 32, 27, 255])),
            "caf\u{e9} \u{1b}\u{fffd}"
        );
        assert_eq!(entry(Value::Null), "");
        assert_eq!(entry(json!("plain")), "plain");
        assert_eq!(Entry::parse("-- No entries --", DONE_UNIT), None);
        assert_eq!(
            Entry::parse(&json!({"MESSAGE": "no cursor"}).to_string(), DONE_UNIT),
            None
        );
    }

    #[test]
    fn pass_on_keeps_the_cursor_and_stops_when_asked() {
        let mut cursor = None;
        let mut seen = 0;
        let flow = DONE.lines().try_for_each(|line| {
            pass_on(line, DONE_UNIT, &mut cursor, &mut |text: &str| {
                seen += 1;
                match Event::parse(text) {
                    Some(event) if event.is_last() => ControlFlow::Break(()),
                    Some(_) | None => ControlFlow::Continue(()),
                }
            })
        });
        assert_eq!(flow, ControlFlow::Break(()));
        assert_eq!(seen, DONE.lines().count() - 3);
        let done = DONE.lines().nth(seen).unwrap();
        assert_eq!(cursor, Some(Entry::parse(done, DONE_UNIT).unwrap().cursor));
    }
}
