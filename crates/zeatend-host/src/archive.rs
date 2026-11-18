//! Unpacking release tarballs.

use std::fs::File;
use std::path::Path;

use flate2::read::GzDecoder;

use crate::HostError;

/// Unpacks a `.tar.gz` into `dest`, keeping modes. Entries leaving `dest` are refused by `tar`.
///
/// # Errors
///
/// When the archive cannot be read or a file cannot be written.
pub fn unpack_tar_gz(archive: &Path, dest: &Path) -> Result<(), HostError> {
    let file = File::open(archive).map_err(HostError::io(archive))?;
    let mut tar = tar::Archive::new(GzDecoder::new(file));
    tar.set_preserve_permissions(true);
    tar.set_overwrite(true);
    tar.unpack(dest).map_err(HostError::io(archive))
}

#[cfg(test)]
mod tests {
    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    fn tar_gz(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
        for (path, data, mode) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(u64::try_from(data.len()).unwrap());
            header.set_mode(*mode);
            header.set_cksum();
            builder.append_data(&mut header, path, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn unpacks_files_with_their_modes() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.tar.gz");
        std::fs::write(&archive, tar_gz(&[("bin/rke2", b"#!/bin/sh\n", 0o755)])).unwrap();
        let dest = dir.path().join("out");
        unpack_tar_gz(&archive, &dest).unwrap();
        let path = dest.join("bin/rke2");
        assert_eq!(std::fs::read(&path).unwrap(), b"#!/bin/sh\n");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[test]
    fn a_missing_archive_names_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let err = unpack_tar_gz(&dir.path().join("none.tar.gz"), dir.path()).unwrap_err();
        assert!(err.to_string().contains("none.tar.gz"), "{err}");
    }
}
