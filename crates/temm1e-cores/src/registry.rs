//! Core registry — discovers, loads, and indexes core definitions.

use std::collections::HashMap;
use std::path::Path;

use temm1e_core::types::error::Temm1eError;
use tracing::{debug, info, warn};

use crate::definition::{parse_core_content, CoreDefinition};

/// Registry that holds all loaded core definitions, indexed by name.
pub struct CoreRegistry {
    cores: HashMap<String, CoreDefinition>,
}

impl CoreRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            cores: HashMap::new(),
        }
    }

    /// Load cores from the global directory (`~/.temm1e/cores/`) and an
    /// optional workspace directory (`<workspace>/cores/`).
    ///
    /// Workspace cores override global cores with the same name.
    pub async fn load(&mut self, workspace_path: Option<&Path>) -> Result<(), Temm1eError> {
        self.cores.clear();

        // Global cores: ~/.temm1e/cores/
        {
            let global_dir = temm1e_core::config::data_dir().join("cores");
            if global_dir.is_dir() {
                self.load_from_dir(&global_dir).await?;
            }
        }

        // Workspace cores: <workspace>/cores/
        if let Some(ws) = workspace_path {
            let ws_dir = ws.join("cores");
            if ws_dir.is_dir() {
                self.load_from_dir(&ws_dir).await?;
            }
        }

        info!(count = self.cores.len(), "TemDOS cores loaded");
        Ok(())
    }

    /// Load all `.md` files from a directory into the registry.
    async fn load_from_dir(&mut self, dir: &Path) -> Result<(), Temm1eError> {
        let entries = std::fs::read_dir(dir).map_err(|e| {
            Temm1eError::Config(format!(
                "Failed to read cores directory {}: {e}",
                dir.display()
            ))
        })?;

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!("Failed to read directory entry: {e}");
                    continue;
                }
            };

            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "md") {
                match self.load_core_file(&path) {
                    Ok(name) => {
                        debug!(core = %name, path = %path.display(), "Loaded core definition");
                    }
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "Failed to load core definition");
                    }
                }
            }
        }

        Ok(())
    }

    /// Load a single core definition file and add it to the registry.
    fn load_core_file(&mut self, path: &Path) -> Result<String, Temm1eError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            Temm1eError::Config(format!("Failed to read core file {}: {e}", path.display()))
        })?;

        let definition = parse_core_content(&content, path.to_path_buf())?;
        let name = definition.name.clone();

        if self.cores.contains_key(&name) {
            debug!(
                core = %name,
                path = %path.display(),
                "Overriding existing core definition"
            );
        }

        self.cores.insert(name.clone(), definition);
        Ok(name)
    }

    /// Hot-load a single core definition into the registry.
    pub fn load_core(&mut self, definition: CoreDefinition) {
        info!(core = %definition.name, version = %definition.version, "Hot-loaded core");
        self.cores.insert(definition.name.clone(), definition);
    }

    /// Get a core definition by name.
    pub fn get_core(&self, name: &str) -> Option<&CoreDefinition> {
        self.cores.get(name)
    }

    /// List all available core definitions.
    pub fn list_cores(&self) -> Vec<&CoreDefinition> {
        let mut cores: Vec<_> = self.cores.values().collect();
        cores.sort_by_key(|c| &c.name);
        cores
    }

    /// Number of loaded cores.
    pub fn len(&self) -> usize {
        self.cores.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.cores.is_empty()
    }

    /// Build a formatted listing of available cores for system prompt injection.
    pub fn format_cores_listing(&self) -> String {
        if self.cores.is_empty() {
            return String::new();
        }

        let mut listing = String::new();
        for core in self.list_cores() {
            listing.push_str(&format!("- **{}** — {}\n", core.name, core.description));
        }
        listing
    }
}

impl Default for CoreRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[tokio::test]
    async fn load_from_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = CoreRegistry::new();
        registry.load_from_dir(dir.path()).await.unwrap();
        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn load_from_dir_with_cores() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("test.md"),
            "---\nname: test\ndescription: Test core\nversion: \"1.0.0\"\n---\nBody",
        )
        .unwrap();

        let mut registry = CoreRegistry::new();
        registry.load_from_dir(dir.path()).await.unwrap();
        assert_eq!(registry.len(), 1);
        assert!(registry.get_core("test").is_some());
    }

    #[tokio::test]
    async fn workspace_overrides_global() {
        let dir = tempfile::tempdir().unwrap();
        let core1 = CoreDefinition {
            name: "test".to_string(),
            description: "Global".to_string(),
            version: "1.0.0".to_string(),
            temperature: None,
            system_prompt: "Global prompt".to_string(),
            source_path: PathBuf::from("/global/test.md"),
        };
        let core2 = CoreDefinition {
            name: "test".to_string(),
            description: "Workspace".to_string(),
            version: "2.0.0".to_string(),
            temperature: None,
            system_prompt: "Workspace prompt".to_string(),
            source_path: dir.path().join("test.md"),
        };

        let mut registry = CoreRegistry::new();
        registry.load_core(core1);
        registry.load_core(core2);

        let core = registry.get_core("test").unwrap();
        assert_eq!(core.description, "Workspace");
        assert_eq!(core.version, "2.0.0");
    }

    #[test]
    fn format_cores_listing() {
        let mut registry = CoreRegistry::new();
        registry.load_core(CoreDefinition {
            name: "alpha".to_string(),
            description: "Alpha core".to_string(),
            version: "1.0.0".to_string(),
            temperature: None,
            system_prompt: String::new(),
            source_path: PathBuf::new(),
        });
        registry.load_core(CoreDefinition {
            name: "beta".to_string(),
            description: "Beta core".to_string(),
            version: "1.0.0".to_string(),
            temperature: None,
            system_prompt: String::new(),
            source_path: PathBuf::new(),
        });

        let listing = registry.format_cores_listing();
        assert!(listing.contains("**alpha**"));
        assert!(listing.contains("**beta**"));
    }
}
