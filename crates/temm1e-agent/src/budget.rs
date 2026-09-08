//! Budget tracker -- tracks cumulative token usage and cost per process
//! lifetime, and enforces a configurable spend limit (0.0 = unlimited).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::{info, warn};

pub use temm1e_core::types::model_catalog::{CostEstimate, Pricing as ModelPricing};
use temm1e_core::types::{message::Usage, model_catalog};

/// Provider identity determines billing. Subscription routes never inherit API
/// rates, even if a custom model entry contains nominal API-equivalent prices.
pub fn get_pricing_with_custom(provider: &str, model: &str) -> ModelPricing {
    let published = model_catalog::pricing(provider, model);
    if matches!(published, ModelPricing::Subscription) {
        return published;
    }
    if let Some(cm) = temm1e_core::config::custom_models::lookup_custom_model(provider, model) {
        if cm.pricing_verified || cm.input_price_per_1m != 0.0 || cm.output_price_per_1m != 0.0 {
            return ModelPricing::custom(cm.input_price_per_1m, cm.output_price_per_1m);
        }
        // Old custom models often recorded 0/0 merely because prices were
        // omitted. Keep a known tariff or unknown state; never infer free.
        return published;
    }
    published
}

/// Legacy model-only API cannot identify who bills the request. Use
/// `get_pricing_with_custom(provider, model)` for an attributable estimate.
pub fn get_pricing(_model: &str) -> ModelPricing {
    ModelPricing::Unknown
}

/// Compatibility projection of a conservative token estimate. Zero means no
/// attributable USD estimate, not free usage. New accounting uses
/// `BudgetTracker::record_model_usage` and retains the typed result.
pub fn calculate_cost(input_tokens: u32, output_tokens: u32, pricing: &ModelPricing) -> f64 {
    pricing
        .estimate(&Usage {
            totals_reported: Some(true),
            input_tokens,
            output_tokens,
            ..Usage::default()
        })
        .upper_usd()
        .unwrap_or(0.0)
}

/// Thread-safe budget tracker that accumulates cost across a session.
/// Uses atomic u64 storing cost in micro-cents (1 USD = 100_000_000 units)
/// for lock-free operation.
pub struct BudgetTracker {
    /// Optional owning session; local counters remain useful per worker.
    parent: Option<Arc<BudgetTracker>>,
    /// Cumulative cost in micro-cents (1 USD = 100_000_000).
    cumulative_micro_cents: AtomicU64,
    /// Maximum spend in micro-cents (0 = unlimited).
    max_micro_cents: u64,
    valid_limit: bool,
    unpriced_calls: AtomicU64,
    subscription_calls: AtomicU64,
    /// Total input tokens consumed.
    total_input_tokens: AtomicU64,
    /// Total output tokens consumed.
    total_output_tokens: AtomicU64,
}

const MICRO_CENTS_PER_USD: f64 = 100_000_000.0;
fn add_saturating(counter: &AtomicU64, amount: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(amount))
    });
}

impl BudgetTracker {
    /// Create a new tracker with a max spend in USD. 0.0 = unlimited.
    pub fn new(max_spend_usd: f64) -> Self {
        Self {
            parent: None,
            cumulative_micro_cents: AtomicU64::new(0),
            max_micro_cents: ((max_spend_usd.max(0.0) * MICRO_CENTS_PER_USD).ceil()) as u64,
            valid_limit: max_spend_usd.is_finite() && max_spend_usd >= 0.0,
            unpriced_calls: AtomicU64::new(0),
            subscription_calls: AtomicU64::new(0),
            total_input_tokens: AtomicU64::new(0),
            total_output_tokens: AtomicU64::new(0),
        }
    }

    /// A child keeps its own totals and forwards each charge/unknown outcome
    /// once to its owner. Admission also checks the owner's remaining budget.
    pub fn child(parent: Arc<BudgetTracker>) -> Self {
        let mut child = Self::new(0.0);
        child.parent = Some(parent);
        child
    }

    /// A USD cap cannot enforce a subscription quota or an unknown tariff.
    /// Reject before making a call rather than silently treating it as free.
    pub fn check_model_budget(&self, pricing: &ModelPricing) -> Result<(), String> {
        if let Some(parent) = &self.parent {
            parent.check_model_budget(pricing)?;
        }
        if self.max_micro_cents > 0 && !pricing.has_usd_tariff() {
            return Err("USD budget cannot be enforced for this subscription or unknown tariff. Configure verified custom API rates, or use provider quota controls and set max_spend_usd = 0.".into());
        }
        self.check_budget()
    }

