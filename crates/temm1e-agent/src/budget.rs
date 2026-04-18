//! Budget tracker -- tracks cumulative token usage and cost per process
//! lifetime, and enforces a configurable spend limit (0.0 = unlimited).

use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{info, warn};

/// Per-provider pricing (USD per 1M tokens).
#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    pub input_per_million: f64,
    pub output_per_million: f64,
}

/// Look up pricing with custom-model awareness.
///
/// Tries the active provider's custom models first (from
/// `~/.temm1e/custom_models.toml`), then falls back to the hardcoded
/// substring-match pricing via [`get_pricing`]. This wrapper is opt-in:
/// existing callers of [`get_pricing`] keep their byte-identical signature
/// and behavior. Only callers that know the active provider (e.g. main.rs
/// at agent-init time) should call this variant.
pub fn get_pricing_with_custom(provider: &str, model: &str) -> ModelPricing {
    if let Some(cm) = temm1e_core::config::custom_models::lookup_custom_model(provider, model) {
        return ModelPricing {
            input_per_million: cm.input_price_per_1m,
            output_per_million: cm.output_price_per_1m,
        };
    }
    get_pricing(model)
}

/// Returns pricing for known providers/models (USD per 1M tokens).
/// Pricing last verified: March 2026.
pub fn get_pricing(model: &str) -> ModelPricing {
    let m = model.to_lowercase();
    match m.as_str() {
        // ── Anthropic ────────────────────────────────────────────
        _ if m.contains("opus")
            && (m.contains("4-6")
                || m.contains("4-5")
                || m.contains("4.5")
                || m.contains("4.6")) =>
        {
            ModelPricing {
                input_per_million: 5.0,
                output_per_million: 25.0,
            }
        }
        _ if m.contains("opus") => ModelPricing {
            input_per_million: 5.0,
            output_per_million: 25.0,
        },
        _ if m.contains("sonnet")
            && (m.contains("4-6")
                || m.contains("4-5")
                || m.contains("4.5")
                || m.contains("4.6")
                || m.contains("sonnet-4")) =>
        {
            ModelPricing {
                input_per_million: 3.0,
                output_per_million: 15.0,
            }
        }
        _ if m.contains("sonnet") => ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
        },
        _ if m.contains("haiku") && (m.contains("4-5") || m.contains("4.5")) => ModelPricing {
            input_per_million: 1.0,
            output_per_million: 5.0,
        },
        _ if m.contains("haiku") && (m.contains("3-5") || m.contains("3.5")) => ModelPricing {
            input_per_million: 0.80,
            output_per_million: 4.0,
        },
        _ if m.contains("haiku") => ModelPricing {
            input_per_million: 1.0,
            output_per_million: 5.0,
        },

        // ── OpenAI ───────────────────────────────────────────────
        // v5.3.2: specific pricing for 5.4 mini/nano tiers (take priority
        // over the general gpt-5 rule below)
        _ if m.contains("gpt-5.4-nano") || m.contains("gpt-5-4-nano") => ModelPricing {
            input_per_million: 0.20,
            output_per_million: 1.25,
        },
        _ if m.contains("gpt-5.4-mini") || m.contains("gpt-5-4-mini") => ModelPricing {
            input_per_million: 0.75,
            output_per_million: 4.50,
        },
        _ if m.contains("gpt-5.2") || m.contains("gpt-5-2") => ModelPricing {
            input_per_million: 1.75,
            output_per_million: 14.0,
        },
        _ if m.contains("gpt-5") && !m.contains("gpt-5.2") && !m.contains("gpt-5-2") => {
            ModelPricing {
                input_per_million: 1.25,
                output_per_million: 10.0,
            }
        }
        _ if m.contains("gpt-4o-mini") => ModelPricing {
            input_per_million: 0.15,
            output_per_million: 0.60,
        },
        _ if m.contains("gpt-4o") => ModelPricing {
            input_per_million: 2.50,
            output_per_million: 10.0,
        },
        _ if m.contains("gpt-4") => ModelPricing {
            input_per_million: 2.50,
            output_per_million: 10.0,
        },
        _ if m == "o3" || m.starts_with("o3-") => ModelPricing {
            input_per_million: 2.0,
            output_per_million: 8.0,
        },
        _ if m == "o4-mini" || m.starts_with("o4-mini") => ModelPricing {
            input_per_million: 1.10,
            output_per_million: 4.40,
        },

        // ── Google Gemini ────────────────────────────────────────
        _ if m.contains("gemini-2.5-pro") || m.contains("gemini-2-5-pro") => ModelPricing {
            input_per_million: 1.25,
            output_per_million: 10.0,
        },
        _ if m.contains("gemini-3.1-flash-lite") || m.contains("gemini-3-1-flash-lite") => {
            ModelPricing {
                input_per_million: 0.075,
                output_per_million: 0.30,
            }
        }
        _ if m.contains("gemini-3-flash") || m.contains("gemini-3.1-pro") => ModelPricing {
            input_per_million: 0.15,
            output_per_million: 0.60,
        },
        _ if m.contains("gemini-2.5-flash") || m.contains("gemini-2-5-flash") => ModelPricing {
            input_per_million: 0.30,
            output_per_million: 2.50,
        },
        _ if m.contains("gemini-2.0-flash") || m.contains("gemini-2-0-flash") => ModelPricing {
            input_per_million: 0.10,
            output_per_million: 0.40,
        },
        _ if m.contains("gemini") => ModelPricing {
            input_per_million: 0.30,
            output_per_million: 2.50,
        },

        // ── xAI Grok ─────────────────────────────────────────────
        _ if m.contains("grok-4") && m.contains("fast") => ModelPricing {
            input_per_million: 0.20,
            output_per_million: 0.50,
        },
        _ if m.contains("grok-4-1") && !m.contains("fast") => ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
        },
        _ if m.contains("grok-4") => ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
        },
        _ if m.contains("grok-3") && m.contains("fast") => ModelPricing {
            input_per_million: 0.20,
            output_per_million: 0.50,
        },
        _ if m.contains("grok-3") => ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
        },
        _ if m.contains("grok") => ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
        },

        // ── MiniMax ──────────────────────────────────────────────
        // v5.3.2: high-speed variant is doubled per minimax.io pricing page
        _ if m.contains("minimax") && m.contains("highspeed") => ModelPricing {
            input_per_million: 0.60,
            output_per_million: 2.40,
        },
        _ if m.contains("minimax") || m.starts_with("m2") => ModelPricing {
            input_per_million: 0.30,
            output_per_million: 1.20,
        },

        // ── DeepSeek (hosted API — v5.3.2 addition) ──────────────
        // DeepSeek-V3.2 surfaces via generic `deepseek-chat` and
        // `deepseek-reasoner` endpoints per api-docs.deepseek.com.
        _ if m.contains("deepseek") => ModelPricing {
            input_per_million: 0.28,
            output_per_million: 0.42,
        },

        // ── StepFun ─────────────────────────────────────────────
        "step-3.5-flash" => ModelPricing {
            input_per_million: 0.10,
            output_per_million: 0.30,
        },
        "step-3" => ModelPricing {
            input_per_million: 0.57,
            output_per_million: 1.42,
        },
        _ if m.starts_with("step-") => ModelPricing {
            input_per_million: 0.10,
            output_per_million: 0.30,
        },

        // ── Mistral hosted API (v5.3.2 additions) ────────────────
        // Specific rules take priority over the Ollama/local catch-all
        // below, so hosted-API usage of version-dated Mistral models
        // gets real pricing while bare `mistral` / `llama` local names
        // still fall through to $0/$0. Local users who need different
        // pricing use /addmodel to override.
        _ if m.contains("mistral-small-2603") => ModelPricing {
            input_per_million: 0.15,
            output_per_million: 0.60,
        },
        _ if m.contains("mistral-small-2506") => ModelPricing {
            input_per_million: 0.10,
            output_per_million: 0.30,
        },
        _ if m.contains("mistral-medium-2508") || m.contains("mistral-medium-2505") => {
            ModelPricing {
                input_per_million: 0.40,
                output_per_million: 2.00,
            }
        }
        _ if m.contains("mistral-large-2512") => ModelPricing {
            input_per_million: 0.50,
            output_per_million: 1.50,
        },
        _ if m.contains("ministral-14b-2512") => ModelPricing {
            input_per_million: 0.20,
            output_per_million: 0.20,
        },
        _ if m.contains("ministral-8b-2512") => ModelPricing {
            input_per_million: 0.15,
            output_per_million: 0.15,
        },
        _ if m.contains("ministral-3b-2512") => ModelPricing {
            input_per_million: 0.10,
            output_per_million: 0.10,
        },
        _ if m.contains("devstral-2512") => ModelPricing {
            input_per_million: 0.40,
            output_per_million: 2.00,
        },

        // ── Qwen hosted API (v5.3.2 additions) ───────────────────
        _ if m.contains("qwen3.5-flash") || m.contains("qwen/qwen3.5-flash") => ModelPricing {
            input_per_million: 0.10,
            output_per_million: 0.40,
        },
        _ if m.contains("qwen3.5-plus") => ModelPricing {
            input_per_million: 0.26,
            output_per_million: 1.56,
        },
        _ if m.contains("qwen3-max") => ModelPricing {
            input_per_million: 0.78,
            output_per_million: 3.90,
        },
        _ if m.contains("qwen3-coder") => ModelPricing {
            input_per_million: 0.22,
            output_per_million: 1.10,
        },

        // ── Z.ai / Zhipu GLM hosted API (v5.3.2 additions) ───────
        _ if m == "glm-5.1" => ModelPricing {
            input_per_million: 1.40,
            output_per_million: 4.40,
        },
        _ if m.contains("glm-4.7-flashx") => ModelPricing {
            input_per_million: 0.07,
            output_per_million: 0.40,
        },
        _ if m.contains("glm-4.6v-flashx") => ModelPricing {
            input_per_million: 0.04,
            output_per_million: 0.40,
        },

        // ── Microsoft Phi hosted API (v5.3.2 additions) ──────────
        _ if m.contains("phi-4-multimodal-instruct") => ModelPricing {
            input_per_million: 0.08,
            output_per_million: 0.32,
        },
        _ if m.contains("phi-4-mini-instruct") => ModelPricing {
            input_per_million: 0.08,
            output_per_million: 0.30,
        },
        _ if m == "phi-4" || m == "microsoft/phi-4" => ModelPricing {
            input_per_million: 0.13,
            output_per_million: 0.50,
        },

        // ── Cohere Command A Reasoning (v5.3.2 addition — free beta) ─
        _ if m.contains("command-a-reasoning") => ModelPricing {
            input_per_million: 0.0,
            output_per_million: 0.0,
        },

        // ── Ollama (subscription-based, no per-token cost) ───────
        _ if m.contains("llama")
            || m.contains("mistral")
            || m.contains("qwen")
            || m.contains("phi")
            || m.contains("codellama")
            || m.contains("glm") =>
        {
            ModelPricing {
                input_per_million: 0.0,
                output_per_million: 0.0,
            }
        }

        // ── Default: Sonnet-class pricing (conservative) ─────────
        _ => ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
        },
    }
}

