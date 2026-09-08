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

#[cfg(test)]
mod tests {
    use super::*;
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
