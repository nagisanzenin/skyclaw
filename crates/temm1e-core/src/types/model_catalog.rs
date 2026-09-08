//! Provider-scoped, dated model facts. A catalog entry is not proof of account
//! entitlement or adapter support. Unknown models and reseller tariffs stay unknown.
use super::message::Usage;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct CatalogLimits {
    pub context_window: usize,
    pub max_input_tokens: Option<usize>,
    pub max_output_tokens: usize,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct TokenRates {
    pub input: f64,
    pub output: f64,
    pub cache_read: Option<f64>,
    pub cache_write_min: Option<f64>,
    pub cache_write_max: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct LongContextRate {
    /// Inclusive threshold, applied to the whole request (including output).
    pub min_input_tokens: u32,
    pub rates: TokenRates,
}

#[derive(Debug, Deserialize)]
pub struct ModelFact {
    pub provider: String,
    pub model: String,
    pub aliases: Vec<String>,
    pub source: String,
    pub capability_source: Option<String>,
    pub checked: String,
    pub limits: Option<CatalogLimits>,
    pub standard: Option<TokenRates>,
    pub long_context: Option<LongContextRate>,
}

pub static CATALOG: LazyLock<Vec<ModelFact>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("model_catalog.json")).expect("bundled model catalog schema")
});

/// Only documented provider aliases are normalized; arbitrary slash prefixes
/// and substring matches must never inherit another seller's billing rate.
pub fn lookup(provider: &str, model: &str) -> Option<&'static ModelFact> {
    let provider = match provider {
        "google" | "google-gemini" => "gemini",
        "xai" => "grok",
        "zhipu" => "zai",
        other => other,
    };
    CATALOG.iter().find(|row| {
        row.provider == provider
            && (row.model == model || row.aliases.iter().any(|alias| alias == model))
    })
}

#[derive(Debug, Clone, Copy)]
pub enum Pricing {
    Published(&'static ModelFact),
    /// User-specified all-input/output rates. No invented cache discount.
    Custom(TokenRates),
    Subscription,
    Unknown,
}

pub fn pricing(provider: &str, model: &str) -> Pricing {
    if matches!(provider, "openai-codex" | "zai-coding-plan") {
        return Pricing::Subscription;
    }
    match lookup(provider, model) {
        Some(row) if row.standard.is_some() => Pricing::Published(row),
        _ => Pricing::Unknown,
    }
}

impl Pricing {
    pub fn custom(input: f64, output: f64) -> Self {
        if !input.is_finite() || !output.is_finite() || input < 0.0 || output < 0.0 {
            return Self::Unknown;
        }
        Self::Custom(TokenRates {
            input,
            output,
            cache_read: Some(input),
            cache_write_min: Some(input),
            cache_write_max: Some(input),
        })
    }

    pub fn has_usd_tariff(&self) -> bool {
        matches!(self, Self::Published(_) | Self::Custom(_))
    }

