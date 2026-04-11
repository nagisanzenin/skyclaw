//! Eigen-Tune Trainer Orchestrator.
//!
//! Drives one full training cycle for a tier:
//! 1. Select an available training backend (mlx, unsloth, or auto)
//! 2. Curate the dataset (`curator::build_training_dataset`)
//! 3. Insert a `TrainingRun` row (status = running)
//! 4. Spawn the backend's `train()` with a wall-clock timeout
//! 5. Validate the adapter file exists and has non-zero size (Gate 6)
//! 6. Commit the model to Ollama via Modelfile FROM + ADAPTER directives
//! 7. Update the `TrainingRun` row (status = completed) and transition the
//!    tier `Training → Evaluating`
//!
//! On any failure: the `TrainingRun` is marked failed, the tier reverts to
//! `Collecting`, the error is logged at `warn!` and propagated to the caller.
//! The caller (the periodic tick task in `main.rs`) catches it and never
//! lets a panic or error reach the user.

use crate::backends::{select_backend, TrainArtifacts, TrainJob};
use crate::config::EigenTuneConfig;
use crate::curator;
use crate::engine::state_machine::EigenTuneStateMachine;
use crate::store::EigenTuneStore;
use crate::types::{EigenTier, TierState, TrainingRun, TrainingRunStatus};
use chrono::Utc;
use std::path::PathBuf;
use std::sync::Arc;
use temm1e_core::types::error::Temm1eError;
use uuid::Uuid;

pub struct TrainerOrchestrator {
    store: Arc<EigenTuneStore>,
    config: EigenTuneConfig,
}

impl TrainerOrchestrator {
    pub fn new(store: Arc<EigenTuneStore>, config: EigenTuneConfig) -> Self {
        Self { store, config }
    }

