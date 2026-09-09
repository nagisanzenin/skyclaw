//! Custom model registry — user-defined models with context window, max
//! output tokens, and per-million-token pricing.
//!
//! Users running LM Studio / Ollama / vLLM / custom proxies need to tell
//! Tem about their local models because the hardcoded registry at
//! [`crate::types::model_registry`] only ships with first-party models
//! from known providers. Custom models live in a separate file so the
//! `credentials.toml` format stays byte-identical and users who never use
//! the feature never see the storage file.
//!
//! Storage: `~/.temm1e/custom_models.toml`
//!
//! Format (flat array — one `[[models]]` entry per custom model):
//! ```toml
//! [[models]]
//! provider = "openai"
//! name = "qwen3-coder-30b-a3b"
//! context_window = 262144
//! max_output_tokens = 65536
//! input_price_per_1m = 0.0
//! output_price_per_1m = 0.0
//! ```
//!
//! Lookup is provider-scoped: `lookup_custom_model("openai", "qwen3-coder")`
//! returns `None` if the entry exists only for a different provider. This
//! prevents custom models from accidentally shadowing first-party model
//! names.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::types::error::Temm1eError;

/// A user-defined model entry with capability limits and pricing.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CustomModel {
    /// Explicit operator capability for this provider/model. Omitted is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_input: Option<bool>,
    /// Provider scope — the model is only valid for this provider.
    pub provider: String,
    /// Model ID as the proxy API expects it (e.g. `qwen3-coder-30b-a3b`).
    pub name: String,
    /// Maximum input context window in tokens.
    pub context_window: usize,
    /// Maximum output tokens the model can generate.
    pub max_output_tokens: usize,
    /// USD per 1M input tokens. Legacy omitted/zero rates are not proof of free usage.
    #[serde(default)]
    pub input_price_per_1m: f64,
    /// USD per 1M output tokens.
    #[serde(default)]
    pub output_price_per_1m: f64,
    /// Explicitly attest both configured rates, including a deliberate zero tariff.
    /// Existing nonzero rates remain effective without this migration flag.
    #[serde(default)]
    pub pricing_verified: bool,
}

/// Top-level file layout — a flat array of `CustomModel` entries.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CustomModelsFile {
    #[serde(default)]
    pub models: Vec<CustomModel>,
}

/// Returns `~/.temm1e/custom_models.toml`.
pub fn custom_models_path() -> PathBuf {
    crate::config::data_dir().join("custom_models.toml")
}

/// Load the full custom models file.
///
/// Missing file → empty list. Parse errors → empty list with a warning log
/// (graceful fallback so a malformed file never crashes the user's session).
pub fn load_custom_models() -> CustomModelsFile {
    match load_custom_models_checked() {
        Ok(file) => file,
        Err(error) => {
            tracing::warn!(%error, "Custom models unavailable; using registry fallback");
            CustomModelsFile::default()
        }
    }
}

/// Mutations must distinguish missing data from unreadable or malformed data.
/// Read-only legacy callers retain their explicit fallback through the public loader.
fn load_custom_models_checked() -> Result<CustomModelsFile, Temm1eError> {
    let path = custom_models_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CustomModelsFile::default())
        }
        Err(error) => {
            return Err(Temm1eError::Config(format!(
                "Cannot read custom models at {}: {error}",
                path.display()
            )))
        }
    };
    toml::from_str(&content).map_err(|_| {
        Temm1eError::Config(format!(
            "Malformed custom models at {}; repair the file before updating it",
            path.display()
        ))
    })
}

fn lock_custom_models() -> Result<crate::private_file::PrivateFileLock, Temm1eError> {
    crate::private_file::PrivateFileLock::try_exclusive(
        &custom_models_path().with_extension("lock"),
    )
    .map_err(|error| Temm1eError::Config(format!("Cannot lock custom models: {error}")))?
    .ok_or_else(|| {
        Temm1eError::Config(
            "Another custom-model update is in progress; retry after it finishes".into(),
        )
    })
}

fn write_custom_models_locked(file: &CustomModelsFile) -> Result<(), Temm1eError> {
    let content = toml::to_string_pretty(file).map_err(|error| {
        Temm1eError::Config(format!("Failed to serialize custom models: {error}"))
    })?;
    crate::private_file::write_private_atomic(&custom_models_path(), content.as_bytes())
        .map_err(|error| Temm1eError::Config(format!("Failed to persist custom models: {error}")))
}

