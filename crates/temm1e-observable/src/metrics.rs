//! In-process metrics collector.
//!
//! Provides thread-safe counters, gauges, and histograms stored entirely
//! in-process. No external dependencies are needed — data lives in
//! `std::sync::atomic` integers and `std::sync::RwLock`-guarded vectors.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::RwLock;

use async_trait::async_trait;
use temm1e_core::traits::{ComponentHealth, HealthState, HealthStatus, Observable};
use temm1e_core::types::error::Temm1eError;

/// Maximum retained series per metric kind; excess new series return an error.
const MAX_SERIES: usize = 1024;
/// Percentiles describe this recent observation window, not lifetime history.
const HISTOGRAM_WINDOW: usize = 1024;
const MAX_KEY_BYTES: usize = 1024;

/// In-process metrics with bounded series cardinality and recent histograms.
pub struct MetricsCollector {
    counters: RwLock<HashMap<String, AtomicU64>>,
    gauges: RwLock<HashMap<String, AtomicI64>>,
    histograms: RwLock<HashMap<String, VecDeque<f64>>>,
}

impl MetricsCollector {
    /// Create a new, empty metrics collector.
    pub fn new() -> Self {
        Self {
            counters: RwLock::new(HashMap::new()),
            gauges: RwLock::new(HashMap::new()),
            histograms: RwLock::new(HashMap::new()),
        }
    }

    // ── Helpers for label-qualified metric names ────────────────────────

    /// Build a composite key from the metric name and its labels.
    ///
    /// Example: `("latency", &[("provider", "anthropic")])` →
    /// `"latency{provider=anthropic}"`.
    fn qualified_name(name: &str, labels: &[(&str, &str)]) -> String {
        fn escape(value: &str) -> String {
            value
                .replace('%', "%25")
                .replace('{', "%7B")
                .replace('}', "%7D")
                .replace(',', "%2C")
                .replace('=', "%3D")
        }
        let name = escape(name);
        if labels.is_empty() {
            return name;
        }
        let mut labels = labels.to_vec();
        labels.sort_unstable();
        let pairs: Vec<String> = labels
            .iter()
            .map(|(k, v)| format!("{}={}", escape(k), escape(v)))
            .collect();
        format!("{name}{{{}}}", pairs.join(","))
    }

    fn checked_key(name: &str, labels: &[(&str, &str)]) -> Result<String, Temm1eError> {
        if labels.len() > 16
            || name.len().saturating_add(
                labels
                    .iter()
                    .map(|(k, v)| k.len().saturating_add(v.len()))
                    .sum::<usize>(),
            ) > MAX_KEY_BYTES
        {
            return Err(Temm1eError::Internal(
                "metric name/labels exceed bounded capacity".into(),
            ));
        }
        let key = Self::qualified_name(name, labels);
        if key.len() > MAX_KEY_BYTES {
            return Err(Temm1eError::Internal(
                "encoded metric key exceeds bounded capacity".into(),
            ));
        }
        Ok(key)
    }

    fn check_capacity<T>(map: &HashMap<String, T>, key: &str) -> Result<(), Temm1eError> {
        if map.len() >= MAX_SERIES && !map.contains_key(key) {
            return Err(Temm1eError::Internal(
                "metric series capacity reached".into(),
            ));
        }
        Ok(())
    }

    // ── Public read accessors (for tests & OtelExporter) ───────────────

    /// Read the current value of a counter.
    pub fn counter_value(&self, key: &str) -> Option<u64> {
        let map = self.counters.read().unwrap_or_else(|e| e.into_inner());
        map.get(key).map(|v| v.load(Ordering::Relaxed))
    }

    /// Read the current value of a gauge.
    pub fn gauge_value(&self, key: &str) -> Option<i64> {
        let map = self.gauges.read().unwrap_or_else(|e| e.into_inner());
        map.get(key).map(|v| v.load(Ordering::Relaxed))
    }

    /// Read up to the most recent 1,024 histogram observations.
    pub fn histogram_values(&self, key: &str) -> Option<Vec<f64>> {
        let map = self.histograms.read().unwrap_or_else(|e| e.into_inner());
        map.get(key).map(|values| values.iter().copied().collect())
    }

