//! systemd units.

use crate::{HostError, cmd};

/// Reloads unit files.
///
/// # Errors
///
/// When `systemctl` fails.
pub async fn daemon_reload() -> Result<(), HostError> {
    cmd::run("systemctl", &["daemon-reload"], None)
        .await
        .map(drop)
}

/// Enables and starts a unit; starting waits until it reports ready.
///
/// # Errors
///
/// When `systemctl` fails; for `rke2-server`, `journalctl -u rke2-server` says why.
pub async fn enable_now(unit: &str) -> Result<(), HostError> {
    cmd::run("systemctl", &["enable", "--now", unit], None)
        .await
        .map(drop)
}

/// Restarts a unit.
///
/// # Errors
///
/// When `systemctl` fails.
pub async fn restart(unit: &str) -> Result<(), HostError> {
    cmd::run("systemctl", &["restart", unit], None)
        .await
        .map(drop)
}

/// Whether a unit is enabled to start at boot.
#[must_use]
pub async fn is_enabled(unit: &str) -> bool {
    cmd::succeeds("systemctl", &["is-enabled", "--quiet", unit]).await
}

/// Whether a unit is active.
#[must_use]
pub async fn is_active(unit: &str) -> bool {
    cmd::succeeds("systemctl", &["is-active", "--quiet", unit]).await
}

/// When the main process of a unit started, in seconds since the Unix epoch.
///
/// The time comes from the D-Bus property in microseconds, because the text of
/// `systemctl show` depends on the systemd version and the time zone.
///
/// # Errors
///
/// When `busctl` fails or the unit has not started.
pub async fn main_started(unit: &str) -> Result<i64, HostError> {
    let object = bus_object(unit);
    let shown = cmd::run(
        "busctl",
        &[
            "--json=short",
            "get-property",
            "org.freedesktop.systemd1",
            &object,
            "org.freedesktop.systemd1.Service",
            "ExecMainStartTimestamp",
        ],
        None,
    )
    .await?;
    unix_seconds(&shown).ok_or_else(|| HostError::Parse {
        what: format!("the start of {unit}"),
        reason: format!("not a time: {:?}", shown.trim()),
    })
}

/// The D-Bus object of a unit. A name without a type, such as `k3s`, is a service.
fn bus_object(unit: &str) -> String {
    let name = if unit.contains('.') {
        unit.to_owned()
    } else {
        format!("{unit}.service")
    };
    format!("/org/freedesktop/systemd1/unit/{}", bus_label(&name))
}

/// A unit name as a D-Bus object path label: every byte but a letter or digit is `_` and two
/// hex digits.
fn bus_label(unit: &str) -> String {
    unit.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() {
                char::from(byte).to_string()
            } else {
                format!("_{byte:02x}")
            }
        })
        .collect()
}

/// The seconds in `{"type":"t","data":1792742430000000}`, the microseconds `busctl` prints.
fn unix_seconds(shown: &str) -> Option<i64> {
    #[derive(serde::Deserialize)]
    struct Property {
        data: i64,
    }
    let property: Property = serde_json::from_str(shown).ok()?;
    (property.data > 0).then_some(property.data / 1_000_000)
}

/// Whether systemd manages this host: the test of `sd_booted(3)`.
#[must_use]
pub fn booted() -> bool {
    std::path::Path::new("/run/systemd/system").is_dir()
}

/// Whether a unit runs or changes state: `systemctl is-active` prints `active`, `activating`,
/// `deactivating`, `reloading` or `refreshing`. A unit that ended or does not exist does not run.
#[must_use]
pub async fn is_running(unit: &str) -> bool {
    let output = tokio::process::Command::new("systemctl")
        .args(["is-active", "--", unit])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await;
    output.is_ok_and(|o| running_state(&String::from_utf8_lossy(&o.stdout)))
}

/// Whether `state`, the output of `systemctl is-active`, is a state of a running unit.
fn running_state(state: &str) -> bool {
    matches!(
        state.trim(),
        "active" | "activating" | "deactivating" | "reloading" | "refreshing"
    )
}

/// The service units whose names match `pattern` and that run, oldest name first.
///
/// # Errors
///
/// When `systemctl` fails.
pub async fn running_services(pattern: &str) -> Result<Vec<String>, HostError> {
    let listed = cmd::run(
        "systemctl",
        &[
            "list-units",
            "--type=service",
            "--plain",
            "--no-legend",
            "--full",
            "--",
            pattern,
        ],
        None,
    )
    .await?;
    let mut running = Vec::new();
    for unit in unit_names(&listed) {
        if is_running(&unit).await {
            running.push(unit);
        }
    }
    Ok(running)
}

