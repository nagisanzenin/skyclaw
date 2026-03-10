//! Tiered Model Routing — classify tasks by complexity and route to
//! appropriate models. Simple tasks (file reads, status checks) use a
//! fast/cheap model; complex tasks (architecture, debugging) use the
//! most capable model. Classification is entirely rule-based (no LLM call).

use serde::{Deserialize, Serialize};
use skyclaw_core::types::message::{ChatMessage, ContentPart, MessageContent, Role};
use skyclaw_core::types::optimization::ExecutionProfile;
use tracing::{debug, info};

// ── Read-only / simple tool names ────────────────────────────────────────

/// Tools considered read-only / low-complexity. If a task uses only these
/// tools and meets other simplicity heuristics, it is classified as Simple.
const READ_ONLY_TOOLS: &[&str] = &[
    "file_read",
    "file_list",
    "check_messages",
    "git_status",
    "git_log",
    "git_diff",
    "http_get",
    "list_directory",
    "read_file",
];

/// Keywords in task descriptions that indicate a complex task.
const COMPLEX_KEYWORDS: &[&str] = &[
    "architecture",
    "architect",
    "debug",
    "debugging",
    "refactor",
    "refactoring",
    "design",
    "redesign",
    "migrate",
    "migration",
    "optimize",
    "optimization",
    "security audit",
    "performance",
    "investigate",
    "root cause",
    "rewrite",
];

/// Greeting/farewell patterns that indicate a trivial message.
const TRIVIAL_PATTERNS: &[&str] = &[
    "hi",
    "hello",
    "hey",
    "thanks",
    "thank you",
    "bye",
    "goodbye",
    "good morning",
    "good evening",
    "good night",
    "ok",
    "okay",
    "got it",
    "sure",
    "yes",
    "no",
    "yep",
    "nope",
    "cool",
    "nice",
    "great",
    "awesome",
    "perfect",
    "understood",
    "\u{1f44d}",
    "\u{1f64f}",
];

/// Action verbs that indicate a non-trivial task.
const ACTION_VERBS: &[&str] = &[
    "find",
    "create",
    "run",
    "deploy",
    "read",
    "write",
    "search",
    "build",
    "fix",
    "update",
    "delete",
    "install",
    "configure",
    "setup",
    "check",
    "test",
    "compile",
    "execute",
    "fetch",
    "download",
    "upload",
    "send",
    "list",
    "show",
    "display",
    "open",
    "close",
    "start",
    "stop",
    "restart",
    "analyze",
    "explain",
    "help me",
    "can you",
    "please",
];

/// Maximum message length in chars for a trivial classification.
const TRIVIAL_MAX_LEN: usize = 50;

/// Maximum task description length (in chars) for a task to be considered Simple.
const SIMPLE_DESCRIPTION_MAX_LEN: usize = 100;

/// History length threshold above which a conversation is considered complex.
const COMPLEX_HISTORY_THRESHOLD: usize = 10;

// ── Enums ────────────────────────────────────────────────────────────────

/// Task complexity level as determined by rule-based classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskComplexity {
    /// Trivial: pure conversation, no tools needed.
    /// Greetings, thanks, one-word responses, simple questions with no action verbs.
    Trivial,
    /// Simple: single read-only tool, short description, shallow history.
    Simple,
    /// Standard: the default bucket for everything that is neither
    /// clearly simple nor clearly complex.
    Standard,
    /// Complex: architecture/debug/refactor tasks, deep history, compound
    /// tool usage, or DONE criteria present.
    Complex,
}

impl TaskComplexity {
    /// Get the execution profile for this complexity level.
    pub fn execution_profile(&self) -> ExecutionProfile {
        match self {
            TaskComplexity::Trivial => ExecutionProfile::trivial(),
            TaskComplexity::Simple => ExecutionProfile::simple(),
            TaskComplexity::Standard => ExecutionProfile::standard(),
            TaskComplexity::Complex => ExecutionProfile::complex(),
        }
    }
}

/// Model tier that maps to a configured model name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelTier {
    /// Fastest / cheapest model for trivial tasks.
    Fast,
    /// Default model — the primary workhorse.
    Primary,
    /// Most capable model for hard tasks.
    Premium,
}

