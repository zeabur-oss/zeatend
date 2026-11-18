//! Writing files atomically with a mode.

use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

use crate::HostError;

/// Writes `content` to `path` with `mode` through a temporary file and a rename, so a reader sees
/// the old file or the new one. Returns whether anything changed.
///
/// # Errors
///
/// When a directory cannot be made or the file cannot be written.
pub fn write(path: &Path, mode: u32, content: &[u8]) -> Result<bool, HostError> {
    if let Ok(meta) = fs::metadata(path)
        && meta.permissions().mode() & 0o7777 == mode
        && fs::read(path).is_ok_and(|old| old == content)
    {
        return Ok(false);
    }
    let dir = path.parent().unwrap_or_else(|| Path::new("/"));
    fs::create_dir_all(dir).map_err(HostError::io(dir))?;
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let temp = dir.join(format!(".{name}.zeatend"));
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(&temp)
        .map_err(HostError::io(&temp))?;
    // `mode` above is masked by the umask; set it exactly.
    file.set_permissions(Permissions::from_mode(mode))
        .map_err(HostError::io(&temp))?;
    file.write_all(content).map_err(HostError::io(&temp))?;
    file.sync_all().map_err(HostError::io(&temp))?;
    fs::rename(&temp, path).map_err(HostError::io(path))?;
    Ok(true)
}

/// Makes a directory and its parents, and sets its mode.
///
/// # Errors
///
/// When it cannot be made or its mode cannot be set.
pub fn make_dir(path: &Path, mode: u32) -> Result<(), HostError> {
    fs::create_dir_all(path).map_err(HostError::io(path))?;
    fs::set_permissions(path, Permissions::from_mode(mode)).map_err(HostError::io(path))
}

/// Copies an executable to `dest` with `mode` through a temporary file and a rename, so a running
/// copy is not changed.
///
/// # Errors
///
/// When it cannot be read or written.
pub fn install(source: &Path, dest: &Path, mode: u32) -> Result<(), HostError> {
    let dir = dest.parent().unwrap_or_else(|| Path::new("/"));
    fs::create_dir_all(dir).map_err(HostError::io(dir))?;
    let name = dest
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let temp = dir.join(format!(".{name}.zeatend"));
    fs::copy(source, &temp).map_err(HostError::io(&temp))?;
    fs::set_permissions(&temp, Permissions::from_mode(mode)).map_err(HostError::io(&temp))?;
    fs::rename(&temp, dest).map_err(HostError::io(dest))
}

/// Removes the file at `path`. A missing file is not an error.
///
/// # Errors
///
/// When it exists and cannot be removed.
pub fn remove(path: &Path) -> Result<(), HostError> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(HostError::io(path)(e)),
        _ => Ok(()),
    }
}

/// Sets `mode` on the regular files in `dir` whose names end with `suffix`. Returns how many
/// changed.
///
/// A symbolic link is not followed. The mode is set on the open file (`fchmod`), and only when
/// the open file has the device and inode that `lstat` found for the name, so a name that changes
/// to a link or another file between the two calls is left as it is.
///
/// # Errors
///
/// When the directory cannot be read or a mode cannot be set.
pub fn set_modes(dir: &Path, suffix: &str, mode: u32) -> Result<usize, HostError> {
    let mut changed = 0;
    for entry in fs::read_dir(dir).map_err(HostError::io(dir))? {
        let path = entry.map_err(HostError::io(dir))?.path();
        let matches = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(suffix));
        if !matches {
            continue;
        }
        let named = fs::symlink_metadata(&path).map_err(HostError::io(&path))?;
        if !named.is_file() || named.permissions().mode() & 0o7777 == mode {
            continue;
        }
        let file = File::open(&path).map_err(HostError::io(&path))?;
        let opened = file.metadata().map_err(HostError::io(&path))?;
        if !opened.is_file() || (opened.dev(), opened.ino()) != (named.dev(), named.ino()) {
            continue;
        }
        file.set_permissions(Permissions::from_mode(mode))
            .map_err(HostError::io(&path))?;
        changed += 1;
    }
    Ok(changed)
}

/// Reads a secret file, refusing one others can read.
///
/// # Errors
///
/// When it cannot be read or its mode lets the group or others read it.
pub fn read_secret(path: &Path) -> Result<Vec<u8>, HostError> {
    let mut file = File::open(path).map_err(HostError::io(path))?;
    let mode = file
        .metadata()
        .map_err(HostError::io(path))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        return Err(HostError::Io {
            path: path.to_owned(),
            source: std::io::Error::other(format!(
                "mode {:o} lets others read it; chmod 0600",
                mode & 0o777
            )),
        });
    }
    let mut bytes = Vec::new();
    std::io::copy(&mut file, &mut bytes).map_err(HostError::io(path))?;
    Ok(bytes)
}

/// 64 random bytes as 128 hexadecimal digits.
///
/// # Errors
///
/// When `/dev/urandom` cannot be read.
pub fn random_hex() -> Result<String, HostError> {
    random_bytes(64).map(hex::encode)
}

/// `n` random bytes.
///
/// # Errors
///
/// When `/dev/urandom` cannot be read.
pub fn random_bytes(n: usize) -> Result<Vec<u8>, HostError> {
    let mut bytes = vec![0_u8; n];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(HostError::io("/dev/urandom"))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_once_with_the_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b.yaml");
        assert!(write(&path, 0o600, b"x").unwrap());
        assert!(!write(&path, 0o600, b"x").unwrap());
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(write(&path, 0o644, b"x").unwrap());
        assert!(read_secret(&path).is_err());
        assert!(write(&path, 0o600, b"y").unwrap());
        assert_eq!(read_secret(&path).unwrap(), b"y");
        assert_eq!(random_hex().unwrap().len(), 128);
        assert_eq!(random_bytes(24).unwrap().len(), 24);
    }

    #[test]
    fn set_modes_skips_links_and_other_names() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let mode = |path: &Path| fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777;
        let target = outside.path().join("target.crt");
        fs::write(&target, "t").unwrap();
        fs::set_permissions(&target, Permissions::from_mode(0o644)).unwrap();
        let own = dir.path().join("server.crt");
        fs::write(&own, "c").unwrap();
        fs::set_permissions(&own, Permissions::from_mode(0o644)).unwrap();
        let other = dir.path().join("server.key");
        fs::write(&other, "k").unwrap();
        fs::set_permissions(&other, Permissions::from_mode(0o644)).unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("link.crt")).unwrap();

        assert_eq!(set_modes(dir.path(), ".crt", 0o600).unwrap(), 1);
        assert_eq!(mode(&own), 0o600);
        assert_eq!(mode(&other), 0o644);
        assert_eq!(mode(&target), 0o644);
        assert_eq!(set_modes(dir.path(), ".crt", 0o600).unwrap(), 0);
    }
}
