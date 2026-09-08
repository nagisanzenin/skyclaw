//! Atomic credential replacement. Temporary contents are private before writing.
use std::io::{self, Write};
use std::path::Path;

pub fn write_private_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    temporary.write_all(contents)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

/// An advisory lock on a stable sidecar inode. The sidecar must never be
/// deleted or atomically replaced while clients may be using it.
/// Closing the descriptor releases the OS lock, including after process exit.
pub struct PrivateFileLock(std::fs::File);
impl PrivateFileLock {
    pub fn try_exclusive(path: &Path) -> io::Result<Option<Self>> {
        let parent = path.parent().unwrap_or(Path::new("."));
        let mut directory = std::fs::DirBuilder::new();
        directory.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(0o700);
        }
        directory.create(parent)?;
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => Ok(Some(Self(file))),
            Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => Ok(None),
            Err(e) => Err(e),
        }
    }
}
impl Drop for PrivateFileLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_handles_contend_and_release_without_deleting_sidecar() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("credential.lock");
        let first = PrivateFileLock::try_exclusive(&path).unwrap().unwrap();
        assert!(PrivateFileLock::try_exclusive(&path).unwrap().is_none());
        drop(first);
        assert!(path.exists());
        assert!(PrivateFileLock::try_exclusive(&path).unwrap().is_some());
    }

    #[test]
    fn replaces_complete_contents_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credential");
        write_private_atomic(&path, b"original").unwrap();
        write_private_atomic(&path, b"replacement").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn replacing_symlink_does_not_modify_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("credential");
        std::fs::write(&target, "untouched").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write_private_atomic(&link, b"credential").unwrap();
        assert_eq!(std::fs::read(target).unwrap(), b"untouched");
        assert!(!std::fs::symlink_metadata(link)
            .unwrap()
            .file_type()
            .is_symlink());
    }
}