// ── User override prefixes ───────────────────────────────────────────────

/// Prefix that forces the Fast tier.
const FORCE_FAST_PREFIX: &str = "!fast";

/// Prefix that forces the Premium tier.
const FORCE_BEST_PREFIX: &str = "!best";

// ── Configuration ────────────────────────────────────────────────────────

/// Configuration for the tiered model router.
///
/// Can be embedded in the agent section of `skyclaw.toml` or supplied
/// programmatically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRouterConfig {
    /// Whether tiered routing is enabled. When `false`, all requests use
    /// the primary model regardless of complexity.
    #[serde(default)]
    pub enabled: bool,

    /// Model name for the Fast tier (e.g. `"claude-haiku-4-5-20251001"`).
    /// If `None`, Fast-tier tasks fall back to the primary model.
    #[serde(default)]
    pub fast_model: Option<String>,

    /// Model name for the Primary tier. This is the default model from
    /// the provider config and must always be set.
    #[serde(default = "default_primary_model")]
    pub primary_model: String,

    /// Model name for the Premium tier (e.g. `"claude-opus-4-6"`).
    /// If `None`, Premium-tier tasks fall back to the primary model.
    #[serde(default)]
    pub premium_model: Option<String>,
}

fn default_primary_model() -> String {
    "claude-sonnet-4-6".to_string()
}

impl Default for ModelRouterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            fast_model: None,
            primary_model: default_primary_model(),
            premium_model: None,
        }
    }
}

// ── Router ───────────────────────────────────────────────────────────────

/// Rule-based model router. Classifies task complexity and selects the
/// appropriate model tier without making any LLM calls.
#[derive(Debug, Clone)]
pub struct ModelRouter {
    config: ModelRouterConfig,
}

impl ModelRouter {
    /// Create a new `ModelRouter` from configuration.
    pub fn new(config: ModelRouterConfig) -> Self {
        Self { config }
    }

    /// Whether the router is enabled. When disabled, [`get_model_name`]
    /// always returns the primary model.
    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    /// Convenience method: classify, select tier, and return the model name
    /// in one call.
    ///
    /// * `history` — conversation history so far.
    /// * `tool_names` — names of tools used (or requested) in this turn.
    /// * `task_description` — the user's message text.
    /// * `is_verification` — whether this is a verification step (Phase 1.1).
    pub fn route(
        &self,
        history: &[ChatMessage],
        tool_names: &[&str],
        task_description: &str,
        is_verification: bool,
    ) -> &str {
        if !self.config.enabled {
            return &self.config.primary_model;
        }

        // Verification steps always use Primary or Premium.
        if is_verification {
            let tier = if self.config.premium_model.is_some() {
                ModelTier::Premium
            } else {
                ModelTier::Primary
            };
            let model = self.get_model_name(tier);
            debug!(
                tier = ?tier,
                model = %model,
                "Verification step — using elevated tier"
            );
            return model;
        }

        // Check for user overrides.
        if let Some(forced) = Self::detect_user_override(task_description) {
            let model = self.get_model_name(forced);
            info!(
                tier = ?forced,
                model = %model,
                "User override detected"
            );
            return model;
        }

        let complexity = self.classify_complexity(history, tool_names, task_description);
        let tier = Self::select_tier(complexity);
        let model = self.get_model_name(tier);

        info!(
            complexity = ?complexity,
            tier = ?tier,
            model = %model,
            "Routed task to model"
        );

        model
    }

