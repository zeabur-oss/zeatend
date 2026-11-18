//! The only place that writes to stdout and stderr directly.

use std::io::{IsTerminal as _, Write as _};

use zeatend_host::download;

/// Writes one line to stdout.
pub(crate) fn line(text: &str) {
    // A closed pipe (`| head`) is not an error worth reporting.
    drop(writeln!(std::io::stdout(), "{text}"));
}

/// Writes one line to stderr.
pub(crate) fn err(text: &str) {
    drop(writeln!(std::io::stderr(), "{text}"));
}

/// Announces a step of `apply`.
pub(crate) fn step(text: &str) {
    err(&format!("==> {text}"));
}

/// Shows a download's progress on stderr: on one line that it rewrites when stderr is a
/// terminal, else one line for each update.
#[derive(Debug)]
pub(crate) struct Meter {
    terminal: bool,
    shown: bool,
}

impl Meter {
    pub(crate) fn new() -> Self {
        Self {
            terminal: std::io::stderr().is_terminal(),
            shown: false,
        }
    }
}

impl download::Progress for Meter {
    fn update(&mut self, done: u64, total: Option<u64>, rate: u64) {
        let done = size(done);
        let of = total.map(|total| format!(" of {}", size(total)));
        let text = format!("    {done}{}, {}/s", of.unwrap_or_default(), size(rate));
        self.shown = true;
        if self.terminal {
            drop(write!(std::io::stderr(), "\r\x1b[K{text}"));
        } else {
            err(&text);
        }
    }

    fn finish(&mut self) {
        if self.terminal && self.shown {
            drop(writeln!(std::io::stderr()));
        }
        self.shown = false;
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
