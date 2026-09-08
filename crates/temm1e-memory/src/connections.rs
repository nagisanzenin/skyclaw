//! Separate the selected memory backend's location from SQLite service metadata.
use std::path::Path;
use temm1e_core::types::config::MemoryConfig;

pub struct MemoryConnections {
    pub primary: String,
    pub sqlite_state: String,
    pub retained_legacy_markdown: bool,
}
impl MemoryConnections {
    pub fn resolve(config: &MemoryConfig, profile: &Path, working_directory: &Path) -> Self {
        let default_sqlite = format!("sqlite:{}/memory.db?mode=rwc", profile.display());
        let mut retained_legacy_markdown = false;
        let primary = config.path.clone().unwrap_or_else(|| {
            if config.backend == "markdown" {
                // Older TUI code handed Markdown a SQLite URL as a directory.
                // Preserve an existing app-created location instead of silently
                // hiding its contents. Do not create that legacy layout anew.
                let legacy = working_directory.join(&default_sqlite);
                if legacy.is_dir() {
                    retained_legacy_markdown = true;
                    legacy.to_string_lossy().into_owned()
                } else {
                    profile.join("memory").to_string_lossy().into_owned()
                }
            } else {
                default_sqlite.clone()
            }
        });
        let sqlite_state = if config.backend == "sqlite" {
            primary.clone() // Preserve existing custom/default SQLite usage tables.
        } else {
            default_sqlite // Markdown directories are never SQLite connections.
        };
        Self {
            primary,
            sqlite_state,
            retained_legacy_markdown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backend_location_and_usage_connection_are_distinct_when_required() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile");
        let mut config = MemoryConfig {
            path: Some("sqlite::memory:".into()),
            ..Default::default()
        };
        let sqlite = MemoryConnections::resolve(&config, &profile, directory.path());
        assert_eq!(sqlite.primary, "sqlite::memory:");
        assert_eq!(sqlite.sqlite_state, sqlite.primary);
        config.backend = "markdown".into();
        config.path = Some(
            directory
                .path()
                .join("notes")
                .to_string_lossy()
                .into_owned(),
        );
        let markdown = MemoryConnections::resolve(&config, &profile, directory.path());
        assert_eq!(markdown.primary, config.path.clone().unwrap());
        assert_eq!(
            markdown.sqlite_state,
            format!("sqlite:{}/memory.db?mode=rwc", profile.display())
        );
        config.path = None;
        let fresh = MemoryConnections::resolve(&config, &profile, directory.path());
        assert_eq!(fresh.primary, profile.join("memory").to_string_lossy());
        assert!(!fresh.retained_legacy_markdown);
    }
    #[cfg(unix)]
    #[test]
    fn existing_legacy_markdown_data_is_retained_without_moving_or_recreating_it() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile");
        let legacy = directory
            .path()
            .join(format!("sqlite:{}/memory.db?mode=rwc", profile.display()));
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("MEMORY.md"), "existing user memory").unwrap();
        let config = MemoryConfig {
            backend: "markdown".into(),
            ..Default::default()
        };
        let connections = MemoryConnections::resolve(&config, &profile, directory.path());
        assert!(connections.retained_legacy_markdown);
        assert_eq!(connections.primary, legacy.to_string_lossy());
        assert_eq!(
            std::fs::read_to_string(legacy.join("MEMORY.md")).unwrap(),
            "existing user memory"
        );
        assert!(!profile.join("memory").exists());
    }
}
