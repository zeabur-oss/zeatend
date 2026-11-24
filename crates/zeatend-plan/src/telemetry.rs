//! The experience program (docs/reference/telemetry.mdx): the events zeatend may send, their
//! request body and the switch that turns them off. Sending is in `zeatend-host`.

use serde::Serialize;
use serde_json::{Value, json};

use crate::cluster::Distribution;
use crate::config::Role;
use crate::finding::Finding;

/// The random identifier of one host. It comes from nothing about the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationId(String);

impl InstallationId {
    /// A version 4 UUID made from 16 random bytes (RFC 9562 §5.4,
    /// <https://www.rfc-editor.org/rfc/rfc9562#section-5.4>): version 4 in the high bits of octet
    /// 6 and variant `10` in octet 8. The bytes should come from a CSPRNG (§6.9).
    #[must_use]
    pub fn from_random(mut bytes: [u8; 16]) -> Self {
        if let Some(byte) = bytes.get_mut(6) {
            *byte = (*byte & 0x0F) | 0x40;
        }
        if let Some(byte) = bytes.get_mut(8) {
            *byte = (*byte & 0x3F) | 0x80;
        }
        let mut text = String::with_capacity(36);
        for (i, byte) in bytes.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                text.push('-');
            }
            text.extend(char::from_digit(u32::from(byte >> 4), 16));
            text.extend(char::from_digit(u32::from(byte & 0x0F), 16));
        }
        Self(text)
    }

    /// Reads a UUID in the hex-and-dash form of RFC 9562 §4, in any case.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let groups: Vec<&str> = text.split('-').collect();
        let sizes = [8, 4, 4, 4, 12];
        let canonical = groups.len() == sizes.len()
            && groups.iter().zip(sizes).all(|(group, size)| {
                group.len() == size && group.bytes().all(|b| b.is_ascii_hexdigit())
            });
        canonical.then(|| Self(text.to_ascii_lowercase()))
    }

    /// The identifier as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What zeatend does with an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// Nothing.
    Off,
    /// Prints the request body; sends nothing.
    Debug,
    /// Sends.
    On,
}

impl Switch {
    /// The switch from the configuration's `telemetry` key, `--no-telemetry`, `DO_NOT_TRACK` and
    /// `ZEATEND_TELEMETRY`. Any one of them turns it off; `DO_NOT_TRACK` set to anything but empty
    /// or `0` counts. `ZEATEND_TELEMETRY=debug` prints instead of sending.
    #[must_use]
    pub fn resolve(
        config: Option<bool>,
        no_telemetry: bool,
        do_not_track: Option<&str>,
        telemetry: Option<&str>,
    ) -> Self {
        let do_not_track = do_not_track.is_some_and(|v| !matches!(v.trim(), "" | "0"));
        if do_not_track || no_telemetry || config == Some(false) {
            return Self::Off;
        }
        match telemetry.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("off") => Self::Off,
            Some("debug") => Self::Debug,
            _ => Self::On,
        }
    }
}

/// How the person runs zeatend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    /// A command: `zeatend plan` or `zeatend apply`.
    Cli,
    /// The install screens: `zeatend` with no command.
    Tui,
}

impl Via {
    /// `cli` or `tui`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Tui => "tui",
        }
    }
}

/// What the host becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// The first node of a cluster.
    Install,
    /// A node that joins a cluster.
    Join,
}

/// The step of `apply` that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// Preparing the host for Longhorn.
    Longhorn,
    /// Writing the files, downloading and installing the release, starting the service.
    Install,
    /// Waiting until the node and the cluster's own deployments are ready.
    Node,
}

/// How `apply` ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// It worked.
    Ok,
    /// It stopped with an error.
    Failed,
}

/// The command that ran the host checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    /// `zeatend plan`, which shows the checks and goes on.
    Plan,
    /// `zeatend apply`, which stops.
    Apply,
}

/// What one run of zeatend sets up, on each of its events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Run {
    /// The version of zeatend.
    pub zeatend_version: &'static str,
    /// `k3s`, `rke2` or `kubesolo`.
    pub distribution: Distribution,
    /// `server` or `worker`; the first node is a server.
    pub role: Role,
    /// Whether the host installs a cluster or joins one.
    pub action: Action,
    /// How the person runs zeatend.
    pub via: Via,
    /// `amd64` or `arm64`.
    pub arch: &'static str,
}

