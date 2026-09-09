//! Eigen-Tune Evaluator Orchestrator.
//!
//! Runs the eval holdout set against a freshly-trained Ollama model and
//! computes accuracy + Wilson lower bound. Writes `eval_accuracy` and
//! `eval_n` to the tier record so the next tick of the state machine can
//! decide `Evaluating → Shadowing` (passes Wilson) or `→ Collecting` (fails).
//!
//! Comparison strategy:
//! 1. For each holdout pair, extract the user message from messages_json
//! 2. Send it to the local model via Ollama's chat endpoint
//! 3. Compare the local response to the stored cloud response via
//!    `judge::embedding::cheap_equivalence_check` (free, no LLM cost)
//! 4. Tally a passed count, compute accuracy + Wilson lower bound

use crate::backends::ollama;
use crate::config::EigenTuneConfig;
use crate::judge::embedding;
use crate::stats::wilson;
use crate::store::EigenTuneStore;
use crate::types::{EigenTier, TrainingPair};
use std::sync::Arc;
use temm1e_core::types::error::Temm1eError;

pub struct EvaluatorOrchestrator {
    store: Arc<EigenTuneStore>,
    config: EigenTuneConfig,
}

#[derive(Debug, Clone)]
pub struct EvalReport {
    pub tier: EigenTier,
    pub run_id: String,
    pub n: i32,
    pub accuracy: f64,
    pub wilson_lower: f64,
    pub passed: bool,
}

impl EvaluatorOrchestrator {
    pub fn new(store: Arc<EigenTuneStore>, config: EigenTuneConfig) -> Self {
        Self { store, config }
    }

    pub async fn run(&self, tier: EigenTier, run_id: &str) -> Result<EvalReport, Temm1eError> {
        wilson::try_wilson_interval(1, 1, self.config.graduation_confidence)
            .map_err(|e| Temm1eError::Config(e.into()))?;
        if !self.config.graduation_accuracy.is_finite()
            || self.config.graduation_accuracy <= 0.0
            || self.config.graduation_accuracy > 1.0
        {
            return Err(Temm1eError::Config(
                "Graduation accuracy must be in (0,1]".into(),
            ));
        }
        // Look up the run record to get the ollama model name
        let run = self
            .store
            .get_run(run_id)
            .await?
            .ok_or_else(|| Temm1eError::Tool(format!("evaluator: run {run_id} not found")))?;
        let model_name = run.ollama_model_name.clone().ok_or_else(|| {
            Temm1eError::Tool(format!("evaluator: run {run_id} has no ollama_model_name"))
        })?;

        // Load eval holdout pairs (we filter by is_eval_holdout == true)
        let all_pairs = self.store.get_pairs_for_tier(tier.as_str(), 0.0).await?;
        let eval_pairs: Vec<TrainingPair> = all_pairs
            .into_iter()
            .filter(|p| p.is_eval_holdout)
            .collect();

        if (eval_pairs.len() as i32) < self.config.min_eval_samples {
            return Err(Temm1eError::Tool(format!(
                "evaluator: insufficient eval samples ({} < {})",
                eval_pairs.len(),
                self.config.min_eval_samples
            )));
        }

        let n = eval_pairs.len() as i32;
        let mut passed: u64 = 0;

        for pair in &eval_pairs {
            // Extract the last user message from messages_json
            let user_message = match extract_last_user_message(&pair.messages_json) {
                Some(m) => m,
                None => continue,
            };
            // Extract the cloud response text from response_json (or messages history)
            let cloud_response = extract_response_text(&pair.response_json);
            if cloud_response.is_empty() {
                continue;
            }

            // Call the local model
            let local_response = match ollama::chat(&model_name, &user_message).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        pair_id = %pair.id,
                        "evaluator: local chat failed, treating as disagreement"
                    );
                    continue;
                }
            };

            // Compare via cheap check first; if inconclusive, default to disagreement
            // (we don't have nomic-embed-text guaranteed installed). Future: embed+compare.
            let agree = embedding::cheap_equivalence_check(&local_response, &cloud_response)
                .unwrap_or(false);

            if agree {
                passed += 1;
            }
        }

        let accuracy = if n > 0 { passed as f64 / n as f64 } else { 0.0 };
        let wilson_lower =
            wilson::try_wilson_interval(passed, n as u64, self.config.graduation_confidence)
                .map_err(|e| temm1e_core::types::error::Temm1eError::Config(e.into()))?
                .0;

        // Write back to the tier record
        let mut tier_record = self.store.get_tier(tier.as_str()).await?;
        tier_record.eval_accuracy = Some(accuracy);
        tier_record.eval_n = Some(n);
        self.store.update_tier(&tier_record).await?;

        let passed_gate = self.config.graduation_accuracy > 0.0
            && self.config.graduation_accuracy <= 1.0
            && wilson_lower >= self.config.graduation_accuracy;
        tracing::info!(
            tier = %tier.as_str(),
            run_id,
            n,
            accuracy,
            wilson_lower,
            threshold = self.config.graduation_accuracy,
            passed = passed_gate,
            "evaluator: complete"
        );

        Ok(EvalReport {
            tier,
            run_id: run_id.to_string(),
            n,
            accuracy,
            wilson_lower,
            passed: passed_gate,
        })
    }
}

