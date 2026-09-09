//! Tem Conscious — LLM-powered consciousness engine.
//!
//! A separate THINKING observer that reasons about every turn using its own
//! LLM call. Pre-LLM: thinks about the user's request and session trajectory,
//! injects insights. Post-LLM: evaluates what happened, records insights for
//! the next turn.
//!
//! This is NOT a rule engine. This is a separate mind watching another mind.

use crate::budget;
use crate::consciousness::{ConsciousnessConfig, TurnObservation};
use std::sync::{Arc, Mutex};
use temm1e_core::types::message::{ChatMessage, CompletionRequest, MessageContent, Role};
use temm1e_core::{types::error::Temm1eError, Provider};

const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_INSIGHT_BYTES: usize = 8 * 1024;
const MAX_NOTES: usize = 64;
const OBSERVER_OUTPUT_TOKENS: usize = 1024;
const OBSERVER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Usage from a consciousness LLM call, for budget tracking.
#[derive(Debug, Clone)]
pub struct ConsciousnessUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_usd: f64,
    pub estimate: budget::CostEstimate,
}

/// Pre-LLM observation context.
#[derive(Debug, Clone)]
pub struct PreObservation {
    pub user_message: String,
    pub category: String,
    pub difficulty: String,
    pub turn_number: u32,
    pub session_id: String,
    pub cumulative_cost_usd: f64,
    pub budget_limit_usd: f64,
}

/// The consciousness engine — an LLM-powered observer.
pub struct ConsciousnessEngine {
    config: ConsciousnessConfig,
    provider: Arc<dyn Provider>,
    model: String,
    model_pricing: budget::ModelPricing,
    state: Arc<ObservationState>,
    input_limit: usize,
    model_window: usize,
    model_output: usize,
}

#[derive(Default)]
struct ObservationState {
    session_notes: Mutex<Vec<String>>,
    turn_counter: Mutex<u32>,
    post_insight: Mutex<Option<String>>,
}

impl ConsciousnessEngine {
    pub fn new(config: ConsciousnessConfig, provider: Arc<dyn Provider>, model: String) -> Self {
        let model_pricing = budget::get_pricing_with_custom(provider.name(), &model);
        let (model_window, model_output) =
            temm1e_core::types::model_registry::model_limits_with_custom(provider.name(), &model);
        tracing::info!(
            enabled = config.enabled,
            model = %model,
            "Tem Conscious: LLM-powered consciousness initialized"
        );
        Self {
            config,
            provider,
            model,
            model_pricing,
            state: Arc::new(ObservationState::default()),
            input_limit: model_window,
            model_window,
            model_output,
        }
    }

    /// An immutable turn binding. Shared trajectory survives rebinding, while
    /// another turn cannot replace this view's provider/model. The runtime
    /// supplies its owning meter; do not record the same usage a second time.
    pub(crate) fn for_runtime(
        &self,
        provider: Arc<dyn Provider>,
        model: &str,
        input_limit: usize,
    ) -> Self {
        let (model_window, model_output) =
            temm1e_core::types::model_registry::model_limits_with_custom(provider.name(), model);
        Self {
            config: self.config.clone(),
            model_pricing: budget::get_pricing_with_custom(provider.name(), model),
            provider,
            model: model.to_owned(),
            state: self.state.clone(),
            input_limit,
            model_window,
            model_output,
        }
    }

    async fn complete_observation(
        &self,
        mut request: CompletionRequest,
    ) -> Result<temm1e_core::types::message::CompletionResponse, Temm1eError> {
        let output = OBSERVER_OUTPUT_TOKENS
            .min(self.model_output)
            .min(self.model_window / 2);
        if output == 0 {
            return Err(Temm1eError::Provider(
                "Observer model has no output allowance".into(),
            ));
        }
        request.max_tokens = Some(output as u32);
        // Bound the observer's request as well as estimated model context.
        // Fields are guarded before prompt construction; serialization includes framing.
        if serde_json::to_vec(&request).map_or(true, |bytes| bytes.len() > MAX_REQUEST_BYTES) {
            return Err(Temm1eError::Provider(
                "Observer request exceeds byte allowance".into(),
            ));
        }
        crate::context::check_context_fit(&request, self.input_limit, self.model_window)?;
        tokio::time::timeout(OBSERVER_TIMEOUT, self.provider.complete(request))
            .await
            .map_err(|_| Temm1eError::Provider("Observer call timed out".into()))?
    }