    /// Standard token charges only: excludes taxes, hosted tools, storage,
    /// geographic/service-tier premiums and negotiated account rates.
    /// Missing cache measurements produce an interval, never a fabricated hit rate.
    pub fn estimate(&self, usage: &Usage) -> CostEstimate {
        if matches!(self, Self::Subscription) {
            return CostEstimate::Subscription;
        }
        let (rates, source) = match self {
            Self::Published(row) => {
                let rates = row
                    .long_context
                    .as_ref()
                    .filter(|tier| usage.input_tokens >= tier.min_input_tokens)
                    .map(|tier| tier.rates)
                    .or(row.standard);
                match rates {
                    Some(rates) => (rates, row.source.clone()),
                    None => return CostEstimate::Unavailable,
                }
            }
            Self::Custom(rates) => (*rates, "user configuration".into()),
            _ => return CostEstimate::Unavailable,
        };
        if usage.totals_reported != Some(true) {
            return CostEstimate::Unavailable;
        }
        let read = usage.cache_read_tokens.unwrap_or(0);
        let write = usage.cache_write_tokens.unwrap_or(0);
        let Some(known_cached) = read.checked_add(write) else {
            return CostEstimate::Unavailable;
        };
        let Some(remaining) = usage.input_tokens.checked_sub(known_cached) else {
            return CostEstimate::Unavailable;
        };
        // A positive, unsupported cache measurement cannot be priced safely.
        if (read > 0 && rates.cache_read.is_none())
            || (write > 0 && (rates.cache_write_min.is_none() || rates.cache_write_max.is_none()))
        {
            return CostEstimate::Unavailable;
        }
        let mut remaining_min = rates.input;
        let mut remaining_max = rates.input;
        if usage.cache_read_tokens.is_none() {
            if let Some(rate) = rates.cache_read {
                remaining_min = remaining_min.min(rate);
                remaining_max = remaining_max.max(rate);
            }
        }
        if usage.cache_write_tokens.is_none() {
            if let (Some(low), Some(high)) = (rates.cache_write_min, rates.cache_write_max) {
                remaining_min = remaining_min.min(low);
                remaining_max = remaining_max.max(high);
            }
        }
        let base = f64::from(read) * rates.cache_read.unwrap_or(0.0)
            + f64::from(usage.output_tokens) * rates.output;
        let lower_usd = (base
            + f64::from(write) * rates.cache_write_min.unwrap_or(0.0)
            + f64::from(remaining) * remaining_min)
            / 1_000_000.0;
        let upper_usd = (base
            + f64::from(write) * rates.cache_write_max.unwrap_or(0.0)
            + f64::from(remaining) * remaining_max)
            / 1_000_000.0;
        if !lower_usd.is_finite()
            || !upper_usd.is_finite()
            || lower_usd < 0.0
            || upper_usd < lower_usd
        {
            return CostEstimate::Unavailable;
        }
        CostEstimate::StandardTokens {
            lower_usd,
            upper_usd,
            source,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CostEstimate {
    StandardTokens {
        lower_usd: f64,
        upper_usd: f64,
        source: String,
    },
    Subscription,
    Unavailable,
}

impl CostEstimate {
    /// Conservative token estimate for budget accounting, not an invoice.
    pub fn upper_usd(&self) -> Option<f64> {
        match self {
            Self::StandardTokens { upper_usd, .. } => Some(*upper_usd),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u32, output: u32, read: Option<u32>, write: Option<u32>) -> Usage {
        Usage {
            totals_reported: Some(true),
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: read,
            cache_write_tokens: write,
            ..Usage::default()
        }
    }

    #[test]
    fn billing_identity_is_exact_and_subscription_is_not_free() {
        assert!(matches!(
            pricing("openai-codex", "gpt-6-astra"),
            Pricing::Subscription
        ));
        for (provider, model) in [
            ("openrouter", "openai/gpt-6-astra"),
            ("openai", "my-gpt-5.6-sol"),
            ("openai", "gpt-5.6-sol-future"),
            ("ollama", "gpt-5.6-sol"),
            ("unknown", "claude-sonnet-5"),
        ] {
            assert!(matches!(pricing(provider, model), Pricing::Unknown));
        }
        assert!(matches!(
            pricing("openai", "gpt-5.6"),
            Pricing::Published(_)
        ));
    }

    #[test]
    fn cache_and_whole_request_tiers_use_normalized_totals() {
        let p = pricing("grok", "grok-4.6");
        let low = p.estimate(&usage(199_999, 1_000, Some(100_000), Some(0)));
        let high = p.estimate(&usage(200_000, 1_000, Some(100_000), Some(0)));
        assert!((low.upper_usd().unwrap() - 0.255998).abs() < 1e-10);
        assert!((high.upper_usd().unwrap() - 0.512).abs() < 1e-10);
        let p = pricing("openai", "gpt-6-astra");
        assert!(
            (p.estimate(&usage(272_000, 1_000, Some(0), Some(0)))
                .upper_usd()
                .unwrap()
                - 2.77)
                .abs()
                < 1e-10
        );
        assert!(
            (p.estimate(&usage(272_001, 1_000, Some(0), Some(0)))
                .upper_usd()
                .unwrap()
                - 5.51502)
                .abs()
                < 1e-10
        );
    }

    #[test]
    fn missing_cache_and_cache_ttl_are_intervals() {
        let estimate = pricing("anthropic", "claude-sonnet-5")
            .estimate(&usage(1_000_000, 1_000_000, None, None));
        match estimate {
            CostEstimate::StandardTokens {
                lower_usd,
                upper_usd,
                ..
            } => {
                assert_eq!(lower_usd, 10.2);
                assert_eq!(upper_usd, 14.0);
            }
            _ => panic!("expected interval"),
        }
        let p = pricing("openai", "gpt-5.2");
        assert_eq!(p.estimate(&Usage::default()), CostEstimate::Unavailable);
        assert_eq!(
            p.estimate(&usage(10, 0, Some(11), Some(0))),
            CostEstimate::Unavailable
        );
        assert_eq!(
            p.estimate(&usage(10, 0, Some(0), Some(5))),
            CostEstimate::Unavailable
        );
    }

    #[test]
    fn bundled_catalog_is_unique_finite_and_sourced() {
        let mut keys = std::collections::HashSet::new();
        for row in CATALOG.iter() {
            for name in std::iter::once(&row.model).chain(row.aliases.iter()) {
                assert!(
                    keys.insert((&row.provider, name)),
                    "duplicate catalog identity"
                );
            }
            assert!(row.source.starts_with("https://"));
            assert_eq!(row.checked, "2026-09-08");
            for rates in row
                .standard
                .iter()
                .chain(row.long_context.iter().map(|v| &v.rates))
            {
                for value in [
                    Some(rates.input),
                    Some(rates.output),
                    rates.cache_read,
                    rates.cache_write_min,
                    rates.cache_write_max,
                ]
                .into_iter()
                .flatten()
                {
                    assert!(value.is_finite() && value >= 0.0);
                }
                assert_eq!(
                    rates.cache_write_min.is_some(),
                    rates.cache_write_max.is_some()
                );
                assert!(rates.cache_write_min <= rates.cache_write_max);
            }
            if let Some(limits) = row.limits {
                assert!(row.capability_source.is_some());
                assert!(limits.context_window >= limits.max_output_tokens);
            }
        }
    }
}