/// Extract the last user message from a ChatML messages_json string.
fn extract_last_user_message(messages_json: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(messages_json).ok()?;
    let messages = parsed.as_array()?;
    for msg in messages.iter().rev() {
        if msg.get("role").and_then(|v| v.as_str()) == Some("user") {
            if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
                return Some(content.to_string());
            }
        }
    }
    None
}

/// Extract the assistant text from a response_json string.
/// Handles both `{"role":"assistant","content":"..."}` and `{"content":"..."}`.
fn extract_response_text(response_json: &str) -> String {
    let parsed: serde_json::Value = match serde_json::from_str(response_json) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    if let Some(s) = parsed.get("content").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(content) = parsed.get("content").and_then(|v| v.as_array()) {
        // Vec<ContentPart>
        let mut out = String::new();
        for part in content {
            if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                out.push_str(t);
                out.push('\n');
            }
        }
        return out;
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_pair(id: &str) -> TrainingPair {
        TrainingPair {
            id: id.to_string(),
            conversation_id: "conv-1".to_string(),
            turn: 1,
            created_at: Utc::now(),
            messages_json:
                r#"[{"role":"user","content":"What is 2+2?"},{"role":"assistant","content":"4"}]"#
                    .to_string(),
            system_prompt: None,
            tools_json: None,
            response_json: r#"{"role":"assistant","content":"4"}"#.to_string(),
            source_model: "claude-sonnet-4-20250514".to_string(),
            source_provider: "anthropic".to_string(),
            complexity: EigenTier::Simple,
            domain_category: Some("math".to_string()),
            quality_alpha: 2.0,
            quality_beta: 2.0,
            quality_score: Some(0.9),
            user_continued: None,
            user_retried: None,
            tool_success: None,
            response_error: None,
            tokens_in: Some(5),
            tokens_out: Some(1),
            cost_usd: Some(0.001),
            dataset_version: None,
            is_eval_holdout: true,
        }
    }

    #[tokio::test]
    async fn run_fails_when_eval_pairs_below_min() {
        let store = Arc::new(EigenTuneStore::new("sqlite::memory:").await.unwrap());
        // Save 3 eval pairs but min_eval_samples = 30
        for i in 0..3 {
            let mut p = make_pair(&format!("p{i}"));
            p.id = format!("p{i}");
            store.save_pair(&p).await.unwrap();
        }
        // Need to register a run for the lookup to succeed
        let run = crate::types::TrainingRun {
            id: "test-run".to_string(),
            started_at: Utc::now(),
            completed_at: None,
            status: crate::types::TrainingRunStatus::Completed,
            base_model: "test".to_string(),
            backend: "mlx".to_string(),
            method: "lora".to_string(),
            dataset_version: 1,
            pair_count: 3,
            general_mix_pct: 0.0,
            output_model_path: None,
            gguf_path: None,
            ollama_model_name: Some("eigentune-test".to_string()),
            train_loss: None,
            eval_loss: None,
            epochs: Some(3),
            learning_rate: Some(2e-4),
            error_message: None,
        };
        store.save_run(&run).await.unwrap();

        let cfg = EigenTuneConfig::default(); // min_eval_samples = 30
        let evaluator = EvaluatorOrchestrator::new(store, cfg);
        let result = evaluator.run(EigenTier::Simple, "test-run").await;
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("insufficient eval samples"));
    }

    #[tokio::test]
    async fn run_fails_when_run_not_found() {
        let store = Arc::new(EigenTuneStore::new("sqlite::memory:").await.unwrap());
        let cfg = EigenTuneConfig::default();
        let evaluator = EvaluatorOrchestrator::new(store, cfg);
        let result = evaluator.run(EigenTier::Simple, "missing-run").await;
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("not found"));
    }

    #[test]
    fn extract_last_user_message_finds_user() {
        let json = r#"[
            {"role":"system","content":"you are helpful"},
            {"role":"user","content":"first"},
            {"role":"assistant","content":"reply"},
            {"role":"user","content":"second"}
        ]"#;
        let extracted = extract_last_user_message(json);
        assert_eq!(extracted, Some("second".to_string()));
    }

    #[test]
    fn extract_last_user_message_handles_no_user() {
        let json = r#"[{"role":"system","content":"only system"}]"#;
        assert_eq!(extract_last_user_message(json), None);
    }

    #[test]
    fn extract_response_text_handles_string_content() {
        let json = r#"{"role":"assistant","content":"4"}"#;
        assert_eq!(extract_response_text(json), "4");
    }
}
