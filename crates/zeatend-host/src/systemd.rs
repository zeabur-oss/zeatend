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
/// `env` holds `NAME=value` entries for the unit's environment, which starts empty.
///
/// # Errors
///
/// When `systemd-run` fails, for example because a unit of that name exists.
pub async fn run_transient(
    unit: &str,
    description: &str,
    env: &[String],
    command: &[&str],
) -> Result<(), HostError> {
    let unit_arg = format!("--unit={unit}");
    let description_arg = format!("--description={description}");
    let env_args: Vec<String> = env.iter().map(|e| format!("--setenv={e}")).collect();
    let mut args = vec![
        unit_arg.as_str(),
        description_arg.as_str(),
        "--no-block",
        "--collect",
        "--quiet",
    ];
    args.extend(env_args.iter().map(String::as_str));
    args.push("--");
    args.extend_from_slice(command);
    cmd::run("systemd-run", &args, None).await.map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

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