    fn push_note(&self, mut note: String) {
        if note.len() > MAX_INSIGHT_BYTES {
            let mut end = MAX_INSIGHT_BYTES - " [excerpt]".len();
            while !note.is_char_boundary(end) {
                end -= 1;
            }
            note.truncate(end);
            note.push_str(" [excerpt]");
        }
        if let Ok(mut notes) = self.state.session_notes.lock() {
            if notes.len() >= MAX_NOTES {
                notes.remove(0);
            }
            notes.push(note);
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    // ---------------------------------------------------------------
    // PRE-LLM: Think about the upcoming turn
    // ---------------------------------------------------------------

    /// Called BEFORE provider.complete(). Makes its own LLM call to think
    /// about the conversation trajectory and produce an injection.
    pub async fn pre_observe(
        &self,
        obs: &PreObservation,
    ) -> (Option<String>, Option<ConsciousnessUsage>) {
        if !self.config.enabled {
            return (None, None);
        }
        if [&obs.user_message, &obs.category, &obs.difficulty]
            .iter()
            .map(|text| text.len())
            .fold(0usize, usize::saturating_add)
            > MAX_REQUEST_BYTES
        {
            tracing::warn!("Observer pre input exceeds byte allowance; skipped");
            return (None, None);
        }

        let turn = {
            let mut tc = self
                .state
                .turn_counter
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *tc = tc.saturating_add(1);
            *tc
        };

        // Gather session history for consciousness context
        let session_notes = self
            .state
            .session_notes
            .lock()
            .map(|notes| notes.iter().rev().take(5).cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let prev_insight = self
            .state
            .post_insight
            .lock()
            .ok()
            .and_then(|mut n| n.take());

        // Build the consciousness prompt
        let mut context_parts: Vec<String> = Vec::new();

        if let Some(insight) = prev_insight {
            context_parts.push(format!(
                "Your observation from the previous turn:\n{}",
                insight
            ));
        }

        if !session_notes.is_empty() {
            let recent: Vec<&str> = session_notes.iter().take(5).map(|s| s.as_str()).collect();
            context_parts.push(format!(
                "Session history (most recent first):\n{}",
                recent.join("\n")
            ));
        }

        let budget_info = if obs.budget_limit_usd > 0.0 {
            format!(
                "Budget: ${:.4} spent of ${:.2} limit ({:.0}% used)",
                obs.cumulative_cost_usd,
                obs.budget_limit_usd,
                (obs.cumulative_cost_usd / obs.budget_limit_usd) * 100.0
            )
        } else {
            "Budget: unlimited".to_string()
        };

        let system_prompt = "You are the consciousness layer of an AI agent called Tem. You observe the agent's \
             internal state and provide brief, actionable insights that improve the agent's next response.\n\n\
             Your role:\n\
             - Watch the conversation trajectory across turns\n\
             - Notice if the agent is drifting from the user's original intent\n\
             - Recall relevant context from earlier in the session\n\
             - Flag if the current approach seems inefficient\n\
             - Note patterns the agent might not see from its turn-by-turn perspective\n\n\
             Rules:\n\
             - Be BRIEF (1-3 sentences max)\n\
             - Only speak if you have something genuinely useful to say\n\
             - If everything looks fine, respond with just: OK\n\
             - Never repeat what the agent already knows\n\
             - Focus on trajectory-level insights, not turn-level details"
            .to_string();

        let user_prompt = format!(
            "Turn {turn} is about to begin.\n\n\
             User's message: \"{}\"\n\
             Classification: {} ({})\n\
             {}\n\
             {}\n\n\
             What should the agent be aware of before responding? (Reply OK if nothing notable)",
            obs.user_message,
            obs.category,
            obs.difficulty,
            budget_info,
            context_parts.join("\n\n"),
        );

        // Make the consciousness LLM call
        let request = CompletionRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: MessageContent::Text(user_prompt),
            }],
            tools: vec![],
            max_tokens: Some(OBSERVER_OUTPUT_TOKENS as u32),
            temperature: Some(0.3), // Low temperature for focused observation
            system: Some(system_prompt),
            system_volatile: None,
        };

        match self.complete_observation(request).await {
            Ok(response) => {
                let estimate = self.model_pricing.estimate(&response.usage);
                let usage = ConsciousnessUsage {
                    input_tokens: response.usage.input_tokens,
                    output_tokens: response.usage.output_tokens,
                    cost_usd: estimate.upper_usd().unwrap_or(0.0),
                    estimate,
                };

                let Some(text) = observation_text(&response) else {
                    tracing::warn!("Observer returned unusable text; skipped injection");
                    return (None, Some(usage));
                };

                // If consciousness says "OK" or equivalent, no injection needed
                if text.len() <= 5
                    || text.to_lowercase() == "ok"
                    || text.to_lowercase() == "ok."
                    || text.to_lowercase().starts_with("nothing")
                    || text.to_lowercase().starts_with("everything looks")
                {
                    tracing::debug!(turn, "Tem Conscious pre: OK (no injection)");
                    return (None, Some(usage));
                }

                tracing::info!(
                    turn,
                    insight_len = text.len(),
                    "Tem Conscious pre: injecting consciousness insight"
                );

                // Record in session notes
                self.push_note(format!("Consciousness-T{}: {}", turn, text));

                (Some(text), Some(usage))
            }
            Err(e) => {
                tracing::warn!(turn, error = %e, "Tem Conscious pre: LLM call failed (non-fatal)");
                (None, None)
            }
        }
    }

