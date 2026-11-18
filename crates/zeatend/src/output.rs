//! The only place that writes to stdout and stderr directly.
//!
//! Progress goes through [`emit`]: as text, or as JSON lines when the install runs in a systemd
//! unit and the journal keeps its output.

use std::io::{IsTerminal as _, Write as _};
use std::sync::atomic::{AtomicBool, Ordering};

use zeatend_host::download;
use zeatend_host::progress::Event;

/// Whether progress prints as JSON lines.
static JSON: AtomicBool = AtomicBool::new(false);
/// Whether a download line that ends without a line feed is on the terminal.
static METER_SHOWN: AtomicBool = AtomicBool::new(false);

/// Prints progress as JSON lines on stdout from now on.
pub(crate) fn print_json_events() {
    JSON.store(true, Ordering::Relaxed);
}

/// Prints one event of progress.
pub(crate) fn emit(event: &Event) {
    if JSON.load(Ordering::Relaxed) {
        stdout(&event.to_line());
        return;
    }
    let terminal = std::io::stderr().is_terminal();
    match event {
        Event::Step { text } => err(&format!("==> {text}")),
        Event::Note { text } => err(&format!("    {text}")),
        Event::Line { text } => stdout(text),
        Event::Download { done, total, rate } => {
            let of = total.map(|total| format!(" of {}", size(total)));
            let text = format!(
                "    {}{}, {}/s",
                size(*done),
                of.unwrap_or_default(),
                size(*rate)
            );
            if terminal {
                METER_SHOWN.store(true, Ordering::Relaxed);
                drop(write!(std::io::stderr(), "\r\x1b[K{text}"));
            } else {
                err(&text);
            }
        }
        Event::DownloadEnd => {
            if METER_SHOWN.swap(false, Ordering::Relaxed) {
                drop(writeln!(std::io::stderr()));
            }
        }
        Event::Done => {}
        Event::Failed { error } => err(&format!("error: {error}")),
    }
}

/// Announces a step of `apply`.
pub(crate) fn step(text: &str) {
    emit(&Event::Step {
        text: text.to_owned(),
    });
}

/// Shows a detail of the current step.
pub(crate) fn note(text: &str) {
    emit(&Event::Note {
        text: text.to_owned(),
    });
}

/// Shows part of the result on stdout.
pub(crate) fn line(text: &str) {
    emit(&Event::Line {
        text: text.to_owned(),
    });
}

/// Writes one line to stdout.
pub(crate) fn stdout(text: &str) {
    // A closed pipe (`| head`) is not an error worth reporting.
    drop(writeln!(std::io::stdout(), "{text}"));
}

/// Writes one line to stderr.
pub(crate) fn err(text: &str) {
    drop(writeln!(std::io::stderr(), "{text}"));
}

/// Shows a download's progress: on one line that it rewrites when stderr is a terminal, else one
/// line for each update.
#[derive(Debug)]
pub(crate) struct Meter;

impl download::Progress for Meter {
    fn update(&mut self, done: u64, total: Option<u64>, rate: u64) {
        emit(&Event::Download { done, total, rate });
    }

    fn finish(&mut self) {
        emit(&Event::DownloadEnd);
    }
}

/// A byte count in KiB, MiB or GiB, with one decimal.
fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut unit = 0;
    let mut tenths = u128::from(bytes) * 10;
    while tenths >= 10 * 1024 && unit + 1 < UNITS.len() {
        tenths /= 1024;
        unit += 1;
    }
    let name = UNITS.get(unit).copied().unwrap_or("B");
    if unit == 0 {
        format!("{bytes} {name}")
    } else {
        format!("{}.{} {name}", tenths / 10, tenths % 10)
    }
}

#[cfg(test)]
mod tests {
    use super::size;

    #[test]
    fn sizes() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(1023), "1023 B");
        assert_eq!(size(1536), "1.5 KiB");
        assert_eq!(size(300 << 20), "300.0 MiB");
        assert_eq!(size(5 << 30), "5.0 GiB");
    }
}
