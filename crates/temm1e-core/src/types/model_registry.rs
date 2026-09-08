//! Model registry — known context window and output token limits for LLM models.
//!
//! Every model entry is sourced from official provider documentation.
//! See `docs/MODEL_REGISTRY.md` for the full reference with sources and pricing.
//!
//! # Usage
//!
//! ```
//! use temm1e_core::types::model_registry::model_limits;
//!
//! let (context_window, max_output) = model_limits("claude-sonnet-4-6");
//! assert_eq!(context_window, 200_000);
//! assert_eq!(max_output, 64_000);
//! ```

/// Model capability limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelLimits {
    /// Maximum input context window in tokens.
    pub context_window: usize,
    /// Maximum output tokens the model can generate.
    pub max_output_tokens: usize,
}

/// Default limits for unknown models — conservative enough for most modern
/// models while not wasting small-context budgets.
pub const DEFAULT_LIMITS: ModelLimits = ModelLimits {
    context_window: 128_000,
    max_output_tokens: 16_384,
};

/// Look up the context window and max output tokens for a model.
///
/// Returns `(context_window, max_output_tokens)`.
///
/// Models are matched by exact name first, then by suffix (stripping provider
/// prefixes like `openai/`, `anthropic/`, etc.) to handle OpenRouter-style
/// namespaced model IDs.
pub fn model_limits(model: &str) -> (usize, usize) {
    let limits = lookup(model)
        .or_else(|| {
            // Try stripping provider prefix: "provider/model" → "model"
            model.split('/').next_back().and_then(lookup)
        })
        .unwrap_or(DEFAULT_LIMITS);
    (limits.context_window, limits.max_output_tokens)
}

/// Return just the ModelLimits struct for a model.
pub fn get_model_limits(model: &str) -> ModelLimits {
    lookup(model)
        .or_else(|| model.split('/').next_back().and_then(lookup))
        .unwrap_or(DEFAULT_LIMITS)
}

/// Look up limits with custom-model awareness.
///
/// Tries the active provider's custom models first (from
/// `~/.temm1e/custom_models.toml`), then falls back to the hardcoded
/// registry via [`model_limits`]. This wrapper is opt-in: existing callers
/// that don't know the active provider keep using [`model_limits`] and see
/// byte-identical behavior. Only callers that know the provider context
/// (e.g. main.rs at agent-init time) should call this variant.
pub fn model_limits_with_custom(provider: &str, model: &str) -> (usize, usize) {
    if let Some(cm) = crate::config::custom_models::lookup_custom_model(provider, model) {
        return (cm.context_window, cm.max_output_tokens);
    }
    model_limits(model)
}