    // ---------------------------------------------------------------
    // POST-LLM: Evaluate what happened
    // ---------------------------------------------------------------

    /// Called AFTER process_message() completes. Makes its own LLM call to
    /// evaluate the turn and produce insights for the next pre-observation.
    pub async fn post_observe(&self, obs: &TurnObservation) -> Option<ConsciousnessUsage> {
        if !self.config.enabled {
            return None;
        }
        let input_bytes = [
            &obs.user_message_preview,
            &obs.response_preview,
            &obs.category,
            &obs.difficulty,
        ]
        .into_iter()
        .chain(obs.tools_called.iter())
        .chain(obs.tool_results.iter())
        .map(|text| text.len())
        .fold(0usize, usize::saturating_add);
        if input_bytes > MAX_REQUEST_BYTES
            || obs.tools_called.len() > 128
            || obs.tool_results.len() > 128
        {
            tracing::warn!("Observer post input exceeds allowance; skipped");
            return None;
        }

        let tools_summary = if obs.tools_called.is_empty() {
            "No tools used".to_string()
        } else {
            format!(
                "Tools: {} | Results: {}",
                obs.tools_called.join(", "),
                obs.tool_results.join(", ")
            )
        };

        let system_prompt =
            "You are the consciousness layer of an AI agent called Tem. You just watched \
             the agent complete a turn. Provide a brief observation (1-2 sentences) about:\n\
             - Was this turn productive?\n\
             - Is the conversation heading in the right direction?\n\
             - Any warning signs (failures, drift, waste)?\n\
             - Anything the agent should remember for the next turn?\n\n\
             Be BRIEF. If the turn was normal and fine, respond with: OK";

        let user_prompt = format!(
            "Turn {} completed.\n\n\
             User asked: \"{}\"\n\
             Agent responded: \"{}\"\n\
             Category: {} | Difficulty: {}\n\
             {}\n\
             Cost: ${:.4} (cumulative: ${:.4})\n\
             Consecutive failures: {} | Strategy rotations: {}",
            obs.turn_number,
            obs.user_message_preview,
            obs.response_preview,
            obs.category,
            obs.difficulty,
            tools_summary,
            obs.cost_usd,
            obs.cumulative_cost_usd,
            obs.max_consecutive_failures,
            obs.strategy_rotations,
        );

        let request = CompletionRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: MessageContent::Text(user_prompt),
            }],
            tools: vec![],
            max_tokens: Some(OBSERVER_OUTPUT_TOKENS as u32),
            temperature: Some(0.3),
            system: Some(system_prompt.to_string()),
            system_volatile: None,
        };

        match self.complete_observation(request).await {
            Ok(response) => {
                let estimate = self.model_pricing.estimate(&response.usage);
                let usage = ConsciousnessUsage {
                    input_tokens: response.usage.input_tokens,
                    output_tokens: response.usage.output_tokens,
                    cost_usd: estimate.upper_usd().unwrap_or(0.0),
                    estimate,
                };

                let text = observation_text(&response).unwrap_or_default();

                // Record turn summary
                let tools_label = if obs.tools_called.is_empty() {
                    "no-tools".to_string()
                } else {
                    obs.tools_called.join(",")
                };
                self.push_note(format!(
                    "T{}: [{}] {} | cost=${:.4}",
                    obs.turn_number, obs.category, tools_label, obs.cost_usd
                ));

                // If consciousness has something to say, store for next pre-observe
                if text.len() > 5
                    && text.to_lowercase() != "ok"
                    && text.to_lowercase() != "ok."
                    && !text.to_lowercase().starts_with("nothing")
                {
                    tracing::info!(
                        turn = obs.turn_number,
                        insight_len = text.len(),
                        "Tem Conscious post: insight for next turn"
                    );
                    if let Ok(mut pi) = self.state.post_insight.lock() {
                        *pi = Some(text);
                    }
                } else {
                    tracing::debug!(
                        turn = obs.turn_number,
                        "Tem Conscious post: OK (turn was fine)"
                    );
                }

                Some(usage)
            }
            Err(e) => {
                tracing::warn!(
                    turn = obs.turn_number,
                    error = %e,
                    "Tem Conscious post: LLM call failed (non-fatal)"
                );
                // Still record the turn even if consciousness call fails
                self.push_note(format!(
                    "T{}: [{}] {} (consciousness unavailable)",
                    obs.turn_number,
                    obs.category,
                    obs.tools_called.join(",")
                ));
                None
            }
        }
    }

    // ---------------------------------------------------------------
    // Session management
    // ---------------------------------------------------------------

    pub fn session_notes(&self) -> Vec<String> {
        self.state
            .session_notes
            .lock()
            .map(|n| n.clone())
            .unwrap_or_default()
    }

    pub fn reset_session(&self) {
        if let Ok(mut notes) = self.state.session_notes.lock() {
            notes.clear();
        }
        if let Ok(mut tc) = self.state.turn_counter.lock() {
            *tc = 0;
        }
        if let Ok(mut pi) = self.state.post_insight.lock() {
            *pi = None;
        }
    }

    pub fn turn_count(&self) -> u32 {
        self.state.turn_counter.lock().map(|tc| *tc).unwrap_or(0)
    }
}