    /// Classify the complexity of a task based on conversation history,
    /// tool usage, and task description. Entirely rule-based.
    pub fn classify_complexity(
        &self,
        history: &[ChatMessage],
        tool_names: &[&str],
        task_description: &str,
    ) -> TaskComplexity {
        let desc_lower = task_description.to_lowercase();

        // ── Trivial signals ─────────────────────────────────────────
        let short_msg = task_description.len() <= TRIVIAL_MAX_LEN;
        let no_tools = tool_names.is_empty();
        let shallow_history = history.len() <= 3;
        let has_action_verb = ACTION_VERBS.iter().any(|v| desc_lower.contains(v));
        let is_greeting = TRIVIAL_PATTERNS
            .iter()
            .any(|p| desc_lower.trim() == *p || desc_lower.starts_with(p));
        let has_path_or_url =
            desc_lower.contains('/') || desc_lower.contains("http") || desc_lower.contains("```");

        if short_msg
            && no_tools
            && (shallow_history || is_greeting)
            && !has_action_verb
            && !has_path_or_url
        {
            return TaskComplexity::Trivial;
        }

        // ── Complex signals ──────────────────────────────────────────

        // 1. Keywords indicating complex work.
        let has_complex_keyword = COMPLEX_KEYWORDS.iter().any(|kw| desc_lower.contains(kw));

        // 2. Deep conversation history.
        let deep_history = history.len() > COMPLEX_HISTORY_THRESHOLD;

        // 3. Multiple distinct tool types used.
        let unique_tools: std::collections::HashSet<&str> = tool_names.iter().copied().collect();
        let multi_tool_types = unique_tools.len() > 2;

        // 4. DONE criteria present (compound task).
        let has_done_criteria = desc_lower.contains("done criteria")
            || desc_lower.contains("done when")
            || desc_lower.contains("acceptance criteria")
            || self.history_contains_done_criteria(history);

        if has_complex_keyword || (deep_history && multi_tool_types) || has_done_criteria {
            return TaskComplexity::Complex;
        }

        // ── Simple signals ───────────────────────────────────────────

        // Short description.
        let short_description = task_description.len() < SIMPLE_DESCRIPTION_MAX_LEN;

        // Single tool call (or zero).
        let single_tool = tool_names.len() <= 1;

        // All requested tools are read-only.
        let all_read_only = tool_names.iter().all(|t| READ_ONLY_TOOLS.contains(t));

        if short_description && single_tool && all_read_only && !deep_history {
            return TaskComplexity::Simple;
        }

        // ── Default ──────────────────────────────────────────────────
        TaskComplexity::Standard
    }

    /// Map a complexity level to a model tier.
    pub fn select_tier(complexity: TaskComplexity) -> ModelTier {
        match complexity {
            TaskComplexity::Trivial => ModelTier::Fast,
            TaskComplexity::Simple => ModelTier::Fast,
            TaskComplexity::Standard => ModelTier::Primary,
            TaskComplexity::Complex => ModelTier::Premium,
        }
    }

    /// Resolve a model tier to a concrete model name string, falling
    /// back to the primary model if a tier's model is not configured.
    pub fn get_model_name(&self, tier: ModelTier) -> &str {
        match tier {
            ModelTier::Fast => self
                .config
                .fast_model
                .as_deref()
                .unwrap_or(&self.config.primary_model),
            ModelTier::Primary => &self.config.primary_model,
            ModelTier::Premium => self
                .config
                .premium_model
                .as_deref()
                .unwrap_or(&self.config.primary_model),
        }
    }

    /// Detect user override prefixes (`!fast`, `!best`) in the task
    /// description. Returns `Some(tier)` if an override is found.
    fn detect_user_override(task_description: &str) -> Option<ModelTier> {
        let trimmed = task_description.trim_start();
        if trimmed.starts_with(FORCE_FAST_PREFIX) {
            Some(ModelTier::Fast)
        } else if trimmed.starts_with(FORCE_BEST_PREFIX) {
            Some(ModelTier::Premium)
        } else {
            None
        }
    }

