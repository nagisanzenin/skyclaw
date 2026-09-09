//! Private, uniquely owned Chromium profiles. Never remove locks or write in a
//! personal Chrome profile. Optional imports are explicit and size bounded.
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
use temm1e_core::types::error::Temm1eError;

pub(crate) struct BrowserProfile(tempfile::TempDir);
impl BrowserProfile {
    pub(crate) fn path(&self) -> &Path {
        self.0.path()
    }
    pub(crate) async fn create(
        kind: &'static str,
        import: Option<PathBuf>,
    ) -> Result<Self, Temm1eError> {
        tokio::task::spawn_blocking(move || {
            let parent = temm1e_core::config::data_dir().join("browser-profiles");
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&parent)?;
            if fs2::available_space(&parent)? < 8 * 1024 * 1024 * 1024 {
                return Err(std::io::Error::other(
                    "browser startup requires 8 GiB free disk reserve",
                ));
            }
            let directory = tempfile::Builder::new()
                .prefix(&format!("{kind}-"))
                .tempdir_in(&parent)?;
            if let Some(source) = import {
                let target = directory.path().join("Default");
                std::fs::create_dir(&target)?;
                let mut budget = CopyBudget {
                    bytes: 128 * 1024 * 1024,
                    entries: 10_000,
                };
                for item in [
                    "Cookies",
                    "Cookies-journal",
                    "Network",
                    "Local Storage",
                    "Session Storage",
                ] {
                    let from = source.join(item);
                    if from.symlink_metadata().is_ok() {
                        copy_bounded(&from, &target.join(item), &mut budget, 0)?;
                    }
                }
            }
            Ok::<_, std::io::Error>(Self(directory))
        })
        .await
        .map_err(|_| Temm1eError::Tool("Browser profile worker failed".into()))?
        .map_err(|e| Temm1eError::Tool(format!("Browser profile: {e}")))
    }
}

struct CopyBudget {
    bytes: u64,
    entries: usize,
}
fn copy_bounded(
    source: &Path,
    target: &Path,
    budget: &mut CopyBudget,
    depth: usize,
) -> std::io::Result<()> {
    let deny = || {
        std::io::Error::other(
            "browser import exceeds limits or contains unsupported filesystem entries",
        )
    };
    if depth > 16 || budget.entries == 0 {
        return Err(deny());
    }
    budget.entries -= 1;
    let meta = source.symlink_metadata()?;
    if meta.file_type().is_symlink() {
        return Err(deny());
    }
    if meta.is_dir() {
        std::fs::create_dir(target)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy_bounded(
                &entry.path(),
                &target.join(entry.file_name()),
                budget,
                depth + 1,
            )?;
        }
    } else if meta.is_file() {
        if meta.len() > budget.bytes {
            return Err(deny());
        }
        let mut input = std::fs::File::open(source)?.take(budget.bytes + 1);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options.open(target)?;
        let mut buffer = [0; 16384];
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            if count as u64 > budget.bytes {
                return Err(deny());
            }
            output.write_all(&buffer[..count])?;
            budget.bytes -= count as u64;
        }
    } else {
        return Err(deny());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_are_bounded_and_never_modify_the_source() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::write(&source, "abcdef").unwrap();
        let mut budget = CopyBudget {
            bytes: 5,
            entries: 10,
        };
        assert!(copy_bounded(&source, &root.path().join("small"), &mut budget, 0).is_err());
        let mut budget = CopyBudget {
            bytes: 6,
            entries: 10,
        };
        copy_bounded(&source, &root.path().join("exact"), &mut budget, 0).unwrap();
        assert_eq!(budget.bytes, 0);
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "abcdef");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&source, root.path().join("link")).unwrap();
            let mut budget = CopyBudget {
                bytes: 100,
                entries: 10,
            };
            assert!(copy_bounded(
                &root.path().join("link"),
                &root.path().join("copy-link"),
                &mut budget,
                0
            )
            .is_err());
        }
    }
}