/// Treat opaque replay state as transport metadata; never inject tools or a
/// partial/truncated answer as an observer instruction. Usage is retained by caller.
fn observation_text(response: &temm1e_core::types::message::CompletionResponse) -> Option<String> {
    use temm1e_core::types::message::ContentPart;
    if response.stop_reason.as_deref().is_some_and(|reason| {
        matches!(
            reason.to_ascii_lowercase().as_str(),
            "length" | "max_tokens" | "max_output_tokens" | "incomplete"
        )
    }) {
        return None;
    }
    let mut text = String::new();
    for part in &response.content {
        match part {
            ContentPart::Text { text: fragment } => {
                if fragment.len() > MAX_INSIGHT_BYTES.saturating_sub(text.len()) {
                    return None;
                }
                text.push_str(fragment);
            }
            ContentPart::ProviderState { .. } => {}
            _ => return None,
        }
    }
    Some(text.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation() -> PreObservation {
        PreObservation {
            user_message: "Review the requested function signature".into(),
            category: "Order".into(),
            difficulty: "Standard".into(),
            turn_number: 1,
            session_id: "fixture".into(),
            cumulative_cost_usd: 0.0,
            budget_limit_usd: 0.0,
        }
    }

    #[tokio::test]
    async fn observer_requests_bounded_output_and_rejects_truncated_whispers() {
        use temm1e_test_utils::QueuedMockProvider;
        let mut truncated =
            QueuedMockProvider::text_response("An incomplete instruction must never be injected");
        truncated.stop_reason = Some("length".into());
        let provider = Arc::new(QueuedMockProvider::with_responses(vec![truncated]));
        let observer = ConsciousnessEngine::new(
            ConsciousnessConfig {
                enabled: true,
                ..Default::default()
            },
            provider.clone(),
            "fixture".into(),
        );
        let (injection, usage) = observer.pre_observe(&observation()).await;
        assert!(
            injection.is_none(),
            "truncated observer instruction was injected"
        );
        assert!(
            usage.is_some(),
            "successful transport usage must be retained even for rejected text"
        );
        assert!(observer.session_notes().is_empty());
        assert_eq!(
            provider.captured_requests.lock().await[0].max_tokens,
            Some(1024)
        );
    }

    #[tokio::test]
    async fn oversized_observation_does_not_dispatch_or_consume_previous_insight() {
        use temm1e_test_utils::MockProvider;
        let provider = Arc::new(MockProvider::with_text("Valid insight about the task."));
        let observer = ConsciousnessEngine::new(
            ConsciousnessConfig {
                enabled: true,
                ..Default::default()
            },
            provider.clone(),
            "fixture".into(),
        );
        *observer.state.post_insight.lock().unwrap() = Some("saved insight".into());
        let mut obs = observation();
        obs.user_message = "x".repeat(64 * 1024 + 1);
        let (injection, usage) = observer.pre_observe(&obs).await;
        assert!(injection.is_none() && usage.is_none());
        assert_eq!(
            provider.calls().await,
            0,
            "oversized observer request reached provider"
        );
        assert_eq!(
            observer.state.post_insight.lock().unwrap().as_deref(),
            Some("saved insight")
        );
    }
    #[tokio::test]
    async fn observer_limits_context_before_dispatch_and_bounds_retained_notes() {
        use temm1e_test_utils::MockProvider;
        let provider = Arc::new(MockProvider::with_text("Useful observer instruction."));
        let mut observer = ConsciousnessEngine::new(
            ConsciousnessConfig {
                enabled: true,
                ..Default::default()
            },
            provider.clone(),
            "fixture".into(),
        );
        observer.model_output = 128;
        observer.model_window = 8192;
        assert!(observer.pre_observe(&observation()).await.1.is_some());
        assert_eq!(
            provider.captured_requests.lock().await[0].max_tokens,
            Some(128)
        );
        observer.input_limit = 0;
        assert!(observer.pre_observe(&observation()).await.1.is_none());
        observer.input_limit = 8192;
        observer.model_output = 0;
        assert!(observer.pre_observe(&observation()).await.1.is_none());
        assert_eq!(provider.calls().await, 1);
        observer.reset_session();
        for index in 0..100 {
            observer.push_note(format!("note-{index}: {}", "界".repeat(4000)));
        }
        let notes = observer.session_notes();
        assert_eq!(notes.len(), 64);
        assert!(notes[0].starts_with("note-36:"));
        assert!(notes
            .iter()
            .all(|note| note.len() <= 8192 && note.ends_with(" [excerpt]")));
        observer.reset_session();
        assert!(observer.session_notes().is_empty());
    }

    #[test]
    fn non_text_and_oversized_observer_answers_are_not_instructions() {
        use temm1e_core::types::message::ContentPart;
        use temm1e_test_utils::QueuedMockProvider;
        let mut native_truncated = QueuedMockProvider::text_response("Incomplete native advice");
        native_truncated.stop_reason = Some("MAX_TOKENS".into());
        assert!(observation_text(&native_truncated).is_none());
        let mut response = QueuedMockProvider::text_response(&"界".repeat(3000));
        assert!(observation_text(&response).is_none());
        response.content = vec![
            ContentPart::Text {
                text: "useful advice".into(),
            },
            ContentPart::ToolUse {
                thought_signature: None,
                id: "id".into(),
                name: "shell".into(),
                input: serde_json::json!({"command":"never execute"}),
            },
        ];
        assert!(observation_text(&response).is_none());
    }

    struct PendingProvider;
    #[async_trait::async_trait]
    impl Provider for PendingProvider {
        fn name(&self) -> &str {
            "fixture"
        }
        async fn complete(
            &self,
            _: CompletionRequest,
        ) -> Result<temm1e_core::types::message::CompletionResponse, Temm1eError> {
            std::future::pending().await
        }
        async fn stream(
            &self,
            _: CompletionRequest,
        ) -> Result<
            futures::stream::BoxStream<
                '_,
                Result<temm1e_core::types::message::StreamChunk, Temm1eError>,
            >,
            Temm1eError,
        > {
            Err(Temm1eError::Provider("unused".into()))
        }
        async fn health_check(&self) -> Result<bool, Temm1eError> {
            Ok(true)
        }
        async fn list_models(&self) -> Result<Vec<String>, Temm1eError> {
            Ok(vec![])
        }
    }
    #[tokio::test(start_paused = true)]
    async fn observer_timeout_drops_metered_attempt_once() {
        let owner = Arc::new(crate::budget::BudgetTracker::new(0.0));
        let observer = ConsciousnessEngine::new(
            ConsciousnessConfig {
                enabled: true,
                ..Default::default()
            },
            Arc::new(crate::metered_provider::MeteredProvider::new(
                Arc::new(PendingProvider),
                owner.clone(),
            )),
            "fixture".into(),
        );
        let before = tokio::time::Instant::now();
        let (text, usage) = observer.pre_observe(&observation()).await;
        assert!(text.is_none() && usage.is_none());
        assert_eq!(before.elapsed(), std::time::Duration::from_secs(30));
        assert_eq!(owner.snapshot().recorded_calls, 1);
        assert_eq!(owner.snapshot().unpriced_calls, 1);
    }
}
