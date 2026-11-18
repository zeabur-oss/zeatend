//! The progress of an install, one JSON object per line.
//!
//! The install unit prints these lines on stdout, so the journal keeps them. The command that
//! follows the unit, and any screen that shows an install, reads them back with [`Event::parse`].

use serde::{Deserialize, Serialize};

/// One step of progress. On the wire it is a JSON object whose `event` field names the variant,
/// for example `{"event":"step","text":"Checking the host"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// A step starts.
    Step {
        /// What the step does.
        text: String,
    },
    /// A detail of the current step, such as a URL or a check result.
    Note {
        /// The detail.
        text: String,
    },
    /// Part of the result, such as the plan or what to run next. It can have more than one line.
    Line {
        /// The text.
        text: String,
    },
    /// How much of a download has arrived.
    Download {
        /// Bytes received.
        done: u64,
        /// The size, when the server sent it.
        total: Option<u64>,
        /// Bytes per second.
        rate: u64,
    },
    /// The current download ended.
    DownloadEnd,
    /// The install finished.
    Done,
    /// The install stopped with an error.
    Failed {
        /// The error.
        error: String,
    },
}

impl Event {
    /// The event as one line of JSON, without the line feed.
    #[must_use]
    pub fn to_line(&self) -> String {
        // Strings and integers always serialize.
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The event in `line`, or `None` when `line` is not an event: another program's output, or
    /// an event from a later version of zeatend.
    #[must_use]
    pub fn parse(line: &str) -> Option<Self> {
        serde_json::from_str(line.trim_end()).ok()
    }

    /// Whether the install ends with this event.
    #[must_use]
    pub const fn is_last(&self) -> bool {
        matches!(self, Self::Done | Self::Failed { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_event_is_one_json_line() {
        let cases = [
            (
                Event::Step {
                    text: "Checking the host".to_owned(),
                },
                r#"{"event":"step","text":"Checking the host"}"#,
            ),
            (
                Event::Note {
                    text: "ok   memory".to_owned(),
                },
                r#"{"event":"note","text":"ok   memory"}"#,
            ),
            (
                Event::Line {
                    text: "K3s v1.37.2+k3s1\n  on node-1".to_owned(),
                },
                r#"{"event":"line","text":"K3s v1.37.2+k3s1\n  on node-1"}"#,
            ),
            (
                Event::Download {
                    done: 1024,
                    total: Some(4096),
                    rate: 512,
                },
                r#"{"event":"download","done":1024,"total":4096,"rate":512}"#,
            ),
            (
                Event::Download {
                    done: 1,
                    total: None,
                    rate: 0,
                },
                r#"{"event":"download","done":1,"total":null,"rate":0}"#,
            ),
            (Event::DownloadEnd, r#"{"event":"download_end"}"#),
            (Event::Done, r#"{"event":"done"}"#),
            (
                Event::Failed {
                    error: "the host is not ready".to_owned(),
                },
                r#"{"event":"failed","error":"the host is not ready"}"#,
            ),
        ];
        for (event, json) in cases {
            let line = event.to_line();
            assert_eq!(line, json);
            assert!(!line.contains('\n'), "{line}");
            assert_eq!(Event::parse(&line), Some(event));
        }
    }

    #[test]
    fn other_lines_are_not_events() {
        for line in [
            "",
            "thread 'main' panicked at src/main.rs:1:1",
            r#"{"event":"progress_bar","value":3}"#,
            r#"{"text":"no tag"}"#,
            "[1,2,3]",
        ] {
            assert_eq!(Event::parse(line), None, "{line}");
        }
        assert_eq!(Event::parse("{\"event\":\"done\"}\n"), Some(Event::Done));
    }

    #[test]
    fn only_done_and_failed_end_an_install() {
        assert!(Event::Done.is_last());
        assert!(
            Event::Failed {
                error: String::new()
            }
            .is_last()
        );
        assert!(!Event::DownloadEnd.is_last());
    }
}
