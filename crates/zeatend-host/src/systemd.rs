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
