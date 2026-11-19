//! Reading the configuration file.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use zeatend_plan::config::{Config, Document, JoinConfig};

use crate::InstallError;

/// Where `zeatend` reads its configuration when `--config` names no other file.
pub(crate) const DEFAULT_CONFIG: &str = "/etc/zeatend/config.yaml";

/// Where the configuration came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// The file `--config` names.
    File,
    /// zeabur-installer's configuration file, on a host it installed.
    Installer,
    /// No file: K3s on this host.
    Defaults,
}

/// Reads the configuration file. Without one, reads zeabur-installer's, else gives the defaults.
pub(crate) fn document_or_defaults(path: &Path) -> Result<(Document, Source), InstallError> {
    if path.exists() {
        return Ok((document(path)?, Source::File));
    }
    if let Some(document) = crate::installer::document()? {
        return Ok((document, Source::Installer));
    }
    Ok((Document::Install(Config::default()), Source::Defaults))
}

/// Reads a configuration file of either kind. A join configuration that holds its token must not
/// be readable by others.
pub(crate) fn document(path: &Path) -> Result<Document, InstallError> {
    let text = fs::read_to_string(path).map_err(InstallError::io(path))?;
    let document = Document::parse(&text)?;
    refuse_shared_token(path, &document)?;
    Ok(document)
}

/// Refuses a join configuration that holds its token in a file others can read.
pub(crate) fn refuse_shared_token(path: &Path, document: &Document) -> Result<(), InstallError> {
    if let Document::Join(JoinConfig { join, .. }) = document
        && join.token.is_some()
    {
        let mode = fs::metadata(path)
            .map_err(InstallError::io(path))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(InstallError::SharedToken {
                path: path.to_owned(),
                mode: mode & 0o777,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::Permissions;

    use super::*;

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let (document, source) = document_or_defaults(&dir.path().join("none.yaml")).unwrap();
        assert_eq!(source, Source::Defaults);
        assert_eq!(document, Document::Install(Config::default()));
    }

    #[test]
    fn a_token_in_a_file_others_can_read_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let hash = "a".repeat(64);
        fs::write(
            &path,
            format!(
                "join:\n  server: https://10.0.0.5:6443\n  token: K10{hash}::abcdef.0123456789abcdef\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&path, Permissions::from_mode(0o644)).unwrap();
        let err = document(&path).unwrap_err().to_string();
        assert!(err.contains("chmod 0600"), "{err}");
        fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(document(&path).unwrap(), Document::Join(_)));
    }

    #[test]
    fn a_kubesolo_file_is_an_install() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        fs::write(&path, "distribution: kubesolo\n").unwrap();
        let Document::Install(config) = document(&path).unwrap() else {
            unreachable!("a file with no join section is an install")
        };
        assert!(config.distribution.rancher().is_none());
    }
}
