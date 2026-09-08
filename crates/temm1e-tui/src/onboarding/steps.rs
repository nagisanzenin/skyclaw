//! Onboarding step definitions and state machine.

use crate::widgets::select_list::{SelectItem, SelectState};

/// Onboarding wizard states.
#[derive(Debug, Clone)]
pub enum OnboardingStep {
    Welcome,
    SelectMode(SelectState<String>),
    SelectProvider(SelectState<String>),
    /// Entered only for providers that require a custom endpoint
    /// (openai-compatible, ollama, lmstudio). See
    /// [`provider_needs_base_url`].
    EnterBaseUrl {
        provider: String,
        input: String,
        error: Option<String>,
    },
    EnterApiKey {
        provider: String,
        input: String,
        error: Option<String>,
    },
    ValidatingKey {
        provider: String,
    },
    SelectModel(SelectState<String>),
    /// Free-text model entry for providers that expose arbitrary models
    /// (openai-compatible, lmstudio, ollama running a custom tag). See
    /// [`provider_needs_custom_model`].
    EnterCustomModel {
        provider: String,
        input: String,
        error: Option<String>,
    },
    Confirm {
        provider: String,
        model: String,
    },
    Saving,
    Done,
}

/// Create the mode selection list.
pub fn mode_select_items() -> Vec<SelectItem<String>> {
    vec![
        SelectItem {
            value: "auto".to_string(),
            label: "Auto".to_string(),
            description: "Adapts personality to each task (recommended)".to_string(),
        },
        SelectItem {
            value: "play".to_string(),
            label: "Play  :3".to_string(),
            description: "Warm, energetic, slightly chaotic".to_string(),
        },
        SelectItem {
            value: "work".to_string(),
            label: "Work >:3".to_string(),
            description: "Sharp, precise, structured".to_string(),
        },
        SelectItem {
            value: "pro".to_string(),
            label: "Pro".to_string(),
            description: "Professional, no emoticons, consultant tone".to_string(),
        },
        SelectItem {
            value: "none".to_string(),
            label: "None".to_string(),
            description: "No personality, minimal identity prompt".to_string(),
        },
    ]
}

/// Create the provider selection list.
pub fn provider_select_items() -> Vec<SelectItem<String>> {
    vec![
        SelectItem {
            value: "anthropic".to_string(),
            label: "Anthropic".to_string(),
            description: "Claude models (recommended)".to_string(),
        },
        SelectItem {
            value: "openai".to_string(),
            label: "OpenAI".to_string(),
            description: "GPT models".to_string(),
        },
        SelectItem {
            value: "gemini".to_string(),
            label: "Gemini".to_string(),
            description: "Google Gemini".to_string(),
        },
        SelectItem {
            value: "grok".to_string(),
            label: "Grok".to_string(),
            description: "xAI Grok".to_string(),
        },
        SelectItem {
            value: "openrouter".to_string(),
            label: "OpenRouter".to_string(),
            description: "Multiple providers via proxy".to_string(),
        },
        SelectItem {
            value: "zai-coding-plan".to_string(),
            label: "Z.ai Coding Plan".to_string(),
            description: "Subscription endpoint · separate from metered API".to_string(),
        },
        SelectItem {
            value: "zai".to_string(),
            label: "Z.ai".to_string(),
            description: "Zhipu GLM models".to_string(),
        },
        SelectItem {
            value: "minimax".to_string(),
            label: "MiniMax".to_string(),
            description: "MiniMax models".to_string(),
        },
        SelectItem {
            value: "stepfun".to_string(),
            label: "StepFun".to_string(),
            description: "Step 3.5 Flash, Step 3 (256K context, ultra-cheap)".to_string(),
        },
        SelectItem {
            value: "ollama".to_string(),
            label: "Ollama".to_string(),
            description: "Local models via Ollama".to_string(),
        },
        SelectItem {
            value: "lmstudio".to_string(),
            label: "LM Studio".to_string(),
            description: "Local models via LM Studio".to_string(),
        },
        SelectItem {
            value: "openai-compatible".to_string(),
            label: "OpenAI-compatible (custom)".to_string(),
            description: "Any OpenAI-compatible endpoint (vLLM, proxies, self-hosted)".to_string(),
        },
    ]
}