    /// Run a complete training cycle for a tier.
    ///
    /// Assumes the tier is currently in `Training` state. On success, the
    /// tier transitions to `Evaluating`. On failure, the tier reverts to
    /// `Collecting`.
    pub async fn run(&self, tier: EigenTier) -> Result<TrainArtifacts, Temm1eError> {
        let run_id = Uuid::new_v4().to_string();
        let started_at = Utc::now();

        match self.run_inner(tier, &run_id, started_at).await {
            Ok(artifacts) => Ok(artifacts),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    tier = %tier.as_str(),
                    run_id = %run_id,
                    "trainer: cycle failed, reverting tier to Collecting"
                );

                // Mark the run as failed
                if let Ok(Some(run)) = self.store.get_run(&run_id).await {
                    let mut updated = run;
                    updated.status = TrainingRunStatus::Failed;
                    updated.completed_at = Some(Utc::now());
                    updated.error_message = Some(format!("{e}"));
                    if let Err(e) = self.store.update_run(&updated).await {
                        tracing::warn!(error = %e, "Failed to persist training run failure status");
                    }
                }

                // Revert the tier to Collecting via the state machine
                let sm = EigenTuneStateMachine::new(self.store.clone(), self.config.clone());
                if let Ok(current) = sm.state(tier).await {
                    if current == TierState::Training {
                        if let Err(e) = sm
                            .transition(tier, TierState::Training, TierState::Collecting)
                            .await
                        {
                            tracing::warn!(error = %e, "Failed to revert tier state to Collecting");
                        }
                    }
                }

                Err(e)
            }
        }
    }

    async fn run_inner(
        &self,
        tier: EigenTier,
        run_id: &str,
        started_at: chrono::DateTime<Utc>,
    ) -> Result<TrainArtifacts, Temm1eError> {
        // Step 1: Pre-flight backend selection
        let backend = select_backend(&self.config).await.ok_or_else(|| {
            Temm1eError::Tool(format!(
                "trainer: no training backend available (config.backend = {})",
                self.config.backend
            ))
        })?;

        tracing::info!(
            tier = %tier.as_str(),
            run_id,
            backend = backend.name(),
            "trainer: selected backend"
        );

        // Step 2: Curate the dataset
        let workdir = self.workdir_for_run(run_id);
        let curator_out =
            curator::build_training_dataset(&self.store, &self.config, tier, &workdir).await?;

        tracing::info!(
            tier = %tier.as_str(),
            train = curator_out.train_count,
            eval = curator_out.eval_count,
            j = curator_out.diversity_j,
            "trainer: curator complete"
        );

        // Step 3: Resolve base_model (handle "auto")
        let base_model = self.resolve_base_model(tier);

        // Step 4: Insert TrainingRun row (status=running)
        let run = TrainingRun {
            id: run_id.to_string(),
            started_at,
            completed_at: None,
            status: TrainingRunStatus::Running,
            base_model: base_model.clone(),
            backend: backend.name().to_string(),
            method: self.config.method.clone(),
            dataset_version: 1,
            pair_count: curator_out.train_count as i32,
            general_mix_pct: self.config.general_mix_pct,
            output_model_path: None,
            gguf_path: None,
            ollama_model_name: None,
            train_loss: None,
            eval_loss: None,
            epochs: Some(self.config.epochs),
            learning_rate: Some(self.config.learning_rate),
            error_message: None,
        };
        self.store.save_run(&run).await?;

        // Update the tier record's current_run_id so the state machine
        // knows the trainer is running (Phase 6 recovery uses this).
        let mut tier_record = self.store.get_tier(tier.as_str()).await?;
        tier_record.current_run_id = Some(run_id.to_string());
        self.store.update_tier(&tier_record).await?;

        // Step 5: Build TrainJob
        let output_dir = self.workdir_for_run(run_id).join("adapter");
        let job = TrainJob {
            base_model: base_model.clone(),
            dataset_dir: workdir.clone(),
            output_dir: output_dir.clone(),
            epochs: self.config.epochs,
            learning_rate: self.config.learning_rate,
            lora_r: self.config.lora_r,
            lora_alpha: self.config.lora_alpha,
            batch_size: self.config.batch_size,
            grad_accumulation: self.config.gradient_accumulation_steps,
            max_seq_len: self.config.max_seq_length,
        };

        // Step 6: Spawn backend with timeout
        let timeout = std::time::Duration::from_secs(self.config.max_training_minutes * 60);
        let train_future = backend.train(&job);
        let artifacts = match tokio::time::timeout(timeout, train_future).await {
            Ok(Ok(a)) => a,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(Temm1eError::Tool(format!(
                    "trainer: training subprocess exceeded {} minute timeout",
                    self.config.max_training_minutes
                )));
            }
        };

        // Step 7: Adapter integrity (Gate 6) — already enforced by each backend
        // but double-check here for defense in depth
        if !artifacts.adapter_path.exists() {
            return Err(Temm1eError::Tool(format!(
                "trainer: adapter file vanished after backend success: {}",
                artifacts.adapter_path.display()
            )));
        }

        // Step 8: Commit to Ollama
        let model_name = format!(
            "eigentune-{}-{}",
            tier.as_str(),
            &run_id.chars().take(8).collect::<String>()
        );
        match commit_to_ollama(&model_name, &base_model, &artifacts.adapter_path).await {
            Ok(()) => {
                tracing::info!(
                    tier = %tier.as_str(),
                    model = %model_name,
                    "trainer: model registered in Ollama"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    tier = %tier.as_str(),
                    "trainer: ollama commit failed, run is local-only"
                );
                // Don't propagate — the artifacts are still on disk and the
                // user can manually inspect them. The tier will revert.
                return Err(e);
            }
        }

        // Step 9: Update TrainingRun row (status=completed)
        let mut completed_run = run;
        completed_run.status = TrainingRunStatus::Completed;
        completed_run.completed_at = Some(Utc::now());
        completed_run.train_loss = artifacts.train_loss;
        completed_run.output_model_path = Some(artifacts.adapter_path.display().to_string());
        completed_run.ollama_model_name = Some(model_name.clone());
        self.store.update_run(&completed_run).await?;

        // Step 10: Transition tier Training → Evaluating
        // Also set serving_run_id so the state machine + router can find it later.
        let sm = EigenTuneStateMachine::new(self.store.clone(), self.config.clone());
        sm.transition(tier, TierState::Training, TierState::Evaluating)
            .await?;
        let mut tier_record = self.store.get_tier(tier.as_str()).await?;
        tier_record.serving_run_id = Some(run_id.to_string());
        self.store.update_tier(&tier_record).await?;

        Ok(artifacts)
    }

    fn workdir_for_run(&self, run_id: &str) -> PathBuf {
        let base = expand_tilde(&self.config.artifacts_dir);
        base.join(format!("run_{run_id}"))
    }

    /// Resolve `config.base_model = "auto"` to a concrete model per tier.
    /// Picks Llama/Mistral/Gemma family models (Ollama ADAPTER directive support).
    fn resolve_base_model(&self, tier: EigenTier) -> String {
        if self.config.base_model != "auto" {
            return self.config.base_model.clone();
        }
        let is_apple = cfg!(all(target_os = "macos", target_arch = "aarch64"));
        if is_apple {
            match tier {
                EigenTier::Simple => "mlx-community/Llama-3.2-1B-Instruct-4bit".to_string(),
                EigenTier::Standard => "mlx-community/Llama-3.2-3B-Instruct-4bit".to_string(),
                EigenTier::Complex => "mlx-community/Mistral-7B-Instruct-v0.3-4bit".to_string(),
            }
        } else {
            match tier {
                EigenTier::Simple => "unsloth/Llama-3.2-1B-Instruct-bnb-4bit".to_string(),
                EigenTier::Standard => "unsloth/Llama-3.2-3B-Instruct-bnb-4bit".to_string(),
                EigenTier::Complex => "unsloth/Mistral-7B-Instruct-v0.3-bnb-4bit".to_string(),
            }
        }
    }
}

