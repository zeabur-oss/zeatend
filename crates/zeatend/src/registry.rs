//! What the registry and proxy settings read on the host: the registry passwords and the files of
//! these settings that zeatend wrote before.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use zeatend_host::files;
use zeatend_plan::cluster::{Distribution, Rancher};
use zeatend_plan::config::Role;
use zeatend_plan::plan::Facts;
use zeatend_plan::registry::{self, HEADER, KUBESOLO_REGISTRY_DIR, Password, RegistryHost};

use crate::InstallError;

/// Reads the password of each host in `hosts` with a `passwordFile`, and the files zeatend wrote
/// for these settings and for `network.interface` on a node of `distribution` that runs `role`,
/// into `facts`.
///
/// # Errors
///
/// When a password file cannot be read, lets others read it or is not text.
pub(crate) fn read(
    facts: &mut Facts,
    distribution: Distribution,
    role: Role,
    hosts: &BTreeMap<String, RegistryHost>,
) -> Result<(), InstallError> {
    for (host, settings) in hosts {
        if let Some(file) = &settings.password_file {
            let bytes = files::read_secret(Path::new(file))?;
            let text = String::from_utf8(bytes).map_err(|_e| InstallError::NotText(file.into()))?;
            let password = text.trim_end_matches(['\r', '\n']).to_owned();
            facts
                .registry_passwords
                .insert(host.clone(), Password(password));
        }
    }
    let mut candidates = registry::optional_files(distribution, role);
    if distribution == Distribution::Rke2 && role == Role::Server {
        candidates.push(Rancher::canal_config_path());
    }
    if distribution == Distribution::KubeSolo {
        candidates.extend(
            fs::read_dir(KUBESOLO_REGISTRY_DIR)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| format!("{}/hosts.toml", entry.path().display())),
        );
    }
    facts.registry_files = candidates
        .into_iter()
        .filter(|path| written_by_zeatend(Path::new(path)))
        .collect();
    facts.registry_files.sort();
    Ok(())
}

/// Whether the file at `path` starts with the header zeatend writes.
fn written_by_zeatend(path: &Path) -> bool {
    fs::read(path).is_ok_and(|bytes| bytes.starts_with(HEADER.as_bytes()))
}

/// Removes the files at `paths` and a directory of [`KUBESOLO_REGISTRY_DIR`] left empty.
///
/// # Errors
///
/// When a file cannot be removed.
pub(crate) fn remove(paths: &[String]) -> Result<(), InstallError> {
    for path in paths.iter().map(Path::new) {
        files::remove(path)?;
        if let Some(dir) = path.parent()
            && dir.parent() == Some(Path::new(KUBESOLO_REGISTRY_DIR))
        {
            // A directory that still holds a file of someone else's stays.
            drop(fs::remove_dir(dir));
        }
    }
    Ok(())
}