    /// Compute percentiles over the most recent 1,024 observations.
    ///
    /// Returns `None` if the histogram does not exist or has no observations.
    /// `percentile` must be in `[0.0, 100.0]`.
    pub fn histogram_percentile(&self, key: &str, percentile: f64) -> Option<f64> {
        if !(0.0..=100.0).contains(&percentile) {
            return None;
        }
        let map = self.histograms.read().unwrap_or_else(|e| e.into_inner());
        let values = map.get(key)?;
        if values.is_empty() {
            return None;
        }
        let mut sorted: Vec<f64> = values.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = ((percentile / 100.0) * (sorted.len() as f64 - 1.0))
            .round()
            .max(0.0) as usize;
        let idx = idx.min(sorted.len() - 1);
        Some(sorted[idx])
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Observable for MetricsCollector {
    /// Record a gauge metric — stores `value * 1000` as an `i64`.
    async fn record_metric(
        &self,
        name: &str,
        value: f64,
        labels: &[(&str, &str)],
    ) -> Result<(), Temm1eError> {
        let key = Self::checked_key(name, labels)?;
        if !value.is_finite() || (value * 1000.0).abs() >= i64::MAX as f64 {
            return Err(Temm1eError::Internal(
                "gauge value is nonfinite or out of range".into(),
            ));
        }
        let encoded = (value * 1000.0) as i64;

        let mut map = self
            .gauges
            .write()
            .map_err(|e| Temm1eError::Internal(format!("gauges lock poisoned: {e}")))?;

        Self::check_capacity(&map, &key)?;
        map.entry(key)
            .and_modify(|v| v.store(encoded, Ordering::Relaxed))
            .or_insert_with(|| AtomicI64::new(encoded));

        tracing::debug!(metric = name, value, "gauge recorded");
        Ok(())
    }

    /// Increment a counter by 1.
    async fn increment_counter(
        &self,
        name: &str,
        labels: &[(&str, &str)],
    ) -> Result<(), Temm1eError> {
        let key = Self::checked_key(name, labels)?;

        let map = self
            .counters
            .read()
            .map_err(|e| Temm1eError::Internal(format!("counters lock poisoned: {e}")))?;

        if let Some(counter) = map.get(&key) {
            let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_add(1))
            });
            tracing::debug!(metric = name, "counter incremented (existing)");
            return Ok(());
        }
        drop(map);

        let mut map = self
            .counters
            .write()
            .map_err(|e| Temm1eError::Internal(format!("counters lock poisoned: {e}")))?;