/// Atomically replace the complete file under the shared writer lock.
/// This is an explicit replacement, not a compare-and-swap of a stale caller snapshot.
pub fn save_custom_models(file: &CustomModelsFile) -> Result<(), Temm1eError> {
    let _lock = lock_custom_models()?;
    write_custom_models_locked(file)
}

/// Add or update a custom model. Upsert by `(provider, name)` — existing
/// entries with the same key are replaced.
pub fn upsert_custom_model(model: CustomModel) -> Result<(), Temm1eError> {
    let _lock = lock_custom_models()?;
    let mut file = load_custom_models_checked()?;
    if let Some(existing) = file
        .models
        .iter_mut()
        .find(|m| m.provider == model.provider && m.name == model.name)
    {
        *existing = model;
    } else {
        file.models.push(model);
    }
    write_custom_models_locked(&file)
}

/// Remove all custom models matching `name` in the given provider scope.
///
/// Returns the number of entries removed. If `provider` is `None`, removes
/// all matches across every provider (use with care — prefer scoped removal).
pub fn remove_custom_model(name: &str, provider: Option<&str>) -> Result<usize, Temm1eError> {
    let _lock = lock_custom_models()?;
    let mut file = load_custom_models_checked()?;
    let before = file.models.len();
    file.models
        .retain(|m| !(m.name == name && provider.is_none_or(|p| m.provider == p)));
    let removed = before - file.models.len();
    if removed > 0 {
        write_custom_models_locked(&file)?;
    }
    Ok(removed)
}

/// Scoped lookup — find a custom model for the given provider + name.
pub fn lookup_custom_model(provider: &str, name: &str) -> Option<CustomModel> {
    load_custom_models()
        .models
        .into_iter()
        .find(|m| m.provider == provider && m.name == name)
}