impl Run {
    /// A run of this zeatend on this machine.
    #[must_use]
    pub const fn new(distribution: Distribution, role: Role, action: Action, via: Via) -> Self {
        Self {
            zeatend_version: env!("CARGO_PKG_VERSION"),
            distribution,
            role,
            action,
            via,
            arch: arch(std::env::consts::ARCH),
        }
    }
}

/// The architecture as release archives name it, from Rust's name for it.
const fn arch(rust: &str) -> &'static str {
    match rust.as_bytes() {
        b"x86_64" => "amd64",
        b"aarch64" => "arm64",
        _ => "other",
    }
}

/// Everything that can be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A plan was made, or could not be: `zeatend plan`, or the Review screen.
    Plan {
        /// The run.
        run: Run,
        /// Whether a plan was made.
        result: Outcome,
    },
    /// Host checks failed.
    ChecksFailed {
        /// The run.
        run: Run,
        /// The command that ran them.
        command: Command,
        /// The codes of the failed findings, such as `disk.low`.
        checks: Vec<String>,
    },
    /// `apply` passed the host checks and starts to change the host.
    ApplyStarted {
        /// The run.
        run: Run,
    },
    /// `apply` ended after it started.
    ApplyFinished {
        /// The run.
        run: Run,
        /// How it ended.
        result: Outcome,
        /// The step that failed.
        step: Option<Step>,
        /// Seconds since it started.
        seconds: u64,
    },
}

impl Event {
    /// The event's name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Plan { .. } => "zeatend_plan",
            Self::ChecksFailed { .. } => "zeatend_checks_failed",
            Self::ApplyStarted { .. } => "zeatend_apply_started",
            Self::ApplyFinished { .. } => "zeatend_apply_finished",
        }
    }

    /// The event's properties.
    fn properties(&self) -> Value {
        let (run, extra) = match self {
            Self::Plan { run, result } => (run, json!({ "result": result })),
            Self::ChecksFailed {
                run,
                command,
                checks,
            } => (run, json!({ "command": command, "checks": checks })),
            Self::ApplyStarted { run } => (run, json!({})),
            Self::ApplyFinished {
                run,
                result,
                step,
                seconds,
            } => (
                run,
                json!({ "result": result, "step": step, "seconds": seconds }),
            ),
        };
        let mut properties = serde_json::to_value(run).unwrap_or_else(|_| json!({}));
        if let (Value::Object(properties), Value::Object(extra)) = (&mut properties, extra) {
            properties.extend(extra);
            properties.insert("$process_person_profile".to_owned(), json!(false));
            properties.insert("$geoip_disable".to_owned(), json!(true));
        }
        properties
    }

    /// The body of the request to `PostHog`'s batch endpoint, with `distinct_id` as the only
    /// identity. No person profile is made, and `PostHog` is told not to look up a location.
    #[must_use]
    pub fn body(&self, key: &str, distinct_id: &str) -> String {
        json!({
            "api_key": key,
            "batch": [{
                "event": self.name(),
                "distinct_id": distinct_id,
                "properties": self.properties(),
            }],
        })
        .to_string()
    }
}