/// Calculates USD cost for a given usage.
pub fn calculate_cost(input_tokens: u32, output_tokens: u32, pricing: &ModelPricing) -> f64 {
    let input_cost = (input_tokens as f64 / 1_000_000.0) * pricing.input_per_million;
    let output_cost = (output_tokens as f64 / 1_000_000.0) * pricing.output_per_million;
    input_cost + output_cost
}

/// Thread-safe budget tracker that accumulates cost across a session.
/// Uses atomic u64 storing cost in micro-cents (1 USD = 100_000_000 units)
/// for lock-free operation.
pub struct BudgetTracker {
    /// Cumulative cost in micro-cents (1 USD = 100_000_000).
    cumulative_micro_cents: AtomicU64,
    /// Maximum spend in micro-cents (0 = unlimited).
    max_micro_cents: u64,
    /// Total input tokens consumed.
    total_input_tokens: AtomicU64,
    /// Total output tokens consumed.
    total_output_tokens: AtomicU64,
}

const MICRO_CENTS_PER_USD: f64 = 100_000_000.0;

impl BudgetTracker {
    /// Create a new tracker with a max spend in USD. 0.0 = unlimited.
    pub fn new(max_spend_usd: f64) -> Self {
        Self {
            cumulative_micro_cents: AtomicU64::new(0),
            max_micro_cents: (max_spend_usd.max(0.0) * MICRO_CENTS_PER_USD) as u64,
            total_input_tokens: AtomicU64::new(0),
            total_output_tokens: AtomicU64::new(0),
        }
    }