/// All custom models for a given provider, in storage order.
pub fn custom_models_for_provider(provider: &str) -> Vec<CustomModel> {
    load_custom_models()
        .models
        .into_iter()
        .filter(|m| m.provider == provider)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_and_explicit_image_capability_survive_toml_roundtrip() {
        let legacy = "provider = 'openai'\nname = 'custom'\ncontext_window = 8192\nmax_output_tokens = 1024\n";
        for capability in [None, Some(false), Some(true)] {
            let mut model: CustomModel = toml::from_str(legacy).unwrap();
            assert_eq!(model.image_input, None);
            model.image_input = capability;
            let encoded = toml::to_string(&model).unwrap();
            assert_eq!(encoded.contains("image_input"), capability.is_some());
            assert_eq!(
                toml::from_str::<CustomModel>(&encoded).unwrap().image_input,
                capability
            );
        }
    }

    // ── TOML round-trip (pure data, no disk I/O) ─────────────────────

    #[test]
    fn empty_file_is_valid() {
        let file: CustomModelsFile = toml::from_str("").unwrap();
        assert!(file.models.is_empty());
    }

    #[test]
    fn round_trip_custom_model() {
        let file = CustomModelsFile {
            models: vec![CustomModel {
                image_input: None,
                provider: "openai".into(),
                name: "qwen3-coder-30b-a3b".into(),
                context_window: 262144,
                max_output_tokens: 65536,
                input_price_per_1m: 0.0,
                output_price_per_1m: 0.0,
                pricing_verified: false,
            }],
        };
        let toml_str = toml::to_string_pretty(&file).unwrap();
        let parsed: CustomModelsFile = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.models.len(), 1);
        assert_eq!(parsed.models[0].name, "qwen3-coder-30b-a3b");
        assert_eq!(parsed.models[0].context_window, 262144);
        assert_eq!(parsed.models[0].max_output_tokens, 65536);
        assert_eq!(parsed.models[0].input_price_per_1m, 0.0);
    }

    #[test]
    fn pricing_defaults_to_zero_when_omitted() {
        let toml_str = r#"
            [[models]]
            provider = "openai"
            name = "free-model"
            context_window = 100000
            max_output_tokens = 16000
        "#;
        let file: CustomModelsFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.models.len(), 1);
        assert_eq!(file.models[0].input_price_per_1m, 0.0);
        assert_eq!(file.models[0].output_price_per_1m, 0.0);
    }

    #[test]
    fn multiple_providers_in_same_file() {
        let toml_str = r#"
            [[models]]
            provider = "openai"
            name = "qwen3-coder"
            context_window = 262144
            max_output_tokens = 65536

            [[models]]
            provider = "anthropic"
            name = "claude-custom"
            context_window = 200000
            max_output_tokens = 64000
            input_price_per_1m = 3.0
            output_price_per_1m = 15.0
        "#;
        let file: CustomModelsFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.models.len(), 2);
        assert_eq!(file.models[0].provider, "openai");
        assert_eq!(file.models[1].provider, "anthropic");
        assert_eq!(file.models[1].input_price_per_1m, 3.0);
    }

    #[test]
    fn malformed_toml_falls_back_to_empty() {
        // Not testing the load_custom_models disk path (that touches HOME);
        // just verifying the parser rejects malformed input and our wrapper
        // returns default on parse error.
        let parsed: Result<CustomModelsFile, _> =
            toml::from_str("this is { not valid toml at all }");
        assert!(parsed.is_err());
    }

    // ── Pure-function helpers used by handlers (no disk I/O) ─────────

    fn sample_file() -> CustomModelsFile {
        CustomModelsFile {
            models: vec![
                CustomModel {
                    image_input: None,
                    provider: "openai".into(),
                    name: "qwen3-coder".into(),
                    context_window: 262144,
                    max_output_tokens: 65536,
                    input_price_per_1m: 0.0,
                    output_price_per_1m: 0.0,
                    pricing_verified: false,
                },
                CustomModel {
                    image_input: None,
                    provider: "openai".into(),
                    name: "llama-3.3-70b".into(),
                    context_window: 131072,
                    max_output_tokens: 16384,
                    input_price_per_1m: 0.0,
                    output_price_per_1m: 0.0,
                    pricing_verified: false,
                },
                CustomModel {
                    image_input: None,
                    provider: "anthropic".into(),
                    name: "claude-custom".into(),
                    context_window: 200000,
                    max_output_tokens: 64000,
                    input_price_per_1m: 3.0,
                    output_price_per_1m: 15.0,
                    pricing_verified: false,
                },
            ],
        }
    }

    /// Pure lookup on an in-memory file (mirrors `lookup_custom_model`
    /// without touching disk — used to unit test the scoping logic).
    fn lookup_in_file<'a>(
        file: &'a CustomModelsFile,
        provider: &str,
        name: &str,
    ) -> Option<&'a CustomModel> {
        file.models
            .iter()
            .find(|m| m.provider == provider && m.name == name)
    }

    #[test]
    fn lookup_is_scoped_by_provider() {
        let file = sample_file();
        assert!(lookup_in_file(&file, "openai", "qwen3-coder").is_some());
        // Same name under a different provider — must NOT match
        assert!(lookup_in_file(&file, "anthropic", "qwen3-coder").is_none());
        // Different name under the correct provider — must NOT match
        assert!(lookup_in_file(&file, "openai", "nonexistent").is_none());
    }

    #[test]
    fn custom_models_for_provider_pure_filter() {
        let file = sample_file();
        let openai: Vec<_> = file
            .models
            .iter()
            .filter(|m| m.provider == "openai")
            .collect();
        assert_eq!(openai.len(), 2);
        let anthropic: Vec<_> = file
            .models
            .iter()
            .filter(|m| m.provider == "anthropic")
            .collect();
        assert_eq!(anthropic.len(), 1);
        assert_eq!(anthropic[0].name, "claude-custom");
    }

    #[test]
    fn upsert_replaces_existing_scoped_by_provider_and_name() {
        let mut file = sample_file();
        let replacement = CustomModel {
            image_input: None,
            provider: "openai".into(),
            name: "qwen3-coder".into(), // same key as existing entry
            context_window: 1_000_000,  // different values
            max_output_tokens: 100_000,
            input_price_per_1m: 0.0,
            output_price_per_1m: 0.0,
            pricing_verified: false,
        };
        // Simulate upsert logic locally
        if let Some(existing) = file
            .models
            .iter_mut()
            .find(|m| m.provider == replacement.provider && m.name == replacement.name)
        {
            *existing = replacement.clone();
        } else {
            file.models.push(replacement.clone());
        }
        assert_eq!(file.models.len(), 3); // count unchanged
        let found = lookup_in_file(&file, "openai", "qwen3-coder").unwrap();
        assert_eq!(found.context_window, 1_000_000); // value replaced
    }

    #[test]
    fn upsert_appends_when_not_found() {
        let mut file = sample_file();
        let new_entry = CustomModel {
            image_input: None,
            provider: "openai".into(),
            name: "brand-new-model".into(),
            context_window: 128_000,
            max_output_tokens: 16_000,
            input_price_per_1m: 0.0,
            output_price_per_1m: 0.0,
            pricing_verified: false,
        };
        if let Some(existing) = file
            .models
            .iter_mut()
            .find(|m| m.provider == new_entry.provider && m.name == new_entry.name)
        {
            *existing = new_entry.clone();
        } else {
            file.models.push(new_entry.clone());
        }
        assert_eq!(file.models.len(), 4);
    }

    #[test]
    fn remove_scoped_to_provider() {
        let mut file = sample_file();
        let before = file.models.len();
        // Simulate scoped remove logic
        file.models
            .retain(|m| !(m.name == "qwen3-coder" && m.provider == "openai"));
        assert_eq!(before - file.models.len(), 1);
        assert!(lookup_in_file(&file, "openai", "qwen3-coder").is_none());
        // Anthropic entries unaffected
        assert!(lookup_in_file(&file, "anthropic", "claude-custom").is_some());
    }
    #[test]
    fn mutations_preserve_invalid_files_and_honor_writer_lock() {
        const CHILD: &str = "TEMM1E_CUSTOM_MODELS_STORAGE_FIXTURE";
        if std::env::var_os(CHILD).is_none() {
            let directory = tempfile::tempdir().unwrap();
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "config::custom_models::tests::mutations_preserve_invalid_files_and_honor_writer_lock", "--nocapture"])
                .env(CHILD, "1").env("TEMM1E_DATA_DIR", directory.path()).output().unwrap();
            assert!(
                child.status.success(),
                "isolated storage fixture failed: {} {}",
                String::from_utf8_lossy(&child.stdout),
                String::from_utf8_lossy(&child.stderr)
            );
            return;
        }
        let path = custom_models_path();
        let corrupt = "[[models]\nname = 'unfinished'\n";
        std::fs::write(&path, corrupt).unwrap();
        assert!(
            upsert_custom_model(sample_file().models[0].clone()).is_err(),
            "upsert erased a malformed registry"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), corrupt);
        assert!(remove_custom_model("missing", None).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), corrupt);
        save_custom_models(&sample_file()).unwrap();
        let before = std::fs::read(&path).unwrap();
        let lock =
            crate::private_file::PrivateFileLock::try_exclusive(&path.with_extension("lock"))
                .unwrap()
                .unwrap();
        assert!(upsert_custom_model(sample_file().models[0].clone()).is_err());
        assert!(remove_custom_model("qwen3-coder", Some("openai")).is_err());
        assert!(save_custom_models(&CustomModelsFile::default()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        drop(lock);
        let mut replacement = sample_file().models[0].clone();
        replacement.max_output_tokens = 1024;
        upsert_custom_model(replacement.clone()).unwrap();
        assert_eq!(
            lookup_custom_model(&replacement.provider, &replacement.name),
            Some(replacement.clone())
        );
        assert_eq!(
            load_custom_models().models.len(),
            sample_file().models.len()
        );
        assert_eq!(
            remove_custom_model(&replacement.name, Some(&replacement.provider)).unwrap(),
            1
        );
        assert_eq!(
            load_custom_models().models.len(),
            sample_file().models.len() - 1
        );
        assert!(path.with_extension("lock").exists());
        // Independent readers never take the writer lock. They must observe
        // a complete prior/new TOML document during cooperating replacements.
        save_custom_models(&sample_file()).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader_stop = stop.clone();
        let reader_path = path.clone();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let reader_barrier = barrier.clone();
        let reader = std::thread::spawn(move || {
            let mut reads = 0;
            reader_barrier.wait();
            loop {
                let visible = std::fs::read_to_string(&reader_path).unwrap();
                let parsed: CustomModelsFile =
                    toml::from_str(&visible).expect("reader observed partial TOML");
                assert_eq!(parsed.models.len(), sample_file().models.len());
                reads += 1;
                if reader_stop.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
            }
            reads
        });
        barrier.wait();
        for output in 1000..1064 {
            let mut file = sample_file();
            file.models[0].max_output_tokens = output;
            save_custom_models(&file).unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Release);
        assert!(reader.join().unwrap() > 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            let target = path.with_file_name("symlink-target.toml");
            let original = std::fs::read(&path).unwrap();
            std::fs::write(&target, &original).unwrap();
            std::fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(&target, &path).unwrap();
            upsert_custom_model(sample_file().models[0].clone()).unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), original);
            assert!(!std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink());
        }
    }
}
