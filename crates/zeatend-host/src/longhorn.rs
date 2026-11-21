//! Reads what a node has of what Longhorn needs, and makes the changes of a
//! [`Prep`].

use std::fs;
use std::path::Path;

use zeatend_plan::longhorn::{
    HostState, ISCSID, MODULES, MODULES_FILE, MULTIPATH_FILE, MULTIPATHD, PACKAGES, PackageManager,
    Prep, Step,
};

use crate::{HostError, cmd, files, systemd};

/// Where packages put programs such as `iscsiadm`, `mount.nfs`, `apt-get` and `dnf`.
const PROGRAM_DIRS: [&str; 4] = ["/usr/sbin", "/sbin", "/usr/bin", "/bin"];

fn has_program(name: &str) -> bool {
    PROGRAM_DIRS
        .iter()
        .any(|dir| Path::new(dir).join(name).exists())
}

fn module_loaded(name: &str) -> bool {
    Path::new("/sys/module").join(name).exists()
}

/// What the node has now.
pub async fn read() -> HostState {
    HostState {
        package_manager: PackageManager::ALL
            .into_iter()
            .find(|m| has_program(m.program())),
        missing_packages: PACKAGES
            .into_iter()
            .filter(|p| !has_program(p.program))
            .collect(),
        unloaded_modules: MODULES.into_iter().filter(|m| !module_loaded(m)).collect(),
        iscsid_running: systemd::is_active(ISCSID).await && systemd::is_enabled(ISCSID).await,
        multipathd_running: systemd::is_active(MULTIPATHD).await,
        modules_file: fs::read_to_string(MODULES_FILE).ok(),
        multipath_file: fs::read_to_string(MULTIPATH_FILE).ok(),
    }
}

/// Makes the changes of `prep`, calling `note` before each. Writes its files every time, and
/// restarts multipathd only when its file changed.
///
/// # Errors
///
/// When a package manager, `modprobe` or `systemctl` fails, or a file cannot be written.
pub async fn apply(prep: &Prep, mut note: impl FnMut(&Step)) -> Result<(), HostError> {
    for step in &prep.steps {
        if let Step::Install { manager, packages } = step {
            note(step);
            install(*manager, packages).await?;
        }
    }
    let mut multipath_changed = false;
    for file in &prep.files {
        let changed = files::write(Path::new(&file.path), file.mode, file.content.as_bytes())?;
        multipath_changed |= changed && file.path == MULTIPATH_FILE;
    }
    for step in &prep.steps {
        match step {
            Step::Install { .. } | Step::WriteModulesFile => {}
            Step::LoadModules { modules } => {
                note(step);
                for module in modules {
                    cmd::run("modprobe", &[module], None).await?;
                }
            }
            Step::StartIscsid => {
                note(step);
                systemd::enable_now(ISCSID).await?;
            }
            Step::BlacklistMultipath => note(step),
        }
    }
    if multipath_changed {
        systemd::restart(MULTIPATHD).await?;
    }
    Ok(())
}

async fn install(manager: PackageManager, packages: &[&str]) -> Result<(), HostError> {
    match manager {
        PackageManager::Apt => {
            let env = [("DEBIAN_FRONTEND", "noninteractive")];
            cmd::run_with("apt-get", &["update", "-qq"], &env, None).await?;
            let mut args = vec!["install", "-y", "-qq", "--no-install-recommends"];
            args.extend(packages);
            cmd::run_with("apt-get", &args, &env, None).await?;
        }
        PackageManager::Dnf => {
            let mut args = vec!["install", "-y", "-q"];
            args.extend(packages);
            cmd::run("dnf", &args, None).await?;
        }
    }
    Ok(())
}
