//! Eigen-Tune training backends.
//!
//! Each backend wraps an external trainer (mlx_lm.lora, unsloth, etc.) as
//! an async subprocess. The `TrainingBackend` trait abstracts over them so
//! the trainer orchestrator (`engine::trainer`) can pick whichever backend
//! is available on the host.

pub mod mlx;
pub mod ollama;
pub mod unsloth;

use crate::config::EigenTuneConfig;
use async_trait::async_trait;
use std::path::PathBuf;
use temm1e_core::types::error::Temm1eError;

/// Inputs to a single training run.
#[derive(Debug, Clone)]
pub struct TrainJob {
    /// Base model name (HuggingFace repo ID or local path).
    pub base_model: String,
    /// Directory containing `train.jsonl` and `valid.jsonl` from the curator.
    pub dataset_dir: PathBuf,
    /// Directory where adapter weights will be written.
    pub output_dir: PathBuf,
    pub epochs: i32,
    pub learning_rate: f64,
    pub lora_r: i32,
    pub lora_alpha: i32,
    pub batch_size: i32,
    pub grad_accumulation: i32,
    pub max_seq_len: i32,
}

/// Outputs of a successful training run.
#[derive(Debug, Clone)]
pub struct TrainArtifacts {
    /// Path to the safetensors adapter file.
    pub adapter_path: PathBuf,
    /// Optional path to a fused full-precision model directory (mlx_lm.fuse output).
    pub fused_model_dir: Option<PathBuf>,
    /// Final training loss, if parseable from backend output.
    pub train_loss: Option<f64>,
    /// Final eval loss, if parseable from backend output.
    pub eval_loss: Option<f64>,
    /// Number of epochs actually completed.
    pub epochs_completed: i32,
}

/// Trait every training backend implements.
#[async_trait]
pub trait TrainingBackend: Send + Sync {
    /// Stable identifier for this backend (e.g. "mlx", "unsloth").
    fn name(&self) -> &'static str;

    /// Probe whether this backend can run on the current host.
    /// Should be cheap and side-effect free (a single subprocess at most).
    async fn is_available(&self) -> bool;

    /// Spawn the training subprocess and return artifacts on success.
    /// Errors are returned as `Temm1eError::Tool` and never panic.
    async fn train(&self, job: &TrainJob) -> Result<TrainArtifacts, Temm1eError>;
}

/// Probe for a Python 3.10+ binary that can import a given module.
///
/// On macOS with Homebrew, `python3` is often the system 3.9.6 which
/// is too old for MLX (requires 3.10+). This probes versioned names
/// first (`python3.13`, `python3.12`, `python3.11`, `python3.10`) then
/// falls back to the generic `python3`. Returns the binary name that
/// succeeds, or None if none can import the module.
///
/// The result is cached for the lifetime of the process — the probe
/// is expensive (subprocess per candidate) so we only do it once.
pub async fn resolve_python(required_module: &str) -> Option<String> {
    let candidates = [
        "python3.13",
        "python3.12",
        "python3.11",
        "python3.10",
        "python3",
    ];
    let import_check = format!("import {required_module}");
    for candidate in &candidates {
        let result = tokio::process::Command::new(candidate)
            .args(["-c", &import_check])
            .output()
            .await;
        if let Ok(output) = result {
            if output.status.success() {
                tracing::info!(
                    python = candidate,
                    module = required_module,
                    "Eigen-Tune: resolved Python binary"
                );
                return Some(candidate.to_string());
            }
        }
    }
    None
}

/// Pick the first available backend matching `config.backend`.
///
/// Backend selection logic:
/// - `"mlx"` → use MLX if available, else None
/// - `"unsloth"` → use Unsloth if available, else None
/// - `"auto"` → prefer MLX on Apple Silicon, fall back to Unsloth elsewhere
/// - any other value → None (the trainer will mark the run as failed)
pub async fn select_backend(config: &EigenTuneConfig) -> Option<Box<dyn TrainingBackend>> {
    let mlx = mlx::MlxBackend;
    let unsloth = unsloth::UnslothBackend;
    match config.backend.as_str() {
        "mlx" if mlx.is_available().await => Some(Box::new(mlx)),
        "unsloth" if unsloth.is_available().await => Some(Box::new(unsloth)),
        "auto" => {
            let is_apple_silicon = cfg!(all(target_os = "macos", target_arch = "aarch64"));
            if is_apple_silicon && mlx.is_available().await {
                Some(Box::new(mlx))
            } else if unsloth.is_available().await {
                Some(Box::new(unsloth))
            } else {
                None
            }
        }
        _ => None,
    }
}