        // Double-check after acquiring write lock.
        Self::check_capacity(&map, &key)?;
        map.entry(key)
            .and_modify(|v| {
                let _ = v.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                    Some(n.saturating_add(1))
                });
            })
            .or_insert_with(|| AtomicU64::new(1));

        tracing::debug!(metric = name, "counter incremented");
        Ok(())
    }

    /// Record a histogram observation.
    async fn observe_histogram(
        &self,
        name: &str,
        value: f64,
        labels: &[(&str, &str)],
    ) -> Result<(), Temm1eError> {
        let key = Self::checked_key(name, labels)?;
        if !value.is_finite() {
            return Err(Temm1eError::Internal(
                "histogram value must be finite".into(),
            ));
        }

        let mut map = self
            .histograms
            .write()
            .map_err(|e| Temm1eError::Internal(format!("histograms lock poisoned: {e}")))?;

        Self::check_capacity(&map, &key)?;
        let values = map.entry(key).or_default();
        if values.len() == HISTOGRAM_WINDOW {
            values.pop_front();
        }
        values.push_back(value);

        tracing::debug!(metric = name, value, "histogram observation recorded");
        Ok(())
    }

    /// In-process metrics are always healthy.
    async fn health_status(&self) -> Result<HealthStatus, Temm1eError> {
        Ok(HealthStatus {
            status: HealthState::Healthy,
            components: vec![ComponentHealth {
                name: "metrics_collector".to_string(),
                status: HealthState::Healthy,
                message: Some("In-process metrics operational".to_string()),
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_metrics_retain_recent_values_and_keep_existing_series_updatable() {
        let mc = MetricsCollector::new();
        for i in 0..(HISTOGRAM_WINDOW + 17) {
            mc.observe_histogram("window", i as f64, &[]).await.unwrap();
        }
        let values = mc.histogram_values("window").unwrap();
        assert_eq!(values.len(), HISTOGRAM_WINDOW);
        assert_eq!(values[0], 17.0);
        assert_eq!(mc.histogram_percentile("window", 0.0), Some(17.0));
        for i in 0..MAX_SERIES {
            let name = format!("series_{i}");
            mc.increment_counter(&name, &[]).await.unwrap();
            mc.record_metric(&name, 1.0, &[]).await.unwrap();
            if i + 1 < MAX_SERIES {
                mc.observe_histogram(&name, 1.0, &[]).await.unwrap();
            }
        }
        assert!(mc.increment_counter("overflow", &[]).await.is_err());
        assert!(mc.record_metric("overflow", 1.0, &[]).await.is_err());
        assert!(mc.observe_histogram("overflow", 1.0, &[]).await.is_err());
        mc.increment_counter("series_0", &[]).await.unwrap();
        mc.record_metric("series_0", 2.0, &[]).await.unwrap();
        mc.observe_histogram("window", 3.0, &[]).await.unwrap();
        assert_eq!(mc.counter_value("series_0"), Some(2));
        assert_eq!(mc.gauge_value("series_0"), Some(2000));
    }

    #[tokio::test]
    async fn labels_are_order_independent_unambiguous_and_bounded() {
        let mc = MetricsCollector::new();
        mc.increment_counter("n", &[("z", "2"), ("a", "1")])
            .await
            .unwrap();
        mc.increment_counter("n", &[("a", "1"), ("z", "2")])
            .await
            .unwrap();
        assert_eq!(mc.counter_value("n{a=1,z=2}"), Some(2));
        assert_ne!(
            MetricsCollector::qualified_name("n", &[("a", "1,z=2")]),
            MetricsCollector::qualified_name("n", &[("a", "1"), ("z", "2")])
        );
        assert!(mc
            .increment_counter(&"x".repeat(MAX_KEY_BYTES + 1), &[])
            .await
            .is_err());
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(mc.observe_histogram("bad", invalid, &[]).await.is_err());
            assert!(mc.record_metric("bad", invalid, &[]).await.is_err());
        }
        assert!(mc.histogram_values("bad").is_none());
    }

    #[tokio::test]
    async fn increment_counter_creates_and_increments() {
        let mc = MetricsCollector::new();
        mc.increment_counter("requests", &[]).await.unwrap();
        mc.increment_counter("requests", &[]).await.unwrap();
        mc.increment_counter("requests", &[]).await.unwrap();

        assert_eq!(mc.counter_value("requests"), Some(3));
    }

    #[tokio::test]
    async fn increment_counter_with_labels() {
        let mc = MetricsCollector::new();
        mc.increment_counter("requests", &[("method", "GET")])
            .await
            .unwrap();
        mc.increment_counter("requests", &[("method", "POST")])
            .await
            .unwrap();
        mc.increment_counter("requests", &[("method", "GET")])
            .await
            .unwrap();

        assert_eq!(mc.counter_value("requests{method=GET}"), Some(2));
        assert_eq!(mc.counter_value("requests{method=POST}"), Some(1));
    }

    #[tokio::test]
    async fn counter_missing_returns_none() {
        let mc = MetricsCollector::new();
        assert_eq!(mc.counter_value("nonexistent"), None);
    }

    #[tokio::test]
    async fn record_metric_sets_gauge() {
        let mc = MetricsCollector::new();
        mc.record_metric("cpu_usage", 72.5, &[]).await.unwrap();

        let raw = mc.gauge_value("cpu_usage").unwrap();
        // 72.5 * 1000 = 72500
        assert_eq!(raw, 72500);
    }

    #[tokio::test]
    async fn record_metric_overwrites_gauge() {
        let mc = MetricsCollector::new();
        mc.record_metric("temperature", 20.0, &[]).await.unwrap();
        mc.record_metric("temperature", 25.5, &[]).await.unwrap();

        let raw = mc.gauge_value("temperature").unwrap();
        assert_eq!(raw, 25500);
    }

    #[tokio::test]
    async fn gauge_with_labels() {
        let mc = MetricsCollector::new();
        mc.record_metric("cpu", 50.0, &[("host", "a")])
            .await
            .unwrap();
        mc.record_metric("cpu", 80.0, &[("host", "b")])
            .await
            .unwrap();

        assert_eq!(mc.gauge_value("cpu{host=a}"), Some(50000));
        assert_eq!(mc.gauge_value("cpu{host=b}"), Some(80000));
    }

    #[tokio::test]
    async fn observe_histogram_records_values() {
        let mc = MetricsCollector::new();
        mc.observe_histogram("latency", 100.0, &[]).await.unwrap();
        mc.observe_histogram("latency", 200.0, &[]).await.unwrap();
        mc.observe_histogram("latency", 150.0, &[]).await.unwrap();

        let vals = mc.histogram_values("latency").unwrap();
        assert_eq!(vals, vec![100.0, 200.0, 150.0]);
    }

    #[tokio::test]
    async fn histogram_percentile_p50() {
        let mc = MetricsCollector::new();
        for v in [10.0, 20.0, 30.0, 40.0, 50.0] {
            mc.observe_histogram("lat", v, &[]).await.unwrap();
        }

        let p50 = mc.histogram_percentile("lat", 50.0).unwrap();
        assert!((p50 - 30.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn histogram_percentile_p99() {
        let mc = MetricsCollector::new();
        for v in 1..=100 {
            mc.observe_histogram("lat", v as f64, &[]).await.unwrap();
        }

        let p99 = mc.histogram_percentile("lat", 99.0).unwrap();
        // With 100 values, p99 index ≈ 98 → value 99.0
        assert!((p99 - 99.0).abs() < 1.0);
    }

    #[tokio::test]
    async fn histogram_percentile_empty_returns_none() {
        let mc = MetricsCollector::new();
        assert!(mc.histogram_percentile("nonexistent", 50.0).is_none());
    }

    #[tokio::test]
    async fn health_status_is_healthy() {
        let mc = MetricsCollector::new();
        let status = mc.health_status().await.unwrap();

        assert!(matches!(status.status, HealthState::Healthy));
        assert_eq!(status.components.len(), 1);
        assert_eq!(status.components[0].name, "metrics_collector");
        assert!(matches!(status.components[0].status, HealthState::Healthy));
    }

    #[tokio::test]
    async fn concurrent_counter_increments() {
        use std::sync::Arc;

        let mc = Arc::new(MetricsCollector::new());
        let mut handles = vec![];

        for _ in 0..10 {
            let mc = Arc::clone(&mc);
            handles.push(tokio::spawn(async move {
                for _ in 0..100 {
                    mc.increment_counter("concurrent", &[]).await.unwrap();
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        assert_eq!(mc.counter_value("concurrent"), Some(1000));
    }

    #[tokio::test]
    async fn concurrent_histogram_observations() {
        use std::sync::Arc;

        let mc = Arc::new(MetricsCollector::new());
        let mut handles = vec![];

        for i in 0..5 {
            let mc = Arc::clone(&mc);
            handles.push(tokio::spawn(async move {
                for j in 0..20 {
                    mc.observe_histogram("conc_hist", (i * 20 + j) as f64, &[])
                        .await
                        .unwrap();
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let vals = mc.histogram_values("conc_hist").unwrap();
        assert_eq!(vals.len(), 100);
    }

    #[tokio::test]
    async fn qualified_name_no_labels() {
        let key = MetricsCollector::qualified_name("metric", &[]);
        assert_eq!(key, "metric");
    }

    #[tokio::test]
    async fn qualified_name_with_labels() {
        let key = MetricsCollector::qualified_name("metric", &[("env", "prod"), ("region", "us")]);
        assert_eq!(key, "metric{env=prod,region=us}");
    }
}
