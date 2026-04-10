//! Unsloth backend — fine-tunes models on NVIDIA GPUs (and CPU/MPS) via a
//! vendored Python wrapper script that drives `unsloth.FastLanguageModel`
//! and TRL's `SFTTrainer`.
//!
//! Unsloth is a Python library, not a CLI binary, so we ship a thin wrapper
//! at `scripts/eigentune_unsloth.py` and invoke it as a subprocess.

use super::{TrainArtifacts, TrainJob, TrainingBackend};
use async_trait::async_trait;
use std::path::PathBuf;
use temm1e_core::types::error::Temm1eError;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

pub struct UnslothBackend;

#[async_trait]
impl TrainingBackend for UnslothBackend {
    fn name(&self) -> &'static str {
        "unsloth"
    }

    async fn is_available(&self) -> bool {
        // Probe for a Python that has all three required packages
        super::resolve_python("unsloth").await.is_some()
    }

    async fn train(&self, job: &TrainJob) -> Result<TrainArtifacts, Temm1eError> {
        let train_jsonl = job.dataset_dir.join("train.jsonl");
        if !train_jsonl.exists() {
            return Err(Temm1eError::Tool(format!(
                "unsloth: train.jsonl missing in dataset_dir {}",
                job.dataset_dir.display()
            )));
        }

        let python = super::resolve_python("unsloth").await.ok_or_else(|| {
            Temm1eError::Tool(
                "unsloth: no Python 3.10+ with unsloth found (tried python3.13 → python3)".into(),
            )
        })?;
        let script = locate_script("eigentune_unsloth.py")?;

        tokio::fs::create_dir_all(&job.output_dir)
            .await
            .map_err(|e| {
                Temm1eError::Tool(format!(
                    "unsloth: create output_dir {}: {e}",
                    job.output_dir.display()
                ))
            })?;

        let mut cmd = build_train_command(&python, &script, job);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);

        tracing::info!(
            python = %python,
            base_model = %job.base_model,
            dataset = %job.dataset_dir.display(),
            output = %job.output_dir.display(),
            script = %script.display(),
            "unsloth: spawning python wrapper"
        );

        let mut child = cmd
            .spawn()
            .map_err(|e| Temm1eError::Tool(format!("unsloth: spawn {python}: {e}")))?;

        // Capture stdout to parse the EIGENTUNE_RESULT line; stream stderr to tracing.
        let stdout_handle = child.stdout.take().map(|stdout| {
            tokio::spawn(async move {
                let mut buf = String::new();
                let mut reader = BufReader::new(stdout);
                let mut line = String::new();
                while let Ok(n) = reader.read_line(&mut line).await {
                    if n == 0 {
                        break;
                    }
                    tracing::info!(target: "unsloth.stdout", "{}", line.trim_end());
                    buf.push_str(&line);
                    line.clear();
                }
                buf
            })
        });

        if let Some(stderr) = child.stderr.take() {
            let reader = BufReader::new(stderr);
            tokio::spawn(async move {
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::warn!(target: "unsloth.stderr", "{}", line);
                }
            });
        }

        let status = child
            .wait()
            .await
            .map_err(|e| Temm1eError::Tool(format!("unsloth: wait subprocess: {e}")))?;

        let stdout = if let Some(handle) = stdout_handle {
            handle.await.unwrap_or_default()
        } else {
            String::new()
        };

        if !status.success() {
            return Err(Temm1eError::Tool(format!(
                "unsloth: python wrapper exited with status {}",
                status.code().unwrap_or(-1)
            )));
        }

        let summary = parse_eigentune_result(&stdout);

        let adapter_path = job.output_dir.join("adapter_model.safetensors");
        if !adapter_path.exists() {
            return Err(Temm1eError::Tool(format!(
                "unsloth: adapter file missing in {} after successful run",
                job.output_dir.display()
            )));
        }

        let metadata = tokio::fs::metadata(&adapter_path)
            .await
            .map_err(|e| Temm1eError::Tool(format!("unsloth: stat adapter: {e}")))?;
        if metadata.len() == 0 {
            return Err(Temm1eError::Tool(
                "unsloth: adapter file is empty".to_string(),
            ));
        }

        Ok(TrainArtifacts {
            adapter_path,
            fused_model_dir: None,
            train_loss: summary.train_loss,
            eval_loss: None,
            epochs_completed: summary.epochs_completed.unwrap_or(job.epochs),
        })
    }
}

#[derive(Debug, Default)]
pub(crate) struct EigentuneResult {
    pub train_loss: Option<f64>,
    pub epochs_completed: Option<i32>,
}

/// Parse the `EIGENTUNE_RESULT {json}` line printed by the Python wrapper.
/// Returns a default if the line is missing or malformed (does not error —
/// the trainer can still succeed without these metrics).
pub(crate) fn parse_eigentune_result(stdout: &str) -> EigentuneResult {
    for line in stdout.lines().rev() {
        if let Some(rest) = line.strip_prefix("EIGENTUNE_RESULT ") {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(rest) {
                return EigentuneResult {
                    train_loss: parsed.get("train_loss").and_then(|v| v.as_f64()),
                    epochs_completed: parsed
                        .get("epochs_completed")
                        .and_then(|v| v.as_i64())
                        .map(|n| n as i32),
                };
            }
        }
    }
    EigentuneResult::default()
}

