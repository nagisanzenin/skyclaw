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
    persist_private(temporary, path)?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn persist_private(temporary: tempfile::NamedTempFile, path: &Path) -> io::Result<()> {
    #[cfg(not(windows))]
    temporary.persist(path).map_err(|e| e.error)?;
    #[cfg(windows)]
    {
        // MoveFileEx can lose a race with short-lived readers/scanners. Keep
        // the same fully written temporary file; never delete the destination
        // or fall back to truncating it. Persistent ACL/handle failures remain
        // errors after a bounded retry window (50 sleeps, at most 500ms).
        let mut temporary = temporary;
        for attempt in 0..=50 {
            match temporary.persist(path) {
                Ok(_) => return Ok(()),
                Err(error) => {
                    let retryable = matches!(error.error.raw_os_error(), Some(5 | 32 | 33));
                    if !retryable || attempt == 50 {
                        return Err(error.error);
                    }
                    temporary = error.file;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
    }
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
    #[cfg(windows)]
    #[test]
    fn replacement_survives_short_reader_and_preserves_file_when_blocked() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credential");
        write_private_atomic(&path, b"original").unwrap();
        // Deny delete sharing deliberately, rather than depending on a scanner
        // or scheduler race to reproduce MoveFileEx failure on the CI host.
        let reader = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            drop(reader);
        });
        write_private_atomic(&path, b"replacement").unwrap();
        release.join().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");

        let reader = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        assert!(write_private_atomic(&path, b"must not appear").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        drop(reader);
        write_private_atomic(&path, b"after release").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"after release");
    }
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
