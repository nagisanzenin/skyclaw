//! MLX backend — fine-tunes models on Apple Silicon via `mlx_lm.lora`.
//!
//! This backend invokes `python3 -m mlx_lm.lora --train ...` as a subprocess.
//! It only runs on macOS aarch64 (Apple Silicon) and only when `mlx-lm` is
//! installed in the system Python environment.

use super::{TrainArtifacts, TrainJob, TrainingBackend};
use async_trait::async_trait;
use temm1e_core::types::error::Temm1eError;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

pub struct MlxBackend;

#[async_trait]
impl TrainingBackend for MlxBackend {
    fn name(&self) -> &'static str {
        "mlx"
    }

    async fn is_available(&self) -> bool {
        if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            return false;
        }
        super::resolve_python("mlx_lm").await.is_some()
    }

    async fn train(&self, job: &TrainJob) -> Result<TrainArtifacts, Temm1eError> {
        // Pre-flight: dataset_dir must contain train.jsonl
        let train_jsonl = job.dataset_dir.join("train.jsonl");
        if !train_jsonl.exists() {
            return Err(Temm1eError::Tool(format!(
                "mlx: train.jsonl missing in dataset_dir {}",
                job.dataset_dir.display()
            )));
        }
        // Ensure output dir exists
        tokio::fs::create_dir_all(&job.output_dir)
            .await
            .map_err(|e| {
                Temm1eError::Tool(format!(
                    "mlx: create output_dir {}: {e}",
                    job.output_dir.display()
                ))
            })?;

        let iters = compute_iters(job);

        let python = super::resolve_python("mlx_lm").await.ok_or_else(|| {
            Temm1eError::Tool(
                "mlx: no Python 3.10+ with mlx_lm found (tried python3.13 → python3)".into(),
            )
        })?;

        let mut cmd = build_train_command(&python, job, iters);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);

        tracing::info!(
            python = %python,
            base_model = %job.base_model,
            dataset = %job.dataset_dir.display(),
            output = %job.output_dir.display(),
            iters,
            "mlx: spawning mlx_lm.lora"
        );

        let mut child = cmd
            .spawn()
            .map_err(|e| Temm1eError::Tool(format!("mlx: spawn {python}: {e}")))?;

        // Stream stdout/stderr to tracing concurrently
        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            tokio::spawn(async move {
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::info!(target: "mlx_lm.lora.stdout", "{}", line);
                }
            });
        }
        if let Some(stderr) = child.stderr.take() {
            let reader = BufReader::new(stderr);
            tokio::spawn(async move {
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::warn!(target: "mlx_lm.lora.stderr", "{}", line);
                }
            });
        }

        let status = child
            .wait()
            .await
            .map_err(|e| Temm1eError::Tool(format!("mlx: wait subprocess: {e}")))?;

        if !status.success() {
            return Err(Temm1eError::Tool(format!(
                "mlx: mlx_lm.lora exited with status {}",
                status.code().unwrap_or(-1)
            )));
        }

        // mlx_lm.lora writes adapters as either adapters.safetensors or adapters.npz.
        // Check for both, prefer safetensors.
        let safetensors = job.output_dir.join("adapters.safetensors");
        let npz = job.output_dir.join("adapters.npz");
        let adapter_path = if safetensors.exists() {
            safetensors
        } else if npz.exists() {
            npz
        } else {
            return Err(Temm1eError::Tool(format!(
                "mlx: adapter file missing in {} after successful run",
                job.output_dir.display()
            )));
        };

        // Validate non-zero size (Gate 6: adapter integrity)
        let metadata = tokio::fs::metadata(&adapter_path)
            .await
            .map_err(|e| Temm1eError::Tool(format!("mlx: stat adapter: {e}")))?;
        if metadata.len() == 0 {
            return Err(Temm1eError::Tool("mlx: adapter file is empty".to_string()));
        }

        Ok(TrainArtifacts {
            adapter_path,
            fused_model_dir: None,
            train_loss: None, // Future: parse from stdout
            eval_loss: None,
            epochs_completed: job.epochs,
        })
    }
}

/// Build the `python3.XX -m mlx_lm.lora --train ...` command for a given job.
/// Extracted as a helper so unit tests can inspect the args without spawning.
fn build_train_command(python: &str, job: &TrainJob, iters: u32) -> Command {
    let mut cmd = Command::new(python);
    cmd.arg("-m")
        .arg("mlx_lm.lora")
        .arg("--train")
        .arg("--model")
        .arg(&job.base_model)
        .arg("--data")
        .arg(&job.dataset_dir)
        .arg("--adapter-path")
        .arg(&job.output_dir)
        .arg("--fine-tune-type")
        .arg("lora")
        .arg("--batch-size")
        .arg(job.batch_size.to_string())
        .arg("--iters")
        .arg(iters.to_string());
    cmd
}

/// Compute the iteration count for mlx_lm.lora.
///
/// MLX uses iterations rather than epochs. We approximate:
///   iters ≈ epochs × ceil(dataset_size / batch_size)
///
/// Without knowing the dataset size at command-build time, we use a
/// conservative default of 200 iterations per epoch which works for
/// datasets in the 200-1000 pair range.
fn compute_iters(job: &TrainJob) -> u32 {
    let per_epoch: u32 = 200;
    (job.epochs.max(1) as u32) * per_epoch
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_job() -> TrainJob {
        TrainJob {
            base_model: "mlx-community/Llama-3.2-1B-Instruct-4bit".to_string(),
            dataset_dir: PathBuf::from("/tmp/eigen-test/data"),
            output_dir: PathBuf::from("/tmp/eigen-test/out"),
            epochs: 3,
            learning_rate: 2e-4,
            lora_r: 32,
            lora_alpha: 64,
            batch_size: 4,
            grad_accumulation: 4,
            max_seq_len: 4096,
        }
    }

    #[tokio::test]
    async fn is_available_returns_false_on_non_apple_silicon() {
        // We can't test the positive case without mlx_lm installed.
        // We CAN test that the platform check excludes non-Apple-Silicon hosts.
        let backend = MlxBackend;
        // On non-aarch64-macos, this MUST be false regardless of python availability.
        if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert!(!backend.is_available().await);
        }
    }

    #[test]
    fn train_command_construction_uses_lora_args() {
        let job = make_job();
        let iters = compute_iters(&job);
        // For epochs=3 with default per_epoch=200, iters = 600
        assert_eq!(iters, 600);

        let cmd = build_train_command("python3.12", &job, iters);
        let std_cmd = cmd.as_std();
        let args: Vec<&str> = std_cmd
            .get_args()
            .map(|os| os.to_str().unwrap_or(""))
            .collect();

        // Sanity check critical args
        assert!(args.contains(&"-m"));
        assert!(args.contains(&"mlx_lm.lora"));
        assert!(args.contains(&"--train"));
        assert!(args.contains(&"--model"));
        assert!(args.contains(&"mlx-community/Llama-3.2-1B-Instruct-4bit"));
        assert!(args.contains(&"--fine-tune-type"));
        assert!(args.contains(&"lora"));
        assert!(args.contains(&"--batch-size"));
        assert!(args.contains(&"4"));
        assert!(args.contains(&"--iters"));
        assert!(args.contains(&"600"));
    }

    #[tokio::test]
    async fn train_returns_error_when_dataset_missing() {
        let backend = MlxBackend;
        let job = make_job();
        // The dataset_dir doesn't exist; train should fail with a clear error.
        let result = backend.train(&job).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(format!("{err}").contains("train.jsonl missing"));
    }
}