/// Locate the vendored Python wrapper script. Search order:
/// 1. `$TEMM1E_SCRIPTS_DIR/eigentune_unsloth.py`
/// 2. Alongside the binary: `<exe_dir>/scripts/<name>`
/// 3. Cargo target/release layout: `<exe_dir>/../scripts/<name>`
/// 4. Workspace dev path: `<CARGO_MANIFEST_DIR>/../../scripts/<name>`
pub(crate) fn locate_script(name: &str) -> Result<PathBuf, Temm1eError> {
    if let Ok(dir) = std::env::var("TEMM1E_SCRIPTS_DIR") {
        let p = PathBuf::from(dir).join(name);
        if p.exists() {
            return Ok(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let p = parent.join("scripts").join(name);
            if p.exists() {
                return Ok(p);
            }
            let p = parent.join("..").join("scripts").join(name);
            if p.exists() {
                return Ok(p);
            }
            let p = parent.join("../..").join("scripts").join(name);
            if p.exists() {
                return Ok(p);
            }
        }
    }
    // Cargo dev workflow: relative to crate manifest dir
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let p = PathBuf::from(manifest_dir)
        .join("../..")
        .join("scripts")
        .join(name);
    if p.exists() {
        return Ok(p);
    }

    Err(Temm1eError::Tool(format!(
        "unsloth: script {name} not found (searched TEMM1E_SCRIPTS_DIR, exe-relative, manifest-relative)"
    )))
}

/// Build the `python3 <script> --model ... --data ...` command for a job.
/// Extracted as a helper so unit tests can inspect args without spawning.
fn build_train_command(python: &str, script: &PathBuf, job: &TrainJob) -> Command {
    let mut cmd = Command::new(python);
    cmd.arg(script)
        .arg("--model")
        .arg(&job.base_model)
        .arg("--data")
        .arg(&job.dataset_dir)
        .arg("--output")
        .arg(&job.output_dir)
        .arg("--epochs")
        .arg(job.epochs.to_string())
        .arg("--lr")
        .arg(job.learning_rate.to_string())
        .arg("--lora-r")
        .arg(job.lora_r.to_string())
        .arg("--lora-alpha")
        .arg(job.lora_alpha.to_string())
        .arg("--batch-size")
        .arg(job.batch_size.to_string())
        .arg("--max-seq-len")
        .arg(job.max_seq_len.to_string());
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_job() -> TrainJob {
        TrainJob {
            base_model: "unsloth/Llama-3.2-1B-Instruct-bnb-4bit".to_string(),
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
    async fn is_available_false_when_unsloth_missing() {
        let backend = UnslothBackend;
        // On a host without unsloth installed, this MUST return false.
        // We can't reliably assert true, so we just verify it doesn't panic.
        let _ = backend.is_available().await;
    }

    #[test]
    fn train_command_construction() {
        let job = make_job();
        let script = PathBuf::from("/tmp/script.py");
        let cmd = build_train_command("python3.12", &script, &job);
        let std_cmd = cmd.as_std();
        let args: Vec<&str> = std_cmd
            .get_args()
            .map(|os| os.to_str().unwrap_or(""))
            .collect();
        assert!(args.contains(&"/tmp/script.py"));
        assert!(args.contains(&"--model"));
        assert!(args.contains(&"unsloth/Llama-3.2-1B-Instruct-bnb-4bit"));
        assert!(args.contains(&"--epochs"));
        assert!(args.contains(&"3"));
        assert!(args.contains(&"--lora-r"));
        assert!(args.contains(&"32"));
    }

    #[test]
    fn parse_eigentune_result_well_formed() {
        let stdout = "Loading model...\n\
                      Training step 50/100\n\
                      EIGENTUNE_RESULT {\"train_loss\": 0.42, \"epochs_completed\": 3}\n";
        let result = parse_eigentune_result(stdout);
        assert!((result.train_loss.unwrap() - 0.42).abs() < 1e-12);
        assert_eq!(result.epochs_completed, Some(3));
    }

    #[test]
    fn parse_eigentune_result_handles_missing_summary_line() {
        let stdout = "Loading model...\nTraining step 50/100\n";
        let result = parse_eigentune_result(stdout);
        assert_eq!(result.train_loss, None);
        assert_eq!(result.epochs_completed, None);
    }

    #[test]
    fn parse_eigentune_result_handles_malformed_json() {
        let stdout = "EIGENTUNE_RESULT {not valid json\n";
        let result = parse_eigentune_result(stdout);
        assert_eq!(result.train_loss, None);
    }

    #[test]
    fn locate_script_finds_in_env_override() {
        // Create a temp script and point TEMM1E_SCRIPTS_DIR at it
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("test_script.py");
        std::fs::write(&script_path, "# test").unwrap();
        std::env::set_var("TEMM1E_SCRIPTS_DIR", dir.path());
        let found = locate_script("test_script.py").unwrap();
        assert_eq!(found, script_path);
        std::env::remove_var("TEMM1E_SCRIPTS_DIR");
    }
}