    pub fn record_model_usage(&self, usage: &Usage, pricing: &ModelPricing) -> CostEstimate {
        let estimate = pricing.estimate(usage);
        self.record_estimate(usage.input_tokens, usage.output_tokens, &estimate);
        estimate
    }

    pub fn record_estimate(&self, input: u32, output: u32, estimate: &CostEstimate) {
        let invalid = matches!(estimate, CostEstimate::StandardTokens {lower_usd, upper_usd,..} if !lower_usd.is_finite() || !upper_usd.is_finite() || *lower_usd < 0.0 || upper_usd < lower_usd);
        let estimate = if invalid {
            &CostEstimate::Unavailable
        } else {
            estimate
        };
        if let Some(parent) = &self.parent {
            parent.record_estimate(input, output, estimate);
        }
        match estimate {
            CostEstimate::StandardTokens {
                lower_usd,
                upper_usd,
                ..
            } => {
                self.record_usage_local(input, output, *upper_usd);
                info!(
                    lower_usd,
                    upper_usd, "Standard token charge interval; not an invoice"
                );
            }
            CostEstimate::Subscription => {
                add_saturating(&self.subscription_calls, 1);
                add_saturating(&self.total_input_tokens, u64::from(input));
                add_saturating(&self.total_output_tokens, u64::from(output));
                info!(
                    input_tokens = input,
                    output_tokens = output,
                    "Subscription usage recorded; charge and quota unknown"
                );
            }
            CostEstimate::Unavailable => {
                add_saturating(&self.unpriced_calls, 1);
                add_saturating(&self.total_input_tokens, u64::from(input));
                add_saturating(&self.total_output_tokens, u64::from(output));
                warn!(
                    input_tokens = input,
                    output_tokens = output,
                    "Usage recorded without an attributable USD estimate"
                );
            }
        }
    }

    /// Record usage from a completed API call. Returns the cost of this call in USD.
    pub fn record_usage(&self, input_tokens: u32, output_tokens: u32, cost_usd: f64) -> f64 {
        if !cost_usd.is_finite() || cost_usd < 0.0 {
            self.record_estimate(input_tokens, output_tokens, &CostEstimate::Unavailable);
            return 0.0;
        }
        self.record_estimate(
            input_tokens,
            output_tokens,
            &CostEstimate::StandardTokens {
                lower_usd: cost_usd,
                upper_usd: cost_usd,
                source: "legacy scalar estimate".into(),
            },
        );
        cost_usd
    }

    fn record_usage_local(&self, input_tokens: u32, output_tokens: u32, cost_usd: f64) -> f64 {
        let micro_cents = (cost_usd * MICRO_CENTS_PER_USD).ceil() as u64;
        let _ = self.cumulative_micro_cents.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |current| Some(current.saturating_add(micro_cents)),
        );
        add_saturating(&self.total_input_tokens, u64::from(input_tokens));
        add_saturating(&self.total_output_tokens, u64::from(output_tokens));

