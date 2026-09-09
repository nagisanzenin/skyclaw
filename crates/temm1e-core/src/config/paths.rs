//! Central application data location, including isolated test/deployment profiles.
use std::path::{Path, PathBuf};

/// Defaults to ~/.temm1e. An explicit TEMM1E_DATA_DIR redirects application data
/// without changing the user's HOME or unrelated tools' configuration.
pub fn data_dir() -> PathBuf {
    resolve_data_dir(
        std::env::var_os("TEMM1E_DATA_DIR")
            .as_deref()
            .map(Path::new),
        dirs::home_dir().as_deref(),
        &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    )
}

fn resolve_data_dir(selected: Option<&Path>, home: Option<&Path>, cwd: &Path) -> PathBuf {
    match selected.filter(|p| !p.as_os_str().is_empty()) {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => cwd.join(path),
        None => home.unwrap_or(cwd).join(".temm1e"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn override_does_not_repurpose_home() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let profile = root.path().join("profile");
        assert_eq!(
            resolve_data_dir(None, Some(&home), root.path()),
            home.join(".temm1e")
        );
        assert_eq!(
            resolve_data_dir(Some(&profile), Some(&home), root.path()),
            profile
        );
        assert_eq!(
            resolve_data_dir(Some(Path::new("relative")), Some(&home), root.path()),
            root.path().join("relative")
        );
    }
}