fn lookup(model: &str) -> Option<ModelLimits> {
    // Current, source-dated facts supersede the legacy compatibility bank.
    if let Some(limits) = super::model_catalog::CATALOG
        .iter()
        .find(|row| row.model == model || row.aliases.iter().any(|alias| alias == model))
        .and_then(|row| row.limits)
    {
        return Some(ModelLimits {
            context_window: limits.context_window,
            max_output_tokens: limits.max_output_tokens,
        });
    }
    Some(match model {
        // ── Anthropic ─────────────────────────────────────────────────
        "claude-sonnet-4-6" | "claude-sonnet-4-20250514" | "claude-sonnet-4-0" => ModelLimits {
            context_window: 200_000,
            max_output_tokens: 64_000,
        },
        "claude-opus-4-6" | "claude-opus-4-20250514" | "claude-opus-4-0" => ModelLimits {
            context_window: 200_000,
            max_output_tokens: 128_000,
        },
        "claude-haiku-4-5" | "claude-haiku-4-5-20251001" => ModelLimits {
            context_window: 200_000,
            max_output_tokens: 64_000,
        },

        // ── OpenAI ────────────────────────────────────────────────────
        "gpt-5.4" => ModelLimits {
            context_window: 1_050_000,
            max_output_tokens: 128_000,
        },
        "gpt-5.3-codex" | "gpt-5.3-codex-spark" => ModelLimits {
            context_window: 400_000,
            max_output_tokens: 128_000,
        },
        "gpt-5.2" | "gpt-5.2-codex" => ModelLimits {
            context_window: 400_000,
            max_output_tokens: 128_000,
        },
        "gpt-5.1-codex" | "gpt-5.1-codex-mini" => ModelLimits {
            context_window: 400_000,
            max_output_tokens: 128_000,
        },
        "gpt-5" | "gpt-5-codex" | "gpt-5-codex-mini" | "gpt-5-mini" => ModelLimits {
            context_window: 400_000,
            max_output_tokens: 128_000,
        },
        // v5.3.2 additions — GPT-5.4 mini/nano tiers
        "gpt-5.4-mini" => ModelLimits {
            context_window: 400_000,
            max_output_tokens: 128_000,
        },
        "gpt-5.4-nano" => ModelLimits {
            context_window: 400_000,
            max_output_tokens: 128_000,
        },
        "gpt-4.1" => ModelLimits {
            context_window: 1_047_576,
            max_output_tokens: 32_768,
        },
        "gpt-4.1-mini" | "gpt-4.1-nano" => ModelLimits {
            context_window: 1_047_576,
            max_output_tokens: 32_768,
        },
        "o4-mini" => ModelLimits {
            context_window: 200_000,
            max_output_tokens: 100_000,
        },
        // v5.3.2 addition — o3-mini replaces o4-mini (retired Feb 2026)
        "o3-mini" => ModelLimits {
            context_window: 200_000,
            max_output_tokens: 100_000,
        },
        "gpt-4o" | "gpt-4o-2024-08-06" => ModelLimits {
            context_window: 128_000,
            max_output_tokens: 16_384,
        },
        "gpt-4o-mini" => ModelLimits {
            context_window: 128_000,
            max_output_tokens: 16_384,
        },
        "gpt-3.5-turbo" => ModelLimits {
            context_window: 16_385,
            max_output_tokens: 4_096,
        },

        // ── Google Gemini ─────────────────────────────────────────────
        "gemini-3-flash-preview" | "gemini-3-flash" => ModelLimits {
            context_window: 1_048_576,
            max_output_tokens: 65_536,
        },
        "gemini-3.1-pro-preview" | "gemini-3.1-pro" | "gemini-3-pro" => ModelLimits {
            context_window: 1_048_576,
            max_output_tokens: 65_536,
        },
        "gemini-3.1-flash-lite-preview" | "gemini-3.1-flash-lite" => ModelLimits {
            context_window: 1_048_576,
            max_output_tokens: 65_536,
        },
        "gemini-2.5-flash" | "gemini-2.5-flash-preview-05-20" => ModelLimits {
            context_window: 1_048_576,
            max_output_tokens: 65_536,
        },
        "gemini-2.5-pro" | "gemini-2.5-pro-preview-06-05" => ModelLimits {
            context_window: 1_048_576,
            max_output_tokens: 65_536,
        },

        // ── xAI Grok ──────────────────────────────────────────────────
        "grok-4-1-fast-non-reasoning" | "grok-4-1-fast" | "grok-4.1-fast" => ModelLimits {
            context_window: 2_000_000,
            max_output_tokens: 30_000,
        },
        "grok-3" | "grok-3-latest" => ModelLimits {
            context_window: 131_072,
            max_output_tokens: 131_072,
        },

        // ── Z.ai (Zhipu AI) ──────────────────────────────────────────
        // Z.ai model API guide, verified 2026-09-08:
        // https://docs.z.ai/guides/vlm/glm-5.3-flash (1M context, 128K output).
        "glm-5.3-flash" => ModelLimits {
            context_window: 1_000_000,
            max_output_tokens: 128_000,
        },
        "glm-4.7-flash" => ModelLimits {
            context_window: 200_000,
            max_output_tokens: 131_072,
        },
        "glm-4.7" => ModelLimits {
            context_window: 200_000,
            max_output_tokens: 131_072,
        },
        "glm-5" => ModelLimits {
            context_window: 200_000,
            max_output_tokens: 131_072,
        },
        "glm-4.6v" => ModelLimits {
            context_window: 131_072,
            max_output_tokens: 32_768,
        },
        // v5.3.2 additions — GLM 5.1 flagship + flashx tiers
        "glm-5.1" => ModelLimits {
            context_window: 200_000,
            max_output_tokens: 131_072,
        },
        "glm-4.7-flashx" => ModelLimits {
            context_window: 200_000,
            max_output_tokens: 131_072,
        },
        "glm-4.6v-flashx" => ModelLimits {
            context_window: 131_072,
            max_output_tokens: 32_768,
        },

        // ── MiniMax ───────────────────────────────────────────────────
        "MiniMax-M2.5" | "minimax-m2.5" => ModelLimits {
            context_window: 204_800,
            max_output_tokens: 196_608,
        },
        // v5.3.2 additions — MiniMax M2.7 (March 2026)
        "MiniMax-M2.7" | "minimax-m2.7" => ModelLimits {
            context_window: 204_800,
            max_output_tokens: 196_608,
        },
        "MiniMax-M2.7-highspeed" | "minimax-m2.7-highspeed" => ModelLimits {
            context_window: 204_800,
            max_output_tokens: 196_608,
        },

        // ── StepFun (阶跃星辰) ───────────────────────────────────────
        "step-3.5-flash" => ModelLimits {
            context_window: 262_144,
            max_output_tokens: 65_536,
        },
        "step-3" => ModelLimits {
            context_window: 262_144,
            max_output_tokens: 65_536,
        },
        "step-2-16k" => ModelLimits {
            context_window: 16_384,
            max_output_tokens: 4_096,
        },

        // ── Meta Llama ────────────────────────────────────────────────
        "llama-4-maverick" | "meta-llama/llama-4-maverick" => ModelLimits {
            context_window: 1_048_576,
            max_output_tokens: 16_384,
        },
        "llama-4-scout" | "meta-llama/llama-4-scout" => ModelLimits {
            context_window: 10_000_000,
            max_output_tokens: 16_384,
        },

        // ── DeepSeek ──────────────────────────────────────────────────
        "deepseek-v3.2" | "deepseek/deepseek-v3.2" => ModelLimits {
            context_window: 163_840,
            max_output_tokens: 65_536,
        },
        "deepseek-r1-0528" | "deepseek/deepseek-r1-0528" => ModelLimits {
            context_window: 163_840,
            max_output_tokens: 65_536,
        },
        "deepseek-r1" | "deepseek/deepseek-r1" => ModelLimits {
            context_window: 64_000,
            max_output_tokens: 16_000,
        },
        "deepseek-v3" | "deepseek/deepseek-v3-0324" => ModelLimits {
            context_window: 163_840,
            max_output_tokens: 65_536,
        },
        // v5.3.2 additions — DeepSeek now surfaces V3.2 via generic endpoints
        "deepseek-chat" | "deepseek/deepseek-chat" => ModelLimits {
            context_window: 128_000,
            max_output_tokens: 8_192,
        },
        "deepseek-reasoner" | "deepseek/deepseek-reasoner" => ModelLimits {
            context_window: 128_000,
            max_output_tokens: 65_536,
        },

        // ── Qwen (Alibaba) ───────────────────────────────────────────
        "qwen3-coder" | "qwen/qwen3-coder" => ModelLimits {
            context_window: 262_144,
            max_output_tokens: 262_000,
        },
        "qwen3-235b-a22b" | "qwen/qwen3-235b-a22b" => ModelLimits {
            context_window: 131_072,
            max_output_tokens: 32_768,
        },
        "qwen3-max" | "qwen/qwen3-max" => ModelLimits {
            context_window: 262_144,
            max_output_tokens: 32_768,
        },
        "qwen3.5-plus-02-15" | "qwen/qwen3.5-plus-02-15" | "qwen3.5-plus" => ModelLimits {
            context_window: 1_000_000,
            max_output_tokens: 32_768,
        },
        "qwen-2.5-7b-instruct" | "qwen/qwen-2.5-7b-instruct" => ModelLimits {
            context_window: 32_768,
            max_output_tokens: 8_192,
        },
        "qwen-2.5-72b-instruct" | "qwen/qwen-2.5-72b-instruct" => ModelLimits {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
        // v5.3.2 addition — Qwen 3.5 economy tier (Feb 2026)
        "qwen3.5-flash" | "qwen/qwen3.5-flash" => ModelLimits {
            context_window: 1_048_576,
            max_output_tokens: 65_536,
        },

        // ── Mistral ──────────────────────────────────────────────────
        "mistral-large-2512" | "mistralai/mistral-large-2512" => ModelLimits {
            context_window: 262_144,
            max_output_tokens: 32_768,
        },
        "mistral-medium-3" | "mistralai/mistral-medium-3" => ModelLimits {
            context_window: 131_072,
            max_output_tokens: 32_768,
        },
        "mistral-medium-3.1" | "mistralai/mistral-medium-3.1" => ModelLimits {
            context_window: 32_000,
            max_output_tokens: 32_768,
        },
        // v5.3.2 additions — Mistral current-generation models (2025-2026)
        "mistral-small-2603" | "mistralai/mistral-small-2603" => ModelLimits {
            context_window: 262_144,
            max_output_tokens: 32_768,
        },
        "mistral-small-2506" | "mistralai/mistral-small-2506" => ModelLimits {
            context_window: 131_072,
            max_output_tokens: 32_768,
        },
        "mistral-medium-2508" | "mistralai/mistral-medium-2508" => ModelLimits {
            context_window: 131_072,
            max_output_tokens: 32_768,
        },
        "mistral-medium-2505" | "mistralai/mistral-medium-2505" => ModelLimits {
            context_window: 131_072,
            max_output_tokens: 32_768,
        },
        "ministral-3b-2512" | "mistralai/ministral-3b-2512" => ModelLimits {
            context_window: 262_144,
            max_output_tokens: 32_768,
        },
        "ministral-8b-2512" | "mistralai/ministral-8b-2512" => ModelLimits {
            context_window: 262_144,
            max_output_tokens: 32_768,
        },
        "ministral-14b-2512" | "mistralai/ministral-14b-2512" => ModelLimits {
            context_window: 262_144,
            max_output_tokens: 32_768,
        },
        "devstral-2512" | "mistralai/devstral-2512" => ModelLimits {
            context_window: 262_144,
            max_output_tokens: 32_768,
        },

        // ── Cohere ───────────────────────────────────────────────────
        "command-a" | "cohere/command-a" => ModelLimits {
            context_window: 256_000,
            max_output_tokens: 4_096,
        },
        "command-r-plus" | "cohere/command-r-plus-08-2024" | "command-r-plus-08-2024" => {
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 4_096,
            }
        }
        // v5.3.2 addition — Command A Reasoning (August 2025)
        "command-a-reasoning-08-2025" | "cohere/command-a-reasoning-08-2025" => ModelLimits {
            context_window: 256_000,
            max_output_tokens: 32_768,
        },

        // ── OpenRouter Stealth ───────────────────────────────────────
        "hunter-alpha" | "openrouter/hunter-alpha" => ModelLimits {
            context_window: 1_048_576,
            max_output_tokens: 32_000,
        },

        // ── Microsoft ────────────────────────────────────────────────
        "phi-4" | "microsoft/phi-4" => ModelLimits {
            context_window: 16_384,
            max_output_tokens: 16_384,
        },
        // v5.3.2 additions — Phi-4 mini and multimodal variants
        "phi-4-mini-instruct" | "microsoft/phi-4-mini-instruct" => ModelLimits {
            context_window: 131_072,
            max_output_tokens: 4_096,
        },
        "phi-4-multimodal-instruct" | "microsoft/phi-4-multimodal-instruct" => ModelLimits {
            context_window: 131_072,
            max_output_tokens: 4_096,
        },

        _ => return None,
    })
}