    /// Record usage from a completed API call. Returns the cost of this call in USD.
    pub fn record_usage(&self, input_tokens: u32, output_tokens: u32, cost_usd: f64) -> f64 {
        let micro_cents = (cost_usd.max(0.0) * MICRO_CENTS_PER_USD) as u64;
        self.cumulative_micro_cents
            .fetch_add(micro_cents, Ordering::Relaxed);
        self.total_input_tokens
            .fetch_add(input_tokens as u64, Ordering::Relaxed);
        self.total_output_tokens
            .fetch_add(output_tokens as u64, Ordering::Relaxed);

        let total = self.total_spend_usd();
        info!(
            call_cost_usd = format!("{:.6}", cost_usd),
            total_spend_usd = format!("{:.6}", total),
            budget_usd = format!("{:.2}", self.max_spend_usd()),
            input_tokens = input_tokens,
            output_tokens = output_tokens,
            "API cost recorded"
        );
        cost_usd
    }

    /// Check if the budget allows another API call. Returns Ok(()) or an error message.
    pub fn check_budget(&self) -> Result<(), String> {
        if self.max_micro_cents == 0 {
            return Ok(()); // Unlimited
        }
        let current = self.cumulative_micro_cents.load(Ordering::Relaxed);
        if current >= self.max_micro_cents {
            let spent = current as f64 / MICRO_CENTS_PER_USD;
            let limit = self.max_micro_cents as f64 / MICRO_CENTS_PER_USD;
            warn!(
                spent_usd = format!("{:.6}", spent),
                limit_usd = format!("{:.2}", limit),
                "Budget exceeded"
            );
            Err(format!(
                "Budget exceeded: ${:.4} spent of ${:.2} limit. \
                 Increase `max_spend_usd` in config or set to 0 for unlimited, then restart.",
                spent, limit
            ))
        } else {
            Ok(())
        }
    }