/// The codes of `findings`, such as `disk.low`: a fixed list, with none of a finding's values.
#[must_use]
pub fn codes<'a>(findings: impl IntoIterator<Item = &'a Finding>) -> Vec<String> {
    findings
        .into_iter()
        .filter_map(|finding| {
            let value = serde_json::to_value(finding).ok()?;
            value.get("code")?.as_str().map(str::to_owned)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn run() -> Run {
        Run {
            zeatend_version: "0.2.0",
            distribution: Distribution::Rke2,
            role: Role::Server,
            action: Action::Install,
            via: Via::Tui,
            arch: "amd64",
        }
    }

    fn body(event: &Event) -> Value {
        serde_json::from_str(&event.body("key", "id")).unwrap()
    }

    fn properties(event: &Event) -> Value {
        body(event)["batch"][0]["properties"].clone()
    }

    #[test]
    fn a_finished_install_sends_the_run_and_how_it_ended() {
        let event = Event::ApplyFinished {
            run: run(),
            result: Outcome::Ok,
            step: None,
            seconds: 312,
        };
        assert_eq!(
            body(&event),
            json!({
                "api_key": "key",
                "batch": [{
                    "event": "zeatend_apply_finished",
                    "distinct_id": "id",
                    "properties": {
                        "zeatend_version": "0.2.0",
                        "distribution": "rke2",
                        "role": "server",
                        "action": "install",
                        "via": "tui",
                        "arch": "amd64",
                        "result": "ok",
                        "step": null,
                        "seconds": 312,
                        "$process_person_profile": false,
                        "$geoip_disable": true,
                    },
                }],
            })
        );
    }

    #[test]
    fn a_failed_join_names_the_step() {
        let event = Event::ApplyFinished {
            run: Run {
                distribution: Distribution::K3s,
                role: Role::Worker,
                action: Action::Join,
                via: Via::Cli,
                ..run()
            },
            result: Outcome::Failed,
            step: Some(Step::Node),
            seconds: 600,
        };
        let properties = properties(&event);
        assert_eq!(properties["action"], "join");
        assert_eq!(properties["role"], "worker");
        assert_eq!(properties["via"], "cli");
        assert_eq!(properties["result"], "failed");
        assert_eq!(properties["step"], "node");
    }

    #[test]
    fn failed_checks_send_their_codes_and_none_of_their_values() {
        let findings = [
            Finding::DiskLow {
                free_tenths_gib: 31,
                path: "/var/lib/secret-name".to_owned(),
                needed_gib: 10,
            },
            Finding::SwapOn,
        ];
        let event = Event::ChecksFailed {
            run: run(),
            command: Command::Apply,
            checks: codes(&findings),
        };
        assert_eq!(event.name(), "zeatend_checks_failed");
        let properties = properties(&event);
        assert_eq!(properties["command"], "apply");
        assert_eq!(properties["checks"], json!(["disk.low", "swap.on"]));
        assert!(!event.body("key", "id").contains("secret-name"));
    }

    #[test]
    fn a_plan_and_a_start_send_the_run() {
        let plan = Event::Plan {
            run: run(),
            result: Outcome::Failed,
        };
        assert_eq!(plan.name(), "zeatend_plan");
        assert_eq!(properties(&plan)["result"], "failed");
        let started = Event::ApplyStarted { run: run() };
        assert_eq!(started.name(), "zeatend_apply_started");
        assert_eq!(properties(&started)["distribution"], "rke2");
        assert!(properties(&started).get("result").is_none());
    }

    #[test]
    fn any_opt_out_turns_it_off_and_debug_only_prints() {
        let resolve = Switch::resolve;
        assert_eq!(resolve(None, false, None, None), Switch::On);
        assert_eq!(resolve(Some(true), false, Some("0"), Some("")), Switch::On);
        assert_eq!(resolve(Some(false), false, None, None), Switch::Off);
        assert_eq!(resolve(None, true, None, None), Switch::Off);
        assert_eq!(resolve(None, false, Some("1"), Some("debug")), Switch::Off);
        assert_eq!(resolve(None, false, Some("true"), None), Switch::Off);
        assert_eq!(resolve(None, false, None, Some(" OFF ")), Switch::Off);
        assert_eq!(resolve(None, false, None, Some("debug")), Switch::Debug);
    }

    #[test]
    fn a_random_identifier_is_a_version_4_uuid_and_reads_back() {
        let id = InstallationId::from_random([0xFF; 16]);
        assert_eq!(id.as_str(), "ffffffff-ffff-4fff-bfff-ffffffffffff");
        assert_eq!(InstallationId::parse(id.as_str()), Some(id));
        assert_eq!(
            InstallationId::parse(" 0A1B2C3D-0000-4000-8000-000000000000\n")
                .unwrap()
                .as_str(),
            "0a1b2c3d-0000-4000-8000-000000000000"
        );
        for bad in [
            "",
            "abc",
            "0a1b2c3d-0000-4000-8000-00000000000",
            "g-g-g-g-g",
        ] {
            assert_eq!(InstallationId::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_architecture_is_the_release_archives_name() {
        assert_eq!(arch("x86_64"), "amd64");
        assert_eq!(arch("aarch64"), "arm64");
        assert_eq!(arch("riscv64"), "other");
    }
}
