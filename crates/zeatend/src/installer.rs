//! What zeatend reads from a host that zeabur-installer installed.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use zeatend_plan::cluster::Distribution;
use zeatend_plan::config::Document;
use zeatend_plan::installer::{self, CONFIG_FILE, VERSION_FILE};
use zeatend_plan::record::{Record, Writer};

use crate::{InstallError, apply};

/// zeatend's configuration from the installer's configuration file; `None` when there is none.
/// A join token in it must not be readable by others, as in zeatend's own file. On K3s and RKE2,
/// `disable` holds the packaged components that the installed cluster turns off.
pub(crate) fn document() -> Result<Option<Document>, InstallError> {
    match fs::read_to_string(CONFIG_FILE) {
        Ok(text) => {
            let mut document = installer::document(&text)?;
            crate::load::refuse_shared_token(Path::new(CONFIG_FILE), &document)?;
            let distribution = match &document {
                Document::Install(config) => config.distribution,
                Document::Join(config) => config.distribution,
            };
            if distribution.rancher().is_some()
                && let Ok(config) =
                    fs::read_to_string(format!("{}/config.yaml", distribution.layout().config_dir))
            {
                document = installer::keep_disabled(document, &config)?;
            }
            Ok(Some(document))
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(InstallError::io(CONFIG_FILE)(e)),
    }
}

/// The install record: the one zeatend wrote, else the installer's.
///
/// # Errors
///
/// When zeatend's record cannot be read.
pub(crate) async fn read_record(
    distribution: Distribution,
) -> Result<Option<Record>, InstallError> {
    match apply::read_record(Path::new(distribution.layout().state_dir))? {
        Some(record) => Ok(Some(record)),
        None => Ok(record(distribution).await),
    }
}

/// The record of an install of `distribution` that the installer made: its state directory names
/// the installer's version, or the node holds the join token file of a node it joined. The
/// release is the one the installed binary reports.
async fn record(distribution: Distribution) -> Option<Record> {
    let installer_version =
        fs::read_to_string(Path::new(installer::state_dir(distribution)).join(VERSION_FILE))
            .ok()
            .map(|version| version.trim().to_owned())
            .filter(|version| !version.is_empty());
    let joined = distribution
        .rancher()
        .is_some_and(|rancher| Path::new(&installer::join_token_file(rancher)).exists());
    if installer_version.is_none() && !joined {
        return None;
    }
    Some(Record {
        writer: Writer::Installer(installer_version),
        distribution: Some(distribution.id().to_owned()),
        version: apply::installed_release(Path::new(distribution.layout().binary)).await,
    })
}
