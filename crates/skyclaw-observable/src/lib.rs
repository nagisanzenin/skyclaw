//! SkyClaw Observable crate
//!
//! Provides in-process metrics collection ([`MetricsCollector`]) and an
//! optional OpenTelemetry exporter ([`OtelExporter`]) that additionally
//! forwards data to an OTLP endpoint.
//!
//! # Factory
//!
//! Use [`create_observable`] to build the right backend from
//! [`ObservabilityConfig`]:
//!
//! - If `otel_enabled` **and** `otel_endpoint` are set → [`OtelExporter`].
//! - Otherwise → [`MetricsCollector`].

pub mod metrics;
pub mod otel;

pub use metrics::MetricsCollector;
pub use otel::OtelExporter;

use skyclaw_core::traits::Observable;
use skyclaw_core::types::config::ObservabilityConfig;
use skyclaw_core::types::error::SkyclawError;

// ── Predefined metric names ────────────────────────────────────────────

/// Provider call latency in milliseconds.
pub const METRIC_PROVIDER_LATENCY: &str = "skyclaw.provider.latency_ms";

/// Total number of tool executions.
pub const METRIC_TOOL_EXECUTIONS: &str = "skyclaw.tool.executions";

/// Total number of tool execution errors.
pub const METRIC_TOOL_ERRORS: &str = "skyclaw.tool.errors";

/// Total tokens consumed across all providers.
pub const METRIC_TOKENS_USED: &str = "skyclaw.tokens.used";

/// Tasks (agent loops) that ran to completion.
pub const METRIC_TASK_COMPLETIONS: &str = "skyclaw.task.completions";

/// Memory backend operations (store, search, delete, …).
pub const METRIC_MEMORY_OPS: &str = "skyclaw.memory.operations";

// ── Factory ────────────────────────────────────────────────────────────

/// Create an [`Observable`] implementation from the supplied config.
///
/// - If `config.otel_enabled` is `true` **and** `config.otel_endpoint` is
///   `Some(endpoint)`, returns an [`OtelExporter`] targeting that endpoint.
/// - Otherwise returns a plain in-process [`MetricsCollector`].
pub fn create_observable(
    config: &ObservabilityConfig,
) -> Result<Box<dyn Observable>, SkyclawError> {
    if config.otel_enabled {
        if let Some(ref endpoint) = config.otel_endpoint {
            let exporter = OtelExporter::new(endpoint)?;
            tracing::info!(endpoint, "Observable: using OtelExporter");
            return Ok(Box::new(exporter));
        }
        tracing::warn!(
            "otel_enabled is true but otel_endpoint is not set; falling back to in-process metrics"
        );
    }

    tracing::info!("Observable: using in-process MetricsCollector");
    Ok(Box::new(MetricsCollector::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predefined_metric_names() {
        assert_eq!(METRIC_PROVIDER_LATENCY, "skyclaw.provider.latency_ms");
        assert_eq!(METRIC_TOOL_EXECUTIONS, "skyclaw.tool.executions");
        assert_eq!(METRIC_TOOL_ERRORS, "skyclaw.tool.errors");
        assert_eq!(METRIC_TOKENS_USED, "skyclaw.tokens.used");
        assert_eq!(METRIC_TASK_COMPLETIONS, "skyclaw.task.completions");
        assert_eq!(METRIC_MEMORY_OPS, "skyclaw.memory.operations");
    }

    #[test]
    fn factory_default_config_returns_metrics_collector() {
        let config = ObservabilityConfig::default();
        let obs = create_observable(&config);
        assert!(obs.is_ok());
    }

    #[test]
    fn factory_otel_enabled_with_endpoint() {
        let config = ObservabilityConfig {
            log_level: "info".to_string(),
            otel_enabled: true,
            otel_endpoint: Some("http://localhost:4317".to_string()),
        };
        let obs = create_observable(&config);
        assert!(obs.is_ok());
    }

    #[test]
    fn factory_otel_enabled_without_endpoint_falls_back() {
        let config = ObservabilityConfig {
            log_level: "info".to_string(),
            otel_enabled: true,
            otel_endpoint: None,
        };
        let obs = create_observable(&config);
        assert!(obs.is_ok());
    }

    #[tokio::test]
    async fn factory_produced_observable_is_functional() {
        let config = ObservabilityConfig::default();
        let obs = create_observable(&config).unwrap();

        obs.increment_counter("test", &[]).await.unwrap();
        obs.record_metric("gauge", 42.0, &[]).await.unwrap();
        obs.observe_histogram("hist", 1.0, &[]).await.unwrap();

        let health = obs.health_status().await.unwrap();
        assert!(matches!(
            health.status,
            skyclaw_core::traits::HealthState::Healthy
        ));
    }
}