/// The unit names in the output of `systemctl list-units --plain --no-legend`, sorted.
fn unit_names(listed: &str) -> Vec<String> {
    let mut names: Vec<String> = listed
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect();
    names.sort();
    names
}

/// Starts `command` as the transient service `unit` and returns without waiting for it.
///
/// It runs `systemd-run --no-block --collect`: the unit's stdout and stderr go to the journal,
/// and systemd removes the unit when it ends, also when it fails.
///
/// # Errors
///
/// When `systemd-run` fails, for example because a unit of that name exists.
pub async fn run_transient(transient: &Transient<'_>) -> Result<(), HostError> {
    let unit_arg = format!("--unit={}", transient.unit);
    let description_arg = format!("--description={}", transient.description);
    let dir_arg = format!("--working-directory={}", transient.dir);
    let env_args: Vec<String> = transient
        .env
        .iter()
        .map(|e| format!("--setenv={e}"))
        .collect();
    let mut args = vec![
        unit_arg.as_str(),
        description_arg.as_str(),
        dir_arg.as_str(),
        "--no-block",
        "--collect",
        "--quiet",
    ];
    args.extend(env_args.iter().map(String::as_str));
    args.push("--");
    let command: Vec<String> = transient.command.iter().map(|a| literal(a)).collect();
    args.extend(command.iter().map(String::as_str));
    cmd::run("systemd-run", &args, None).await.map(drop)
}

/// `arg` with each `$` doubled. systemd expands `$NAME` and `${NAME}` in the command of a
/// transient service as in `ExecStart=`, where `$$` is a literal `$` (systemd.service(5),
/// "Command lines"). `--expand-environment=no` does the same but needs systemd 254.
fn literal(arg: &str) -> String {
    arg.replace('$', "$$")
}

/// A transient service for [`run_transient`].
#[derive(Debug, Clone, Copy)]
pub struct Transient<'a> {
    /// The unit name, ending in `.service`.
    pub unit: &'a str,
    /// The unit's description.
    pub description: &'a str,
    /// The absolute working directory of the command.
    pub dir: &'a str,
    /// `NAME=value` entries for the unit's environment, which starts empty.
    pub env: &'a [String],
    /// The program and its arguments, passed as they are.
    pub command: &'a [&'a str],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dollar_signs_stay_literal() {
        assert_eq!(literal("/root/${CLUSTER}.yaml"), "/root/$${CLUSTER}.yaml");
        assert_eq!(literal("a$b$$c"), "a$$b$$$$c");
        assert_eq!(
            literal("/etc/zeatend/config.yaml"),
            "/etc/zeatend/config.yaml"
        );
    }

    #[test]
    fn unix_seconds_come_from_microseconds() {
        assert_eq!(
            unix_seconds("{\"type\":\"t\",\"data\":1792742430999999}\n"),
            Some(1_792_742_430)
        );
        assert_eq!(unix_seconds("{\"type\":\"t\",\"data\":0}"), None);
        assert_eq!(unix_seconds("n/a"), None);
    }

    #[test]
    fn a_unit_without_a_type_is_a_service() {
        assert_eq!(
            bus_object("k3s"),
            "/org/freedesktop/systemd1/unit/k3s_2eservice"
        );
        assert_eq!(bus_object("k3s.service"), bus_object("k3s"));
    }

    #[test]
    fn unit_names_become_bus_labels() {
        assert_eq!(bus_label("rke2-server.service"), "rke2_2dserver_2eservice");
    }

    #[test]
    fn running_states() {
        for state in ["active\n", "activating\n", "deactivating", "reloading"] {
            assert!(running_state(state), "{state}");
        }
        for state in ["inactive\n", "failed\n", "", "unknown"] {
            assert!(!running_state(state), "{state}");
        }
    }

    #[test]
    fn unit_names_are_the_first_column() {
        let listed = "\
zeatend-install-1763460100.service loaded active running zeatend apply
zeatend-install-1763460000.service loaded active running zeatend apply

";
        assert_eq!(
            unit_names(listed),
            [
                "zeatend-install-1763460000.service",
                "zeatend-install-1763460100.service"
            ]
        );
        assert_eq!(unit_names(""), Vec::<String>::new());
    }
}