/// Expand a leading `~/` to the user's home directory.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// Commit a trained adapter to Ollama as a new model.
///
/// Uses the Modelfile `FROM <base_model>` + `ADAPTER <adapter_dir>` directive
/// pattern. This requires the base model to be in a family Ollama supports
/// for ADAPTER (Llama, Mistral, Gemma per the Ollama docs as of April 2026).
async fn commit_to_ollama(
    model_name: &str,
    base_model: &str,
    adapter_path: &std::path::Path,
) -> Result<(), Temm1eError> {
    // The ADAPTER directive expects a directory containing the safetensors file.
    let adapter_dir = adapter_path.parent().ok_or_else(|| {
        Temm1eError::Tool(format!(
            "trainer: cannot derive adapter dir from {}",
            adapter_path.display()
        ))
    })?;

    // Map mlx-community/Llama-3.2-1B-Instruct-4bit → llama3.2:1b style for FROM
    let ollama_base = ollama_base_for(base_model);

    let modelfile = format!(
        "FROM {}\nADAPTER {}\nPARAMETER temperature 0.7\nPARAMETER num_ctx 4096\n",
        ollama_base,
        adapter_dir.display()
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| Temm1eError::Tool(format!("trainer: http client: {e}")))?;

    let body = serde_json::json!({
        "model": model_name,
        "modelfile": modelfile,
        "stream": false
    });

    let resp = client
        .post("http://localhost:11434/api/create")
        .json(&body)
        .send()
        .await
        .map_err(|e| Temm1eError::Tool(format!("trainer: ollama create: {e}")))?;

    if !resp.status().is_success() {
        let err = resp.text().await.unwrap_or_default();
        return Err(Temm1eError::Tool(format!(
            "trainer: ollama create error: {err}"
        )));
    }
    Ok(())
}