        let total = self.total_spend_usd();
        info!(
            call_cost_usd = format!("{:.6}", cost_usd),
            total_spend_usd = format!("{:.6}", total),
            budget_usd = format!("{:.2}", self.max_spend_usd()),
            input_tokens = input_tokens,
            output_tokens = output_tokens,
            "Estimated standard token budget recorded"
        );
        cost_usd
    }

    /// Check if the budget allows another API call. Returns Ok(()) or an error message.
    pub fn check_budget(&self) -> Result<(), String> {
        if let Some(parent) = &self.parent {
            parent.check_budget()?;
        }
        if !self.valid_limit {
            return Err("max_spend_usd must be finite and nonnegative; zero disables the USD estimate limit.".into());
        }
        if self.max_micro_cents == 0 {
            return Ok(()); // Unlimited
        }
        if self.unpriced_calls.load(Ordering::Relaxed) > 0 {
            return Err("USD budget cannot be enforced after a call with unknown pricing or missing usage totals.".into());
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

    /// Each field is atomic and monotonic, but this is not a mutually
    /// consistent snapshot during concurrent writes. Join contributing tasks
    /// before using it as a final accounting total.
    pub fn snapshot(&self) -> BudgetSnapshot {
        BudgetSnapshot {
            input_tokens: self.total_input_tokens.load(Ordering::Relaxed),
            output_tokens: self.total_output_tokens.load(Ordering::Relaxed),
            cost_usd: self.total_spend_usd(),
            unpriced_calls: self.unpriced_calls.load(Ordering::Relaxed),
            subscription_calls: self.subscription_calls.load(Ordering::Relaxed),
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
    pub unpriced_calls: u64,
    pub subscription_calls: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculate_cost_known_pricing() {
        let pricing = ModelPricing::custom(3.0, 15.0);
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
        let pricing = ModelPricing::custom(3.0, 15.0);
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
    fn unknown_and_subscription_accounting_never_claims_free_calls() {
        let tracker = BudgetTracker::new(1.0);
        assert!(tracker.check_model_budget(&ModelPricing::Unknown).is_err());
        assert!(tracker
            .check_model_budget(&ModelPricing::Subscription)
            .is_err());
        let usage = Usage {
            totals_reported: Some(true),
            input_tokens: 100,
            output_tokens: 20,
            ..Usage::default()
        };
        tracker.record_model_usage(&usage, &ModelPricing::Unknown);
        tracker.record_model_usage(&usage, &ModelPricing::Subscription);
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.unpriced_calls, 1);
        assert_eq!(snapshot.subscription_calls, 1);
        assert_eq!(tracker.total_tokens(), (200, 40));
        assert_eq!(snapshot.cost_usd, 0.0); // known subtotal, not overall cost
        assert!(tracker.check_budget().is_err());
        assert!(BudgetTracker::new(0.0)
            .check_model_budget(&ModelPricing::Subscription)
            .is_ok());
    }

    #[test]
    fn invalid_numbers_and_counter_overflow_cannot_reset_budget() {
        for limit in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(BudgetTracker::new(limit).check_budget().is_err());
        }
        let tracker = BudgetTracker::new(1.0);
        tracker.record_usage(10, 1, f64::NAN);
        assert_eq!(tracker.snapshot().unpriced_calls, 1);
        assert!(tracker.check_budget().is_err());
        let tracker = BudgetTracker::new(1.0);
        tracker.record_usage(1, 1, 1e20);
        tracker.record_usage(1, 1, 1e20);
        assert_eq!(
            tracker.cumulative_micro_cents.load(Ordering::Relaxed),
            u64::MAX
        );
        assert!(tracker.check_budget().is_err());
        let tiny = BudgetTracker::new(1e-10);
        tiny.record_usage(1, 0, 1e-11);
        assert!(tiny.check_budget().is_err()); // rounds conservatively to ledger precision
    }

    #[test]
    fn missing_totals_blocks_further_limited_spending() {
        let tracker = BudgetTracker::new(1.0);
        let price = ModelPricing::custom(3.0, 15.0);
        assert!(tracker.check_model_budget(&price).is_ok());
        tracker.record_model_usage(&Usage::default(), &price);
        assert!(tracker.check_model_budget(&price).is_err());
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
    #[test]
    fn sibling_workers_share_admission_without_double_charging_local_totals() {
        let parent = Arc::new(BudgetTracker::new(1.0));
        let a = BudgetTracker::child(parent.clone());
        let b = BudgetTracker::child(parent.clone());
        a.record_usage(10, 2, 0.4);
        b.record_usage(20, 3, 0.6);
        assert_eq!(a.snapshot().input_tokens, 10);
        assert_eq!(b.snapshot().input_tokens, 20);
        assert_eq!(parent.snapshot().input_tokens, 30);
        assert_eq!(parent.snapshot().cost_usd, 1.0);
        assert!(a.check_budget().is_err());
        assert!(b.check_budget().is_err());
        let parent = Arc::new(BudgetTracker::new(1.0));
        let child = BudgetTracker::child(parent.clone());
        child.record_estimate(5, 1, &CostEstimate::Unavailable);
        assert_eq!(parent.snapshot().unpriced_calls, 1);
        assert!(parent.check_budget().is_err());
        assert!(child
            .check_model_budget(&ModelPricing::Subscription)
            .is_err());
    }
    #[test]
    fn accounting_counters_saturate_instead_of_reopening_admission() {
        let tracker = BudgetTracker::new(1.0);
        tracker.unpriced_calls.store(u64::MAX, Ordering::Relaxed);
        tracker
            .total_input_tokens
            .store(u64::MAX, Ordering::Relaxed);
        tracker.record_estimate(1, 1, &CostEstimate::Unavailable);
        assert_eq!(tracker.snapshot().unpriced_calls, u64::MAX);
        assert_eq!(tracker.snapshot().input_tokens, u64::MAX);
        assert!(tracker.check_budget().is_err());
    }
}