/// Default model for each provider.
pub fn default_model(provider_name: &str) -> &'static str {
    match provider_name {
        "anthropic" => "claude-sonnet-4-6",
        "openai" => "gpt-5.2",
        "openai-codex" => "gpt-5.4",
        "gemini" => "gemini-3-flash-preview",
        "grok" | "xai" => "grok-4-1-fast-non-reasoning",
        "openrouter" => "anthropic/claude-sonnet-4-6",
        "minimax" => "MiniMax-M2.5",
        "stepfun" => "step-3.5-flash",
        "zai" => "glm-4.7-flash",
        "zai-coding-plan" => "glm-5.3-flash",
        "ollama" => "llama3.3",
        // LM Studio runs whatever local model the user downloaded — there is
        // no universal "default". This placeholder is a popular Qwen 3.5
        // variant; users override it with `/addmodel <name> ...` as soon as
        // they pick their actual model. See issue #45.
        "lmstudio" => "qwen3.5-7b-instruct",
        // Generic custom endpoint — model name must be supplied by the user
        // (onboarding prompts for it; `/addmodel` sets it post-hoc). Empty
        // sentinel signals callers to force user input.
        "openai-compatible" => "",
        _ => "claude-sonnet-4-6",
    }
}

/// Suggested model IDs for each provider (used by /model listing and onboarding).
/// These are not an account entitlement probe. Existing configured models and
/// explicit custom registrations remain valid even when absent from this list.
/// Modern additions have source-dated facts in `model_catalog.json`; inclusion
/// does not promote an unaccepted adapter or silently change saved defaults.
pub fn available_models_for_provider(provider: &str) -> Vec<&'static str> {
    match provider {
        "anthropic" => vec![
            "claude-sonnet-4-6",
            "claude-sonnet-5",
            "claude-opus-5",
            "claude-fable-5-1",
            "claude-opus-4-6",
            "claude-haiku-4-5",
        ],
        "openai" => vec![
            "gpt-5.4",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-6-astra",
            "gpt-5.5",
            "gpt-5.4-mini",
            "gpt-5.4-nano",
            "gpt-5.2",
            "gpt-4.1",
            "gpt-4.1-mini",
            "gpt-4o",
            "o3-mini",
            "o4-mini",
            "gpt-3.5-turbo",
        ],
        // Codex uses the native Responses adapter, not the API-key route.
        // Account-specific availability is still decided by the provider.
        "openai-codex" => vec![
            "gpt-5.4",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-6-astra",
            "gpt-5.5",
            "gpt-5.3-codex",
            "gpt-5.3-codex-spark",
        ],
        "gemini" => vec![
            "gemini-3-flash-preview",
            "gemini-3.8-flash",
            "gemini-3.1-pro-preview",
            "gemini-3.1-flash-lite-preview",
            "gemini-2.5-flash",
            "gemini-2.5-pro",
        ],
        "grok" | "xai" => vec!["grok-4-1-fast-non-reasoning", "grok-4.6", "grok-3"],
        "openrouter" => vec![
            "anthropic/claude-sonnet-4-6",
            "openai/gpt-5.2",
            "google/gemini-3-flash-preview",
            "openrouter/hunter-alpha",
        ],
        "zai-coding-plan" => vec!["glm-5.3-flash"],
        "zai" | "zhipu" => vec![
            "glm-5.3-flash",
            "glm-5.1",
            "glm-5",
            "glm-4.7",
            "glm-4.7-flash",
            "glm-4.7-flashx",
            "glm-4.6v",
            "glm-4.6v-flashx",
        ],
        "minimax" => vec![
            "MiniMax-M2.7",
            "MiniMax-M2.7-highspeed",
            "MiniMax-M2.5",
            "MiniMax-M2.5-highspeed",
        ],
        "stepfun" => vec!["step-3.5-flash", "step-3", "step-2-16k"],
        "deepseek" => vec!["deepseek-chat", "deepseek-reasoner"],
        "mistral" | "mistralai" => vec![
            "mistral-large-2512",
            "mistral-medium-2508",
            "mistral-small-2603",
            "ministral-14b-2512",
            "ministral-8b-2512",
            "ministral-3b-2512",
            "devstral-2512",
        ],
        "cohere" => vec!["command-a", "command-a-reasoning-08-2025", "command-r-plus"],
        "microsoft" => vec!["phi-4", "phi-4-mini-instruct", "phi-4-multimodal-instruct"],
        // LM Studio runs arbitrary local models — the suggestions below are
        // popular community defaults shown in /listmodels for orientation.
        // Real models must be registered via /addmodel so context window and
        // pricing are tracked correctly.
        "lmstudio" => vec![
            "qwen3.5-7b-instruct",
            "qwen3.5-14b-instruct",
            "qwen3-coder-30b-a3b",
            "llama-3.3-8b-instruct",
            "llama-3.3-70b-instruct",
            "mistral-7b-instruct",
            "phi-4",
        ],
        _ => vec![],
    }
}