/// Whether a provider needs a custom base_url step in onboarding.
///
/// Returns `true` for providers that point at arbitrary endpoints (local or
/// self-hosted). Vendor presets (anthropic, openai, gemini, …) embed their
/// base_url in the factory and skip this step.
pub fn provider_needs_base_url(provider: &str) -> bool {
    matches!(provider, "openai-compatible" | "ollama" | "lmstudio")
}

/// Whether a provider expects free-text model entry instead of a preset list.
///
/// Returns `true` when the provider runs arbitrary local models — the user
/// supplies the exact tag/name (e.g., `rwkv7`, `llama3.3:8b-q4_K_M`).
pub fn provider_needs_custom_model(provider: &str) -> bool {
    // Empty available-models list is the canonical signal, but we enumerate
    // explicitly so tests can assert the contract.
    matches!(provider, "openai-compatible")
}

/// Suggested default base_url for providers that support local endpoints.
///
/// Pre-fills the EnterBaseUrl step. Users can edit or clear before submitting.
pub fn default_base_url_for_provider(provider: &str) -> Option<&'static str> {
    match provider {
        "ollama" => Some("http://localhost:11434/v1"),
        "lmstudio" => Some("http://localhost:1234/v1"),
        // openai-compatible is intentionally blank — users MUST supply their
        // own endpoint (there is no "common" default for arbitrary proxies).
        _ => None,
    }
}

/// Create the model selection list for a provider.
pub fn model_select_items(provider: &str) -> Vec<SelectItem<String>> {
    use temm1e_core::types::model_registry::{
        available_models_for_provider, is_vision_model, model_limits,
    };

    available_models_for_provider(provider)
        .into_iter()
        .map(|model| {
            let (ctx_window, max_output) = model_limits(model);
            let vision = if is_vision_model(model) {
                " | Vision"
            } else {
                ""
            };
            SelectItem {
                value: model.to_string(),
                label: model.to_string(),
                description: format!(
                    "{}K ctx / {}K out{}",
                    ctx_window / 1000,
                    max_output / 1000,
                    vision,
                ),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_compatible_preset_present() {
        let items = provider_select_items();
        assert!(
            items.iter().any(|i| i.value == "openai-compatible"),
            "openai-compatible preset must appear in the onboarding provider list"
        );
    }

    #[test]
    fn lmstudio_preset_present() {
        let items = provider_select_items();
        assert!(items.iter().any(|i| i.value == "lmstudio"));
    }

    #[test]
    fn providers_needing_base_url() {
        assert!(provider_needs_base_url("openai-compatible"));
        assert!(provider_needs_base_url("ollama"));
        assert!(provider_needs_base_url("lmstudio"));
        // Vendor presets must NOT go through the base_url step
        assert!(!provider_needs_base_url("anthropic"));
        assert!(!provider_needs_base_url("openai"));
        assert!(!provider_needs_base_url("gemini"));
        assert!(!provider_needs_base_url("grok"));
        assert!(!provider_needs_base_url("openrouter"));
        assert!(!provider_needs_base_url("zai"));
        assert!(!provider_needs_base_url("minimax"));
        assert!(!provider_needs_base_url("stepfun"));
    }

    #[test]
    fn custom_model_required_for_openai_compatible() {
        assert!(provider_needs_custom_model("openai-compatible"));
        // Providers with curated model lists must NOT hit the custom-model path
        assert!(!provider_needs_custom_model("anthropic"));
        assert!(!provider_needs_custom_model("openai"));
        assert!(!provider_needs_custom_model("ollama"));
    }

    #[test]
    fn default_base_url_hints() {
        assert_eq!(
            default_base_url_for_provider("ollama"),
            Some("http://localhost:11434/v1")
        );
        assert_eq!(
            default_base_url_for_provider("lmstudio"),
            Some("http://localhost:1234/v1")
        );
        // openai-compatible intentionally has no default — user must supply one
        assert_eq!(default_base_url_for_provider("openai-compatible"), None);
        assert_eq!(default_base_url_for_provider("anthropic"), None);
    }

    #[test]
    fn openai_compatible_has_empty_default_model() {
        use temm1e_core::types::model_registry::default_model;
        // Empty sentinel forces the custom-model prompt
        assert_eq!(default_model("openai-compatible"), "");
    }

    #[test]
    fn openai_compatible_has_empty_model_list() {
        use temm1e_core::types::model_registry::available_models_for_provider;
        assert!(available_models_for_provider("openai-compatible").is_empty());
    }
}