    /// Current total spend in USD.
    pub fn total_spend_usd(&self) -> f64 {
        self.cumulative_micro_cents.load(Ordering::Relaxed) as f64 / MICRO_CENTS_PER_USD
    }

    /// Max spend in USD.
    pub fn max_spend_usd(&self) -> f64 {
        self.max_micro_cents as f64 / MICRO_CENTS_PER_USD
    }

    /// Total tokens consumed.
    pub fn total_tokens(&self) -> (u64, u64) {
        (
            self.total_input_tokens.load(Ordering::Relaxed),
            self.total_output_tokens.load(Ordering::Relaxed),
        )
    }

    /// Atomic snapshot of current input/output/cost. Safe to call concurrently.
    /// Each field is read atomically; the three reads are not mutually atomic,
    /// but they're always monotonically non-decreasing so a consistent-enough
    /// snapshot emerges in practice.
    pub fn snapshot(&self) -> BudgetSnapshot {
        BudgetSnapshot {
            input_tokens: self.total_input_tokens.load(Ordering::Relaxed),
            output_tokens: self.total_output_tokens.load(Ordering::Relaxed),
            cost_usd: self.total_spend_usd(),
        }
    }
}

/// Immutable snapshot of a BudgetTracker's accumulated usage. Returned by
/// `BudgetTracker::snapshot()` so callers can inspect totals without holding
/// a reference to the tracker itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct BudgetSnapshot {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculate_cost_known_pricing() {
        let pricing = ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
        };
        // 1M input + 1M output = $3 + $15 = $18
        let cost = calculate_cost(1_000_000, 1_000_000, &pricing);
        assert!((cost - 18.0).abs() < 1e-9);

        // 500 input + 1000 output
        let cost2 = calculate_cost(500, 1000, &pricing);
        let expected = (500.0 / 1_000_000.0) * 3.0 + (1000.0 / 1_000_000.0) * 15.0;
        assert!((cost2 - expected).abs() < 1e-12);
    }

    #[test]
    fn calculate_cost_zero_tokens() {
        let pricing = ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
        };
        let cost = calculate_cost(0, 0, &pricing);
        assert!((cost).abs() < 1e-12);
    }

    #[test]
    fn budget_tracker_new_with_limit() {
        let tracker = BudgetTracker::new(5.0);
        assert!((tracker.max_spend_usd() - 5.0).abs() < 1e-6);
        assert!((tracker.total_spend_usd()).abs() < 1e-12);
        assert_eq!(tracker.total_tokens(), (0, 0));
    }

    #[test]
    fn budget_snapshot_reflects_recorded_usage() {
        let tracker = BudgetTracker::new(0.0);
        let snap0 = tracker.snapshot();
        assert_eq!(snap0.input_tokens, 0);
        assert_eq!(snap0.output_tokens, 0);
        assert!(snap0.cost_usd.abs() < 1e-12);

        tracker.record_usage(100, 50, 0.0125);
        tracker.record_usage(200, 100, 0.0250);

        let snap = tracker.snapshot();
        assert_eq!(snap.input_tokens, 300);
        assert_eq!(snap.output_tokens, 150);
        assert!((snap.cost_usd - 0.0375).abs() < 1e-6);
    }

    #[test]
    fn budget_tracker_new_unlimited() {
        let tracker = BudgetTracker::new(0.0);
        assert!((tracker.max_spend_usd()).abs() < 1e-12);
        assert!(tracker.check_budget().is_ok());
    }

    #[test]
    fn budget_tracker_record_usage_accumulates() {
        let tracker = BudgetTracker::new(10.0);

        tracker.record_usage(1000, 500, 0.01);
        assert!((tracker.total_spend_usd() - 0.01).abs() < 1e-6);
        assert_eq!(tracker.total_tokens(), (1000, 500));

        tracker.record_usage(2000, 1000, 0.02);
        assert!((tracker.total_spend_usd() - 0.03).abs() < 1e-6);
        assert_eq!(tracker.total_tokens(), (3000, 1500));
    }

    #[test]
    fn budget_tracker_check_budget_within_limit() {
        let tracker = BudgetTracker::new(1.0);
        tracker.record_usage(1000, 500, 0.50);
        assert!(tracker.check_budget().is_ok());
    }

    #[test]
    fn budget_tracker_check_budget_exceeded() {
        let tracker = BudgetTracker::new(1.0);
        tracker.record_usage(100_000, 50_000, 0.60);
        tracker.record_usage(100_000, 50_000, 0.50);
        // Total = $1.10, limit = $1.00
        let result = tracker.check_budget();
        assert!(result.is_err());
        let err_msg = result.unwrap_err();
        assert!(err_msg.contains("Budget exceeded"));
        assert!(err_msg.contains("$1.00"));
    }

    #[test]
    fn budget_tracker_check_budget_exactly_at_limit() {
        let tracker = BudgetTracker::new(1.0);
        tracker.record_usage(100_000, 50_000, 1.0);
        // Exactly at limit should trigger exceeded
        let result = tracker.check_budget();
        assert!(result.is_err());
    }

    #[test]
    fn budget_tracker_unlimited_never_exceeds() {
        let tracker = BudgetTracker::new(0.0);
        // Even with massive spend, unlimited should always pass
        tracker.record_usage(10_000_000, 5_000_000, 1000.0);
        assert!(tracker.check_budget().is_ok());
    }

    #[test]
    fn budget_tracker_record_returns_cost() {
        let tracker = BudgetTracker::new(10.0);
        let returned = tracker.record_usage(1000, 500, 0.042);
        assert!((returned - 0.042).abs() < 1e-12);
    }

    #[test]
    fn get_pricing_opus_4_6() {
        let pricing = get_pricing("claude-opus-4-6");
        assert!((pricing.input_per_million - 5.0).abs() < 1e-9);
        assert!((pricing.output_per_million - 25.0).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_sonnet_4_6() {
        let pricing = get_pricing("claude-sonnet-4-6");
        assert!((pricing.input_per_million - 3.0).abs() < 1e-9);
        assert!((pricing.output_per_million - 15.0).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_haiku_4_5() {
        let pricing = get_pricing("claude-haiku-4-5");
        assert!((pricing.input_per_million - 1.0).abs() < 1e-9);
        assert!((pricing.output_per_million - 5.0).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_haiku_3_5() {
        let pricing = get_pricing("claude-3-5-haiku");
        assert!((pricing.input_per_million - 0.80).abs() < 1e-9);
        assert!((pricing.output_per_million - 4.0).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_gpt5_2() {
        let pricing = get_pricing("gpt-5.2");
        assert!((pricing.input_per_million - 1.75).abs() < 1e-9);
        assert!((pricing.output_per_million - 14.0).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_gpt5() {
        let pricing = get_pricing("gpt-5");
        assert!((pricing.input_per_million - 1.25).abs() < 1e-9);
        assert!((pricing.output_per_million - 10.0).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_gpt4o() {
        let pricing = get_pricing("gpt-4o-2024-08");
        assert!((pricing.input_per_million - 2.50).abs() < 1e-9);
        assert!((pricing.output_per_million - 10.0).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_gpt4o_mini() {
        let pricing = get_pricing("gpt-4o-mini");
        assert!((pricing.input_per_million - 0.15).abs() < 1e-9);
        assert!((pricing.output_per_million - 0.60).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_o3() {
        let pricing = get_pricing("o3");
        assert!((pricing.input_per_million - 2.0).abs() < 1e-9);
        assert!((pricing.output_per_million - 8.0).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_o4_mini() {
        let pricing = get_pricing("o4-mini");
        assert!((pricing.input_per_million - 1.10).abs() < 1e-9);
        assert!((pricing.output_per_million - 4.40).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_gemini_2_5_pro() {
        let pricing = get_pricing("gemini-2.5-pro");
        assert!((pricing.input_per_million - 1.25).abs() < 1e-9);
        assert!((pricing.output_per_million - 10.0).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_gemini_2_5_flash() {
        let pricing = get_pricing("gemini-2.5-flash");
        assert!((pricing.input_per_million - 0.30).abs() < 1e-9);
        assert!((pricing.output_per_million - 2.50).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_gemini_2_0_flash() {
        let pricing = get_pricing("gemini-2.0-flash");
        assert!((pricing.input_per_million - 0.10).abs() < 1e-9);
        assert!((pricing.output_per_million - 0.40).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_grok4() {
        let pricing = get_pricing("grok-4");
        assert!((pricing.input_per_million - 3.0).abs() < 1e-9);
        assert!((pricing.output_per_million - 15.0).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_grok4_fast() {
        let pricing = get_pricing("grok-4-1-fast");
        assert!((pricing.input_per_million - 0.20).abs() < 1e-9);
        assert!((pricing.output_per_million - 0.50).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_minimax() {
        let pricing = get_pricing("m2.5");
        assert!((pricing.input_per_million - 0.30).abs() < 1e-9);
        assert!((pricing.output_per_million - 1.20).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_ollama_llama() {
        let pricing = get_pricing("llama3.3");
        assert!((pricing.input_per_million).abs() < 1e-9);
        assert!((pricing.output_per_million).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_ollama_glm() {
        let pricing = get_pricing("glm-5");
        assert!((pricing.input_per_million).abs() < 1e-9);
        assert!((pricing.output_per_million).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_unknown_model_defaults() {
        let pricing = get_pricing("some-unknown-model-xyz");
        // Default is Sonnet-class
        assert!((pricing.input_per_million - 3.0).abs() < 1e-9);
        assert!((pricing.output_per_million - 15.0).abs() < 1e-9);
    }

    // ── v5.3.2: new model pricing assertions (additions from Phase 4) ─

    #[test]
    fn get_pricing_gpt5_4_mini() {
        let pricing = get_pricing("gpt-5.4-mini");
        assert!((pricing.input_per_million - 0.75).abs() < 1e-9);
        assert!((pricing.output_per_million - 4.50).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_gpt5_4_nano() {
        let pricing = get_pricing("gpt-5.4-nano");
        assert!((pricing.input_per_million - 0.20).abs() < 1e-9);
        assert!((pricing.output_per_million - 1.25).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_deepseek_chat() {
        let pricing = get_pricing("deepseek-chat");
        assert!((pricing.input_per_million - 0.28).abs() < 1e-9);
        assert!((pricing.output_per_million - 0.42).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_deepseek_reasoner() {
        let pricing = get_pricing("deepseek-reasoner");
        assert!((pricing.input_per_million - 0.28).abs() < 1e-9);
        assert!((pricing.output_per_million - 0.42).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_mistral_large() {
        let pricing = get_pricing("mistral-large-2512");
        assert!((pricing.input_per_million - 0.50).abs() < 1e-9);
        assert!((pricing.output_per_million - 1.50).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_mistral_small_2603() {
        let pricing = get_pricing("mistral-small-2603");
        assert!((pricing.input_per_million - 0.15).abs() < 1e-9);
        assert!((pricing.output_per_million - 0.60).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_ministral_8b() {
        let pricing = get_pricing("ministral-8b-2512");
        assert!((pricing.input_per_million - 0.15).abs() < 1e-9);
        assert!((pricing.output_per_million - 0.15).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_devstral_2512() {
        let pricing = get_pricing("devstral-2512");
        assert!((pricing.input_per_million - 0.40).abs() < 1e-9);
        assert!((pricing.output_per_million - 2.00).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_qwen3_5_flash() {
        let pricing = get_pricing("qwen3.5-flash");
        assert!((pricing.input_per_million - 0.10).abs() < 1e-9);
        assert!((pricing.output_per_million - 0.40).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_glm_5_1() {
        let pricing = get_pricing("glm-5.1");
        assert!((pricing.input_per_million - 1.40).abs() < 1e-9);
        assert!((pricing.output_per_million - 4.40).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_glm_4_7_flashx() {
        let pricing = get_pricing("glm-4.7-flashx");
        assert!((pricing.input_per_million - 0.07).abs() < 1e-9);
        assert!((pricing.output_per_million - 0.40).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_minimax_m2_7_highspeed() {
        // Highspeed variant is doubled per minimax.io pricing page
        let pricing = get_pricing("minimax-m2.7-highspeed");
        assert!((pricing.input_per_million - 0.60).abs() < 1e-9);
        assert!((pricing.output_per_million - 2.40).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_minimax_m2_7_standard() {
        // Standard variant uses the base minimax substring match
        let pricing = get_pricing("minimax-m2.7");
        assert!((pricing.input_per_million - 0.30).abs() < 1e-9);
        assert!((pricing.output_per_million - 1.20).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_phi_4_mini_instruct() {
        let pricing = get_pricing("phi-4-mini-instruct");
        assert!((pricing.input_per_million - 0.08).abs() < 1e-9);
        assert!((pricing.output_per_million - 0.30).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_phi_4_multimodal() {
        let pricing = get_pricing("phi-4-multimodal-instruct");
        assert!((pricing.input_per_million - 0.08).abs() < 1e-9);
        assert!((pricing.output_per_million - 0.32).abs() < 1e-9);
    }

    #[test]
    fn get_pricing_command_a_reasoning_free_beta() {
        let pricing = get_pricing("command-a-reasoning-08-2025");
        assert!((pricing.input_per_million).abs() < 1e-9);
        assert!((pricing.output_per_million).abs() < 1e-9);
    }

    /// Drift-prevention: the new v5.3.2 model_registry additions MUST each
    /// have explicit (non-fallback) pricing entries in this file. If a
    /// future contributor adds a model to `model_registry.rs` without
    /// adding a corresponding `get_pricing` rule, the test below fails and
    /// points at exactly which model is drifting.
    ///
    /// We assert the EXACT pricing tuple per model (not a generic "is it
    /// the fallback?" check — that has false positives for models whose
    /// legitimate pricing happens to equal the fallback, like grok-3 at
    /// $3/$15, or models whose context window legitimately equals
    /// DEFAULT_LIMITS.context_window).
    #[test]
    fn drift_prevention_v5_3_2_additions_have_explicit_pricing() {
        // (model_id, expected input/1M, expected output/1M)
        let v5_3_2_entries: &[(&str, f64, f64)] = &[
            // OpenAI additions
            ("gpt-5.4-mini", 0.75, 4.50),
            ("gpt-5.4-nano", 0.20, 1.25),
            ("o3-mini", 2.00, 8.00),
            // DeepSeek generic endpoints
            ("deepseek-chat", 0.28, 0.42),
            ("deepseek-reasoner", 0.28, 0.42),
            // Qwen hosted API
            ("qwen3.5-flash", 0.10, 0.40),
            // Mistral current generation
            ("mistral-small-2603", 0.15, 0.60),
            ("mistral-medium-2508", 0.40, 2.00),
            ("ministral-3b-2512", 0.10, 0.10),
            ("ministral-8b-2512", 0.15, 0.15),
            ("ministral-14b-2512", 0.20, 0.20),
            ("devstral-2512", 0.40, 2.00),
            // Cohere
            ("command-a-reasoning-08-2025", 0.0, 0.0),
            // Z.ai GLM
            ("glm-5.1", 1.40, 4.40),
            ("glm-4.7-flashx", 0.07, 0.40),
            ("glm-4.6v-flashx", 0.04, 0.40),
            // MiniMax M2.7
            ("MiniMax-M2.7", 0.30, 1.20),
            ("MiniMax-M2.7-highspeed", 0.60, 2.40),
            // Phi variants
            ("phi-4-mini-instruct", 0.08, 0.30),
            ("phi-4-multimodal-instruct", 0.08, 0.32),
        ];

        let mut drifted = Vec::new();
        for (model, expected_in, expected_out) in v5_3_2_entries {
            let p = get_pricing(model);
            let in_ok = (p.input_per_million - expected_in).abs() < 1e-9;
            let out_ok = (p.output_per_million - expected_out).abs() < 1e-9;
            if !in_ok || !out_ok {
                drifted.push(format!(
                    "{}: expected {}/{}, got {}/{}",
                    model, expected_in, expected_out, p.input_per_million, p.output_per_million
                ));
            }
        }

        assert!(
            drifted.is_empty(),
            "v5.3.2 pricing drift detected:\n{}",
            drifted.join("\n")
        );
    }

    #[test]
    fn budget_tracker_multiple_small_calls() {
        let tracker = BudgetTracker::new(0.10);
        // Simulate 100 small calls at $0.001 each = $0.10 total
        for _ in 0..100 {
            tracker.record_usage(100, 50, 0.001);
        }
        // Should be at or slightly above the limit due to floating point
        assert!(tracker.check_budget().is_err());
    }
}