/// Quick vision check for model display.
pub fn is_vision_model(model: &str) -> bool {
    let m = model.rsplit('/').next().unwrap_or(model).to_lowercase();
    if m == "glm-5.3-flash" {
        return true;
    }
    if m.starts_with("glm-") {
        return m.contains('v') && !m.starts_with("glm-5");
    }
    if m.starts_with("minimax") {
        return false;
    }
    if m.starts_with("step-") {
        return m == "step-3"; // only step-3 supports vision
    }
    if m.starts_with("gpt-3") {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_anthropic_models() {
        let (ctx, out) = model_limits("claude-sonnet-4-6");
        assert_eq!(ctx, 200_000);
        assert_eq!(out, 64_000);

        let (ctx, out) = model_limits("claude-opus-4-6");
        assert_eq!(ctx, 200_000);
        assert_eq!(out, 128_000);
    }

    #[test]
    fn known_openai_models() {
        let (ctx, out) = model_limits("gpt-5.4");
        assert_eq!(ctx, 1_050_000);
        assert_eq!(out, 128_000);

        let (ctx, out) = model_limits("gpt-4.1");
        assert_eq!(ctx, 1_047_576);
        assert_eq!(out, 32_768);

        let (ctx, out) = model_limits("o4-mini");
        assert_eq!(ctx, 200_000);
        assert_eq!(out, 100_000);
    }

    #[test]
    fn known_gemini_models() {
        let (ctx, out) = model_limits("gemini-3-flash-preview");
        assert_eq!(ctx, 1_048_576);
        assert_eq!(out, 65_536);

        let (ctx, out) = model_limits("gemini-3.1-flash-lite-preview");
        assert_eq!(ctx, 1_048_576);
        assert_eq!(out, 65_536);
    }

    #[test]
    fn known_grok_models() {
        let (ctx, out) = model_limits("grok-4-1-fast-non-reasoning");
        assert_eq!(ctx, 2_000_000);
        assert_eq!(out, 30_000);
    }

    #[test]
    fn known_proxy_models() {
        // DeepSeek
        let (ctx, out) = model_limits("deepseek-v3.2");
        assert_eq!(ctx, 163_840);
        assert_eq!(out, 65_536);

        // Qwen via OpenRouter prefix
        let (ctx, out) = model_limits("qwen/qwen3-coder");
        assert_eq!(ctx, 262_144);
        assert_eq!(out, 262_000);

        // Qwen small model (issue #6)
        let (ctx, out) = model_limits("qwen/qwen-2.5-7b-instruct");
        assert_eq!(ctx, 32_768);
        assert_eq!(out, 8_192);

        // Llama
        let (ctx, out) = model_limits("meta-llama/llama-4-maverick");
        assert_eq!(ctx, 1_048_576);
        assert_eq!(out, 16_384);

        // Mistral
        let (ctx, out) = model_limits("mistral-large-2512");
        assert_eq!(ctx, 262_144);
        assert_eq!(out, 32_768);
    }

    #[test]
    fn provider_prefix_stripping() {
        // "anthropic/claude-sonnet-4-6" → strips to "claude-sonnet-4-6"
        let (ctx, out) = model_limits("anthropic/claude-sonnet-4-6");
        assert_eq!(ctx, 200_000);
        assert_eq!(out, 64_000);

        // "openai/gpt-5.2" → strips to "gpt-5.2"
        let (ctx, out) = model_limits("openai/gpt-5.2");
        assert_eq!(ctx, 400_000);
        assert_eq!(out, 128_000);
    }

    #[test]
    fn unknown_model_gets_defaults() {
        let (ctx, out) = model_limits("some-unknown-model-v99");
        assert_eq!(ctx, DEFAULT_LIMITS.context_window);
        assert_eq!(out, DEFAULT_LIMITS.max_output_tokens);
    }

    #[test]
    fn zai_models() {
        let (ctx, out) = model_limits("glm-4.7-flash");
        assert_eq!(ctx, 200_000);
        assert_eq!(out, 131_072);

        let (ctx, out) = model_limits("glm-4.6v");
        assert_eq!(ctx, 131_072);
        assert_eq!(out, 32_768);
    }

    #[test]
    fn minimax_model() {
        let (ctx, out) = model_limits("MiniMax-M2.5");
        assert_eq!(ctx, 204_800);
        assert_eq!(out, 196_608);
    }

    #[test]
    fn hunter_alpha_model() {
        let (ctx, out) = model_limits("hunter-alpha");
        assert_eq!(ctx, 1_048_576);
        assert_eq!(out, 32_000);

        let (ctx, out) = model_limits("openrouter/hunter-alpha");
        assert_eq!(ctx, 1_048_576);
        assert_eq!(out, 32_000);
    }

    // ── v5.3.2: additions from Phase 4 research ──────────────────

    #[test]
    fn openai_gpt_5_4_tiers_registered() {
        let (ctx, out) = model_limits("gpt-5.4-mini");
        assert_eq!(ctx, 400_000);
        assert_eq!(out, 128_000);

        let (ctx, out) = model_limits("gpt-5.4-nano");
        assert_eq!(ctx, 400_000);
        assert_eq!(out, 128_000);
    }

    #[test]
    fn openai_o3_mini_registered() {
        let (ctx, out) = model_limits("o3-mini");
        assert_eq!(ctx, 200_000);
        assert_eq!(out, 100_000);
    }

    #[test]
    fn deepseek_generic_endpoints_registered() {
        // DeepSeek now surfaces V3.2 via generic deepseek-chat / deepseek-reasoner
        let (ctx, out) = model_limits("deepseek-chat");
        assert_eq!(ctx, 128_000);
        assert_eq!(out, 8_192);

        let (ctx, out) = model_limits("deepseek-reasoner");
        assert_eq!(ctx, 128_000);
        assert_eq!(out, 65_536);
    }

    #[test]
    fn qwen3_5_flash_registered() {
        let (ctx, out) = model_limits("qwen3.5-flash");
        assert_eq!(ctx, 1_048_576);
        assert_eq!(out, 65_536);
    }

    #[test]
    fn mistral_current_generation_registered() {
        // New dated variants from docs.mistral.ai
        let (ctx, _out) = model_limits("mistral-small-2603");
        assert_eq!(ctx, 262_144);

        let (ctx, _out) = model_limits("mistral-medium-2508");
        assert_eq!(ctx, 131_072);

        let (ctx, out) = model_limits("ministral-14b-2512");
        assert_eq!(ctx, 262_144);
        assert_eq!(out, 32_768);

        let (ctx, _out) = model_limits("devstral-2512");
        assert_eq!(ctx, 262_144);
    }

    #[test]
    fn glm_5_1_and_flashx_registered() {
        let (ctx, out) = model_limits("glm-5.1");
        assert_eq!(ctx, 200_000);
        assert_eq!(out, 131_072);

        let (ctx, _out) = model_limits("glm-4.7-flashx");
        assert_eq!(ctx, 200_000);

        let (ctx, _out) = model_limits("glm-4.6v-flashx");
        assert_eq!(ctx, 131_072);
    }

    #[test]
    fn minimax_m2_7_registered() {
        let (ctx, out) = model_limits("MiniMax-M2.7");
        assert_eq!(ctx, 204_800);
        assert_eq!(out, 196_608);

        let (ctx, _out) = model_limits("MiniMax-M2.7-highspeed");
        assert_eq!(ctx, 204_800);
    }

    #[test]
    fn phi_4_variants_registered() {
        let (ctx, out) = model_limits("phi-4-mini-instruct");
        assert_eq!(ctx, 131_072);
        assert_eq!(out, 4_096);

        let (ctx, out) = model_limits("phi-4-multimodal-instruct");
        assert_eq!(ctx, 131_072);
        assert_eq!(out, 4_096);
    }

    #[test]
    fn command_a_reasoning_registered() {
        let (ctx, out) = model_limits("command-a-reasoning-08-2025");
        assert_eq!(ctx, 256_000);
        assert_eq!(out, 32_768);
    }
}