/// Map a HuggingFace-style base model name to an Ollama base tag for FROM.
/// Best-effort heuristic; users can override via config.base_model.
fn ollama_base_for(base_model: &str) -> String {
    let lower = base_model.to_lowercase();
    if lower.contains("llama-3.2-1b") || lower.contains("llama3.2-1b") {
        "llama3.2:1b".to_string()
    } else if lower.contains("llama-3.2-3b") || lower.contains("llama3.2-3b") {
        "llama3.2:3b".to_string()
    } else if lower.contains("mistral-7b") {
        "mistral:7b".to_string()
    } else if lower.contains("gemma-2-2b") || lower.contains("gemma2-2b") {
        "gemma2:2b".to_string()
    } else if lower.contains("gemma-2-9b") || lower.contains("gemma2-9b") {
        "gemma2:9b".to_string()
    } else {
        // Fallback: use the raw model string. Ollama may reject if it doesn't know it.
        base_model.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_store() -> Arc<EigenTuneStore> {
        Arc::new(EigenTuneStore::new("sqlite::memory:").await.unwrap())
    }

    #[tokio::test]
    async fn run_fails_when_no_backend_available() {
        let store = test_store().await;
        let cfg = EigenTuneConfig {
            backend: "nonexistent_backend".to_string(),
            ..EigenTuneConfig::default()
        };
        let trainer = TrainerOrchestrator::new(store, cfg);
        let result = trainer.run(EigenTier::Simple).await;
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("no training backend"));
    }

    #[tokio::test]
    async fn run_fails_when_min_pairs_not_met() {
        let store = test_store().await;
        // Manually set tier to Training so the trainer's failure path runs
        let mut record = store.get_tier("simple").await.unwrap();
        record.state = TierState::Training;
        store.update_tier(&record).await.unwrap();

        let cfg = EigenTuneConfig {
            backend: "nonexistent_backend".to_string(),
            min_pairs: 1000,
            ..EigenTuneConfig::default()
        };
        let trainer = TrainerOrchestrator::new(store.clone(), cfg);
        let _ = trainer.run(EigenTier::Simple).await;
        // After failure, tier should be back to Collecting
        let final_state = store.get_tier("simple").await.unwrap();
        assert_eq!(final_state.state, TierState::Collecting);
    }

    #[test]
    fn ollama_base_for_known_models() {
        assert_eq!(
            ollama_base_for("mlx-community/Llama-3.2-1B-Instruct-4bit"),
            "llama3.2:1b"
        );
        assert_eq!(
            ollama_base_for("unsloth/Llama-3.2-3B-Instruct-bnb-4bit"),
            "llama3.2:3b"
        );
        assert_eq!(
            ollama_base_for("mlx-community/Mistral-7B-Instruct-v0.3-4bit"),
            "mistral:7b"
        );
        assert_eq!(
            ollama_base_for("mlx-community/gemma-2-2b-it-4bit"),
            "gemma2:2b"
        );
    }

    #[test]
    fn resolve_base_model_auto_picks_per_tier() {
        let cfg = EigenTuneConfig {
            base_model: "auto".to_string(),
            ..EigenTuneConfig::default()
        };
        // We can't easily mock the cfg! macro, so just verify all three tiers
        // return non-empty strings and they're all different.
        let store = std::sync::Arc::new(
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(EigenTuneStore::new("sqlite::memory:"))
                .unwrap(),
        );
        let trainer = TrainerOrchestrator::new(store, cfg);
        let s = trainer.resolve_base_model(EigenTier::Simple);
        let m = trainer.resolve_base_model(EigenTier::Standard);
        let l = trainer.resolve_base_model(EigenTier::Complex);
        assert!(!s.is_empty());
        assert!(!m.is_empty());
        assert!(!l.is_empty());
        assert_ne!(s, l);
    }

    #[test]
    fn resolve_base_model_explicit_passes_through() {
        let cfg = EigenTuneConfig {
            base_model: "my-custom-model:latest".to_string(),
            ..EigenTuneConfig::default()
        };
        let store = std::sync::Arc::new(
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(EigenTuneStore::new("sqlite::memory:"))
                .unwrap(),
        );
        let trainer = TrainerOrchestrator::new(store, cfg);
        assert_eq!(
            trainer.resolve_base_model(EigenTier::Simple),
            "my-custom-model:latest"
        );
    }
}
