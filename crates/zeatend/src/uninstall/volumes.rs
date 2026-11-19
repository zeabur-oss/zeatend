//! Keeping the volumes out of the uninstall's way, or removing them.

use std::fs;
use std::path::Path;

use zeatend_plan::uninstall::Volume;

use super::{at, exists, io, remove_tree};
use crate::InstallError;

/// Whether `volume` exists under `root`, in its place or stashed by an uninstall that stopped.
pub(super) fn present(root: &Path, volume: Volume) -> bool {
    exists(root, volume.path) || volume.stash.is_some_and(|stash| exists(root, stash))
}

/// Moves the volumes that the uninstall would remove out of its way.
pub(super) fn stash(root: &Path, volumes: &[Volume]) -> Result<(), InstallError> {
    for volume in volumes {
        let Some(stash) = volume.stash else { continue };
        let (path, stash) = (at(root, volume.path), at(root, stash));
        if path.symlink_metadata().is_err() {
            continue;
        }
        if stash.symlink_metadata().is_ok() {
            return Err(InstallError::StashExists { path, stash });
        }
        if let Err(source) = fs::rename(&path, &stash) {
            return Err(InstallError::Stash {
                path,
                stash,
                source,
            });
        }
    }
    Ok(())
}

/// Moves stashed volumes back to their place.
pub(super) fn restore(root: &Path, volumes: &[Volume]) -> Result<(), InstallError> {
    for volume in volumes {
        let Some(stash) = volume.stash else { continue };
        let (path, stash) = (at(root, volume.path), at(root, stash));
        if stash.symlink_metadata().is_err() {
            continue;
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io(dir))?;
        }
        if let Err(source) = fs::rename(&stash, &path) {
            return Err(InstallError::Restore {
                path,
                stash,
                source,
            });
        }
    }
    Ok(())
}

/// Removes the volumes, in their place and stashed.
pub(super) fn delete(root: &Path, volumes: &[Volume]) -> Result<(), InstallError> {
    for volume in volumes {
        for path in [Some(volume.path), volume.stash].into_iter().flatten() {
            remove_tree(&at(root, path))?;
        }
    }
    Ok(())
}