    /// Check whether the conversation history already contains DONE
    /// criteria injected by the done-criteria engine.
    fn history_contains_done_criteria(&self, history: &[ChatMessage]) -> bool {
        for msg in history {
            if !matches!(msg.role, Role::System) {
                continue;
            }
            let text = match &msg.content {
                MessageContent::Text(t) => t.as_str(),
                MessageContent::Parts(parts) => {
                    // Check each text part.
                    for part in parts {
                        if let ContentPart::Text { text } = part {
                            let lower = text.to_lowercase();
                            if lower.contains("done criteria")
                                || lower.contains("completion conditions")
                            {
                                return true;
                            }
                        }
                    }
                    continue;
                }
            };
            let lower = text.to_lowercase();
            if lower.contains("done criteria") || lower.contains("completion conditions") {
                return true;
            }
        }
        false
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use skyclaw_core::types::message::{ChatMessage, MessageContent, Role};

    fn make_config(fast: Option<&str>, primary: &str, premium: Option<&str>) -> ModelRouterConfig {
        ModelRouterConfig {
            enabled: true,
            fast_model: fast.map(|s| s.to_string()),
            primary_model: primary.to_string(),
            premium_model: premium.map(|s| s.to_string()),
        }
    }

    fn make_router() -> ModelRouter {
        ModelRouter::new(make_config(
            Some("claude-haiku-4-5-20251001"),
            "claude-sonnet-4-6",
            Some("claude-opus-4-6"),
        ))
    }

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: MessageContent::Text(text.to_string()),
        }
    }

    fn assistant_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Text(text.to_string()),
        }
    }

    fn system_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::System,
            content: MessageContent::Text(text.to_string()),
        }
    }

    // ── Classification tests ─────────────────────────────────────────

    #[test]
    fn simple_task_short_readonly() {
        let router = make_router();
        let history = vec![user_msg("show me the file")];
        let complexity = router.classify_complexity(&history, &["file_read"], "show me the file");
        assert_eq!(complexity, TaskComplexity::Simple);
    }

    #[test]
    fn simple_task_no_tools() {
        let router = make_router();
        let history = vec![user_msg("hi")];
        let complexity = router.classify_complexity(&history, &[], "hi");
        assert_eq!(complexity, TaskComplexity::Trivial);
    }

    #[test]
    fn simple_task_git_status() {
        let router = make_router();
        let history = vec![user_msg("git status")];
        let complexity = router.classify_complexity(&history, &["git_status"], "git status");
        assert_eq!(complexity, TaskComplexity::Simple);
    }

    #[test]
    fn standard_task_long_description() {
        let router = make_router();
        let long_desc = "Please read the configuration file and then update the database \
                         connection string to point to the new staging server at db.staging.internal";
        let history = vec![user_msg(long_desc)];
        let complexity = router.classify_complexity(&history, &["file_read"], long_desc);
        assert_eq!(complexity, TaskComplexity::Standard);
    }

    #[test]
    fn standard_task_write_tool() {
        let router = make_router();
        let history = vec![user_msg("write hello to file.txt")];
        let complexity =
            router.classify_complexity(&history, &["file_write"], "write hello to file.txt");
        assert_eq!(complexity, TaskComplexity::Standard);
    }

    #[test]
    fn complex_task_architecture_keyword() {
        let router = make_router();
        let history = vec![user_msg("design the architecture for the new auth system")];
        let complexity = router.classify_complexity(
            &history,
            &["shell"],
            "design the architecture for the new auth system",
        );
        assert_eq!(complexity, TaskComplexity::Complex);
    }

    #[test]
    fn complex_task_debug_keyword() {
        let router = make_router();
        let history = vec![user_msg("debug the memory leak in the worker pool")];
        let complexity = router.classify_complexity(
            &history,
            &["shell"],
            "debug the memory leak in the worker pool",
        );
        assert_eq!(complexity, TaskComplexity::Complex);
    }

    #[test]
    fn complex_task_refactor_keyword() {
        let router = make_router();
        let history = vec![user_msg("refactor the error handling")];
        let complexity =
            router.classify_complexity(&history, &["shell"], "refactor the error handling");
        assert_eq!(complexity, TaskComplexity::Complex);
    }

    #[test]
    fn complex_task_deep_history_multi_tools() {
        let router = make_router();
        // Build a history with > 10 messages.
        let mut history = Vec::new();
        for i in 0..12 {
            history.push(user_msg(&format!("message {}", i)));
            history.push(assistant_msg(&format!("reply {}", i)));
        }
        let complexity = router.classify_complexity(
            &history,
            &["shell", "file_write", "git_status"],
            "continue working on the task",
        );
        assert_eq!(complexity, TaskComplexity::Complex);
    }

    #[test]
    fn complex_task_done_criteria_in_description() {
        let router = make_router();
        let history = vec![user_msg("done criteria: all tests pass")];
        let complexity =
            router.classify_complexity(&history, &["shell"], "done criteria: all tests pass");
        assert_eq!(complexity, TaskComplexity::Complex);
    }

    #[test]
    fn complex_task_done_criteria_in_history() {
        let router = make_router();
        let history = vec![
            user_msg("build a REST API"),
            system_msg("DONE CRITERIA: 1. Server starts. 2. Tests pass."),
            assistant_msg("I'll start building it."),
        ];
        let complexity = router.classify_complexity(&history, &["shell"], "continue");
        assert_eq!(complexity, TaskComplexity::Complex);
    }

    // ── Tier selection tests ─────────────────────────────────────────

    #[test]
    fn select_tier_simple_maps_to_fast() {
        assert_eq!(
            ModelRouter::select_tier(TaskComplexity::Simple),
            ModelTier::Fast
        );
    }

    #[test]
    fn select_tier_standard_maps_to_primary() {
        assert_eq!(
            ModelRouter::select_tier(TaskComplexity::Standard),
            ModelTier::Primary
        );
    }

    #[test]
    fn select_tier_complex_maps_to_premium() {
        assert_eq!(
            ModelRouter::select_tier(TaskComplexity::Complex),
            ModelTier::Premium
        );
    }

    // ── Model name lookup tests ──────────────────────────────────────

    #[test]
    fn get_model_name_with_all_tiers_configured() {
        let router = make_router();
        assert_eq!(
            router.get_model_name(ModelTier::Fast),
            "claude-haiku-4-5-20251001"
        );
        assert_eq!(
            router.get_model_name(ModelTier::Primary),
            "claude-sonnet-4-6"
        );
        assert_eq!(router.get_model_name(ModelTier::Premium), "claude-opus-4-6");
    }

    #[test]
    fn get_model_name_fast_falls_back_to_primary() {
        let router = ModelRouter::new(make_config(
            None,
            "claude-sonnet-4-6",
            Some("claude-opus-4-6"),
        ));
        assert_eq!(router.get_model_name(ModelTier::Fast), "claude-sonnet-4-6");
    }

    #[test]
    fn get_model_name_premium_falls_back_to_primary() {
        let router = ModelRouter::new(make_config(
            Some("claude-haiku-4-5-20251001"),
            "claude-sonnet-4-6",
            None,
        ));
        assert_eq!(
            router.get_model_name(ModelTier::Premium),
            "claude-sonnet-4-6"
        );
    }

    #[test]
    fn get_model_name_all_fallback_to_primary() {
        let router = ModelRouter::new(make_config(None, "claude-sonnet-4-6", None));
        assert_eq!(router.get_model_name(ModelTier::Fast), "claude-sonnet-4-6");
        assert_eq!(
            router.get_model_name(ModelTier::Primary),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            router.get_model_name(ModelTier::Premium),
            "claude-sonnet-4-6"
        );
    }

    // ── User override tests ──────────────────────────────────────────

    #[test]
    fn user_override_fast() {
        assert_eq!(
            ModelRouter::detect_user_override("!fast read the logs"),
            Some(ModelTier::Fast)
        );
    }

    #[test]
    fn user_override_best() {
        assert_eq!(
            ModelRouter::detect_user_override("!best redesign the API layer"),
            Some(ModelTier::Premium)
        );
    }

    #[test]
    fn user_override_none() {
        assert_eq!(
            ModelRouter::detect_user_override("just a normal message"),
            None
        );
    }

    #[test]
    fn user_override_with_leading_whitespace() {
        assert_eq!(
            ModelRouter::detect_user_override("  !fast check status"),
            Some(ModelTier::Fast)
        );
    }

    // ── Verification always uses Primary or Premium ──────────────────

    #[test]
    fn verification_uses_premium_when_available() {
        let router = make_router();
        let model = router.route(&[], &[], "verify the output", true);
        assert_eq!(model, "claude-opus-4-6");
    }

    #[test]
    fn verification_uses_primary_when_no_premium() {
        let router = ModelRouter::new(make_config(
            Some("claude-haiku-4-5-20251001"),
            "claude-sonnet-4-6",
            None,
        ));
        let model = router.route(&[], &[], "verify the output", true);
        assert_eq!(model, "claude-sonnet-4-6");
    }

    #[test]
    fn verification_ignores_user_override() {
        let router = make_router();
        // Even with !fast prefix, verification should use premium.
        let model = router.route(&[], &[], "!fast verify", true);
        assert_eq!(model, "claude-opus-4-6");
    }

    // ── Disabled router always returns primary ───────────────────────

    #[test]
    fn disabled_router_returns_primary() {
        let config = ModelRouterConfig {
            enabled: false,
            fast_model: Some("claude-haiku-4-5-20251001".to_string()),
            primary_model: "claude-sonnet-4-6".to_string(),
            premium_model: Some("claude-opus-4-6".to_string()),
        };
        let router = ModelRouter::new(config);
        let model = router.route(&[], &[], "debug the architecture", false);
        assert_eq!(model, "claude-sonnet-4-6");
    }

    // ── Route integration tests ──────────────────────────────────────

    #[test]
    fn route_simple_task_to_fast_model() {
        let router = make_router();
        let history = vec![user_msg("git status")];
        let model = router.route(&history, &["git_status"], "git status", false);
        assert_eq!(model, "claude-haiku-4-5-20251001");
    }

    #[test]
    fn route_standard_task_to_primary_model() {
        let router = make_router();
        let history = vec![user_msg(
            "update the config file with new database credentials",
        )];
        let model = router.route(
            &history,
            &["file_write"],
            "update the config file with new database credentials",
            false,
        );
        assert_eq!(model, "claude-sonnet-4-6");
    }

    #[test]
    fn route_complex_task_to_premium_model() {
        let router = make_router();
        let history = vec![user_msg("refactor the authentication module")];
        let model = router.route(
            &history,
            &["shell", "file_write"],
            "refactor the authentication module",
            false,
        );
        assert_eq!(model, "claude-opus-4-6");
    }

    #[test]
    fn route_user_override_fast_overrides_complexity() {
        let router = make_router();
        // Even though "refactor" is a complex keyword, !fast forces Fast tier.
        let model = router.route(&[], &["shell"], "!fast refactor the module", false);
        assert_eq!(model, "claude-haiku-4-5-20251001");
    }

    #[test]
    fn route_user_override_best_overrides_complexity() {
        let router = make_router();
        // Even though it's a simple task, !best forces Premium tier.
        let model = router.route(&[], &["git_status"], "!best git status", false);
        assert_eq!(model, "claude-opus-4-6");
    }

    // ── Config serde tests ───────────────────────────────────────────

    #[test]
    fn config_serde_roundtrip() {
        let config = ModelRouterConfig {
            enabled: true,
            fast_model: Some("claude-haiku-4-5-20251001".to_string()),
            primary_model: "claude-sonnet-4-6".to_string(),
            premium_model: Some("claude-opus-4-6".to_string()),
        };
        let json = serde_json::to_string(&config).unwrap();
        let restored: ModelRouterConfig = serde_json::from_str(&json).unwrap();
        assert!(restored.enabled);
        assert_eq!(
            restored.fast_model.as_deref(),
            Some("claude-haiku-4-5-20251001")
        );
        assert_eq!(restored.primary_model, "claude-sonnet-4-6");
        assert_eq!(restored.premium_model.as_deref(), Some("claude-opus-4-6"));
    }

    #[test]
    fn config_default_values() {
        let config = ModelRouterConfig::default();
        assert!(!config.enabled);
        assert!(config.fast_model.is_none());
        assert_eq!(config.primary_model, "claude-sonnet-4-6");
        assert!(config.premium_model.is_none());
    }

    #[test]
    fn config_toml_roundtrip() {
        let config = ModelRouterConfig {
            enabled: true,
            fast_model: Some("claude-haiku-4-5-20251001".to_string()),
            primary_model: "claude-sonnet-4-6".to_string(),
            premium_model: None,
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let restored: ModelRouterConfig = toml::from_str(&toml_str).unwrap();
        assert!(restored.enabled);
        assert_eq!(
            restored.fast_model.as_deref(),
            Some("claude-haiku-4-5-20251001")
        );
        assert!(restored.premium_model.is_none());
    }

    #[test]
    fn config_deserialize_minimal_toml() {
        let toml_str = r#"
            enabled = true
            primary_model = "gpt-4o"
        "#;
        let config: ModelRouterConfig = toml::from_str(toml_str).unwrap();
        assert!(config.enabled);
        assert_eq!(config.primary_model, "gpt-4o");
        assert!(config.fast_model.is_none());
        assert!(config.premium_model.is_none());
    }

    // ── Edge cases ───────────────────────────────────────────────────

    #[test]
    fn empty_history_and_no_tools_is_trivial() {
        let router = make_router();
        let complexity = router.classify_complexity(&[], &[], "hello");
        assert_eq!(complexity, TaskComplexity::Trivial);
    }

    #[test]
    fn complex_keyword_case_insensitive() {
        let router = make_router();
        let complexity = router.classify_complexity(&[], &["shell"], "DEBUG the connection issue");
        assert_eq!(complexity, TaskComplexity::Complex);
    }

    #[test]
    fn multiple_readonly_tools_still_standard() {
        // More than 1 tool → not single_tool, so not Simple even if all read-only.
        let router = make_router();
        let complexity =
            router.classify_complexity(&[], &["file_read", "git_status"], "check files");
        assert_eq!(complexity, TaskComplexity::Standard);
    }

    #[test]
    fn deep_history_alone_not_complex() {
        // Deep history without multi-tool-types should be Standard, not Complex.
        let router = make_router();
        let mut history = Vec::new();
        for i in 0..12 {
            history.push(user_msg(&format!("msg {}", i)));
            history.push(assistant_msg(&format!("reply {}", i)));
        }
        let complexity = router.classify_complexity(&history, &["shell"], "continue");
        assert_eq!(complexity, TaskComplexity::Standard);
    }

    #[test]
    fn acceptance_criteria_triggers_complex() {
        let router = make_router();
        let complexity = router.classify_complexity(
            &[],
            &["shell"],
            "build feature X. acceptance criteria: tests pass, docs updated",
        );
        assert_eq!(complexity, TaskComplexity::Complex);
    }

    // ── Trivial classification tests ────────────────────────────────

    #[test]
    fn trivial_greeting_hi() {
        let router = make_router();
        let complexity = router.classify_complexity(&[], &[], "hi");
        assert_eq!(complexity, TaskComplexity::Trivial);
    }

    #[test]
    fn trivial_greeting_thanks() {
        let router = make_router();
        let complexity = router.classify_complexity(&[], &[], "thanks");
        assert_eq!(complexity, TaskComplexity::Trivial);
    }

    #[test]
    fn trivial_short_no_action() {
        let router = make_router();
        let complexity = router.classify_complexity(&[], &[], "cool");
        assert_eq!(complexity, TaskComplexity::Trivial);
    }

    #[test]
    fn not_trivial_with_action_verb() {
        let router = make_router();
        let complexity = router.classify_complexity(&[], &[], "help me fix this");
        assert_ne!(complexity, TaskComplexity::Trivial);
    }

    #[test]
    fn not_trivial_with_path() {
        let router = make_router();
        let complexity = router.classify_complexity(&[], &[], "read /etc/hosts");
        assert_ne!(complexity, TaskComplexity::Trivial);
    }

    #[test]
    fn not_trivial_long_message() {
        let router = make_router();
        let long = "I was wondering if you could tell me a bit about how the authentication system works in this project";
        let complexity = router.classify_complexity(&[], &[], long);
        assert_ne!(complexity, TaskComplexity::Trivial);
    }

    #[test]
    fn trivial_emoji_response() {
        let router = make_router();
        let complexity = router.classify_complexity(&[], &[], "\u{1f44d}");
        assert_eq!(complexity, TaskComplexity::Trivial);
    }

    #[test]
    fn execution_profile_from_complexity() {
        assert!(TaskComplexity::Trivial.execution_profile().skip_tool_loop);
        assert!(!TaskComplexity::Simple.execution_profile().skip_tool_loop);
        assert!(TaskComplexity::Standard.execution_profile().use_learn);
        assert_eq!(
            TaskComplexity::Complex.execution_profile().max_iterations,
            10
        );
    }

    #[test]
    fn select_tier_trivial_maps_to_fast() {
        assert_eq!(
            ModelRouter::select_tier(TaskComplexity::Trivial),
            ModelTier::Fast
        );
    }
}
