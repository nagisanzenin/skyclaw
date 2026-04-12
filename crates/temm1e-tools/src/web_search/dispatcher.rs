//! Web search dispatcher — fans out across backends, merges results.

use crate::web_search::backends::SearchBackend;
use crate::web_search::cache::{Cache, CacheKey};
use crate::web_search::governor::Governor;
use crate::web_search::types::*;
use crate::web_search::url_norm;
use reqwest::Url;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;

/// Dispatcher configuration values.
#[derive(Debug, Clone)]
pub struct DispatcherConfig {
    pub default_max_results: usize,
    pub default_max_total_chars: usize,
    pub default_max_snippet_chars: usize,
    pub backend_timeout: Duration,
    pub default_backends: Vec<String>,
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            default_max_results: DEFAULT_MAX_RESULTS,
            default_max_total_chars: DEFAULT_MAX_TOTAL_CHARS,
            default_max_snippet_chars: DEFAULT_MAX_SNIPPET_CHARS,
            backend_timeout: Duration::from_secs(DEFAULT_BACKEND_TIMEOUT_SECS),
            default_backends: vec!["wikipedia".into(), "hackernews".into(), "github".into()],
        }
    }
}

pub struct Dispatcher {
    backends: Vec<Arc<dyn SearchBackend>>,
    governor: Arc<Governor>,
    cache: Arc<Cache>,
    config: DispatcherConfig,
}

impl Dispatcher {
    pub fn new(
        backends: Vec<Arc<dyn SearchBackend>>,
        governor: Arc<Governor>,
        cache: Arc<Cache>,
        config: DispatcherConfig,
    ) -> Self {
        Self {
            backends,
            governor,
            cache,
            config,
        }
    }

    /// Resolve raw user input into a clamped SearchRequest.
    /// Returns the resolved request, optional backend filter, and clamps applied.
    pub fn resolve(&self, raw: RawSearchInput) -> Result<ResolvedInput, String> {
        let mut clamps = Vec::new();

        let query = raw.query.trim().to_string();
        if query.is_empty() {
            return Err("query must be non-empty".into());
        }

        let max_results = clamp_with_log(
            raw.max_results.unwrap_or(self.config.default_max_results),
            MIN_MAX_RESULTS,
            HARD_MAX_RESULTS,
            "max_results",
            &mut clamps,
        );
        let max_total_chars = clamp_with_log(
            raw.max_total_chars
                .unwrap_or(self.config.default_max_total_chars),
            MIN_MAX_TOTAL_CHARS,
            HARD_MAX_TOTAL_CHARS,
            "max_total_chars",
            &mut clamps,
        );
        let max_snippet_chars = clamp_with_log(
            raw.max_snippet_chars
                .unwrap_or(self.config.default_max_snippet_chars),
            MIN_MAX_SNIPPET_CHARS,
            HARD_MAX_SNIPPET_CHARS,
            "max_snippet_chars",
            &mut clamps,
        );

        let time_range = raw
            .time_range
            .as_deref()
            .and_then(TimeRange::parse)
            .unwrap_or(TimeRange::All);
        let sort = raw
            .sort
            .as_deref()
            .and_then(SortOrder::parse)
            .unwrap_or(SortOrder::Relevance);
        let category = raw.category.as_deref().and_then(Category::parse);

        let backends_filter: Option<Vec<String>> = raw
            .backends
            .map(|names| names.into_iter().map(|n| n.to_ascii_lowercase()).collect());

        let req = SearchRequest {
            query,
            max_results,
            max_total_chars,
            max_snippet_chars,
            time_range,
            category,
            language: raw.language,
            region: raw.region,
            include_domains: raw.include_domains.unwrap_or_default(),
            exclude_domains: raw.exclude_domains.unwrap_or_default(),
            sort,
        };

        Ok(ResolvedInput {
            req,
            backends_filter,
            clamps_applied: clamps,
        })
    }

    pub async fn search(&self, raw: RawSearchInput) -> DispatcherOutput {
        let raw_query = raw.query.clone();
        let resolved = match self.resolve(raw) {
            Ok(r) => r,
            Err(msg) => {
                let req = SearchRequest {
                    query: raw_query.clone(),
                    max_results: self.config.default_max_results,
                    max_total_chars: self.config.default_max_total_chars,
                    max_snippet_chars: self.config.default_max_snippet_chars,
                    time_range: TimeRange::All,
                    category: None,
                    language: None,
                    region: None,
                    include_domains: vec![],
                    exclude_domains: vec![],
                    sort: SortOrder::Relevance,
                };
                return DispatcherOutput::input_error(raw_query, req, msg);
            }
        };

        let cache_key = CacheKey::from_request(&resolved.req, &resolved.backends_filter);
        if let Some(cached) = self.cache.get(&cache_key) {
            return cached;
        }

        let selected: Vec<Arc<dyn SearchBackend>> = match &resolved.backends_filter {
            Some(names) => self
                .backends
                .iter()
                .filter(|b| names.contains(&b.name().to_string()) && b.enabled())
                .cloned()
                .collect(),
            None => self.default_set(),
        };

        let catalog = self.catalog();
        let query = resolved.req.query.clone();

        if selected.is_empty() {
            let mut out = DispatcherOutput {
                query: query.clone(),
                req: resolved.req,
                hits: vec![],
                total_candidates_before_truncation: 0,
                backends_succeeded: vec![],
                backends_failed: vec![],
                backends_skipped: vec![],
                catalog,
                clamps_applied: resolved.clamps_applied,
                input_error: None,
            };
            out.backends_failed.push((
                "(none)".into(),
                "no backends enabled or matched filter".into(),
            ));
            return out;
        }

        // Fan-out via JoinSet (tokio-native, panic-safe per task)
        let timeout = self.config.backend_timeout;
        let governor = self.governor.clone();
        let req = resolved.req.clone();
        let mut set: JoinSet<BackendOutcome> = JoinSet::new();
        for b in selected {
            let governor = governor.clone();
            let req = req.clone();
            let backend_name = b.name().to_string();
            let backend_id = b.id();
            let cap = req.per_backend_raw_cap();
            set.spawn(async move {
                if let Err(retry_after) = governor.try_acquire(&backend_name) {
                    return BackendOutcome::Skipped {
                        id: backend_id,
                        name: backend_name,
                        reason: format!("rate limit, retry in {}ms", retry_after),
                    };
                }
                let started = Instant::now();
                let result = tokio::time::timeout(timeout, b.search(&req)).await;
                match result {
                    Ok(Ok(mut hits)) => {
                        if hits.len() > cap {
                            hits.truncate(cap);
                        }
                        BackendOutcome::Ok {
                            id: backend_id,
                            name: backend_name,
                            hits,
                            latency_ms: started.elapsed().as_millis(),
                        }
                    }
                    Ok(Err(e)) => BackendOutcome::Failed {
                        id: backend_id,
                        name: backend_name,
                        error: e.to_string(),
                    },
                    Err(_) => BackendOutcome::Timeout {
                        id: backend_id,
                        name: backend_name,
                    },
                }
            });
        }

        let mut outcomes: Vec<BackendOutcome> = Vec::new();
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(outcome) => outcomes.push(outcome),
                Err(e) => {
                    tracing::warn!("backend task panicked: {e}");
                }
            }
        }

        // Merge into a final DispatcherOutput
        let merged = merge_outcomes(
            query,
            resolved.req,
            outcomes,
            catalog,
            resolved.clamps_applied,
            &self.backends,
        );

        // Cache the result
        self.cache.put(cache_key, merged.clone());
        merged
    }

    fn default_set(&self) -> Vec<Arc<dyn SearchBackend>> {
        let preferred: Vec<&str> = self
            .config
            .default_backends
            .iter()
            .map(|s| s.as_str())
            .collect();
        let mut chosen: Vec<Arc<dyn SearchBackend>> = self
            .backends
            .iter()
            .filter(|b| preferred.contains(&b.name()) && b.enabled())
            .cloned()
            .collect();
        if chosen.is_empty() {
            chosen = self
                .backends
                .iter()
                .filter(|b| b.enabled())
                .take(3)
                .cloned()
                .collect();
        }
        chosen
    }

    pub fn catalog(&self) -> Catalog {
        let mut available = Vec::new();
        let mut disabled_with_hint = Vec::new();
        let mut custom = Vec::new();
        for b in &self.backends {
            if b.enabled() {
                if b.is_custom() {
                    custom.push(b.name().to_string());
                } else {
                    available.push(b.name().to_string());
                }
            } else if let Some(hint) = b.disabled_env_hint() {
                disabled_with_hint.push((b.name().to_string(), hint.to_string()));
            }
        }
        Catalog {
            available,
            disabled_with_hint,
            custom,
        }
    }
}

fn clamp_with_log(
    value: usize,
    min: usize,
    max: usize,
    name: &str,
    clamps: &mut Vec<String>,
) -> usize {
    if value < min {
        clamps.push(format!("{name}={value} → {min}"));
        min
    } else if value > max {
        clamps.push(format!("{name}={value} → {max}"));
        max
    } else {
        value
    }
}

/// Merge backend outcomes into a final DispatcherOutput.
/// Performs URL normalization, dedupe, domain filtering, scoring, sort, truncate.
fn merge_outcomes(
    query: String,
    req: SearchRequest,
    outcomes: Vec<BackendOutcome>,
    catalog: Catalog,
    clamps: Vec<String>,
    all_backends: &[Arc<dyn SearchBackend>],
) -> DispatcherOutput {
    let mut backends_succeeded = Vec::new();
    let mut backends_failed = Vec::new();
    let mut backends_skipped = Vec::new();
    let mut all_hits: Vec<SearchHit> = Vec::new();

    // Build a name → weight lookup
    let weights: HashMap<String, f32> = all_backends
        .iter()
        .map(|b| (b.name().to_string(), b.default_weight()))
        .collect();

    for outcome in outcomes {
        match outcome {
            BackendOutcome::Ok { name, hits, .. } => {
                backends_succeeded.push(name.clone());
                let weight = weights.get(&name).copied().unwrap_or(1.0);
                for mut hit in hits {
                    hit.score *= weight;
                    all_hits.push(hit);
                }
            }
            BackendOutcome::Failed { name, error, .. } => {
                backends_failed.push((name, error));
            }
            BackendOutcome::Skipped { name, reason, .. } => {
                backends_skipped.push((name, reason));
            }
            BackendOutcome::Timeout { name, .. } => {
                backends_failed.push((name, "timeout".into()));
            }
        }
    }

    // Domain filter
    if !req.include_domains.is_empty() || !req.exclude_domains.is_empty() {
        all_hits.retain(|hit| {
            let host = Url::parse(&hit.url)
                .ok()
                .and_then(|u: Url| u.host_str().map(String::from))
                .unwrap_or_default();
            if !req.include_domains.is_empty()
                && !url_norm::host_matches_suffix(&host, &req.include_domains)
            {
                return false;
            }
            if !req.exclude_domains.is_empty()
                && url_norm::host_matches_suffix(&host, &req.exclude_domains)
            {
                return false;
            }
            true
        });
    }

    // Dedupe by normalized URL
    let mut by_norm: HashMap<String, SearchHit> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for hit in all_hits.into_iter() {
        let norm = url_norm::normalize(&hit.url);
        if let Some(existing) = by_norm.get_mut(&norm) {
            // Merge: keep the one with higher score, append other source to also_in
            if hit.score > existing.score {
                let mut new_hit = hit;
                if !new_hit.also_in.contains(&existing.source_name) {
                    new_hit.also_in.push(existing.source_name.clone());
                }
                new_hit.also_in.extend(
                    existing
                        .also_in
                        .iter()
                        .filter(|&n| n != &new_hit.source_name)
                        .cloned(),
                );
                *existing = new_hit;
            } else if !existing.also_in.contains(&hit.source_name)
                && existing.source_name != hit.source_name
            {
                existing.also_in.push(hit.source_name);
            }
        } else {
            order.push(norm.clone());
            by_norm.insert(norm, hit);
        }
    }

    let mut deduped: Vec<SearchHit> = order
        .into_iter()
        .filter_map(|k| by_norm.remove(&k))
        .collect();

    let total_before_truncate = deduped.len();

    // Sort
    match req.sort {
        SortOrder::Relevance | SortOrder::Score => {
            deduped.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        SortOrder::Date => {
            deduped.sort_by(|a, b| match (&b.published, &a.published) {
                (Some(b), Some(a)) => b.cmp(a),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            });
        }
    }

    // Truncate snippets to max_snippet_chars (UTF-8 safe)
    for hit in &mut deduped {
        hit.snippet =
            crate::web_search::backends::truncate_safe(&hit.snippet, req.max_snippet_chars);
    }

    // Truncate to max_results
    if deduped.len() > req.max_results {
        deduped.truncate(req.max_results);
    }

    DispatcherOutput {
        query,
        req,
        hits: deduped,
        total_candidates_before_truncation: total_before_truncate,
        backends_succeeded,
        backends_failed,
        backends_skipped,
        catalog,
        clamps_applied: clamps,
        input_error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct MockBackend {
        id: BackendId,
        name: String,
        enabled: bool,
        weight: f32,
        result: Result<Vec<SearchHit>, BackendError>,
    }

    #[async_trait]
    impl SearchBackend for MockBackend {
        fn id(&self) -> BackendId {
            self.id
        }
        fn name(&self) -> &str {
            &self.name
        }
        fn enabled(&self) -> bool {
            self.enabled
        }
        fn default_weight(&self) -> f32 {
            self.weight
        }
        async fn search(&self, _req: &SearchRequest) -> Result<Vec<SearchHit>, BackendError> {
            self.result.clone()
        }
    }

    fn mock_hit(title: &str, url: &str, source: &str) -> SearchHit {
        SearchHit {
            title: title.into(),
            url: url.into(),
            snippet: format!("snippet for {title}"),
            source: BackendId::Wikipedia,
            source_name: source.into(),
            published: None,
            score: 0.5,
            signal: None,
            also_in: vec![],
        }
    }

    fn make_dispatcher(backends: Vec<Arc<dyn SearchBackend>>) -> Dispatcher {
        Dispatcher::new(
            backends,
            Arc::new(Governor::new(HashMap::new())),
            Arc::new(Cache::new(10, Duration::from_secs(60))),
            DispatcherConfig::default(),
        )
    }

    fn raw_query(q: &str) -> RawSearchInput {
        RawSearchInput {
            query: q.into(),
            ..Default::default()
        }
    }

    // ── resolve tests ───────────────────────────────────────────────────

    #[test]
    fn resolve_uses_defaults_when_unspecified() {
        let d = make_dispatcher(vec![]);
        let r = d.resolve(raw_query("hello")).unwrap();
        assert_eq!(r.req.max_results, DEFAULT_MAX_RESULTS);
        assert_eq!(r.req.max_total_chars, DEFAULT_MAX_TOTAL_CHARS);
        assert_eq!(r.req.max_snippet_chars, DEFAULT_MAX_SNIPPET_CHARS);
        assert_eq!(r.req.time_range, TimeRange::All);
        assert!(r.clamps_applied.is_empty());
    }

    #[test]
    fn resolve_clamps_oversize_max_results() {
        let d = make_dispatcher(vec![]);
        let mut raw = raw_query("hello");
        raw.max_results = Some(1000);
        let r = d.resolve(raw).unwrap();
        assert_eq!(r.req.max_results, HARD_MAX_RESULTS);
        assert!(r.clamps_applied.iter().any(|c| c.contains("max_results")));
    }

    #[test]
    fn resolve_clamps_oversize_max_total_chars() {
        let d = make_dispatcher(vec![]);
        let mut raw = raw_query("hello");
        raw.max_total_chars = Some(999_999);
        let r = d.resolve(raw).unwrap();
        assert_eq!(r.req.max_total_chars, HARD_MAX_TOTAL_CHARS);
    }

    #[test]
    fn resolve_clamps_oversize_max_snippet_chars() {
        let d = make_dispatcher(vec![]);
        let mut raw = raw_query("hello");
        raw.max_snippet_chars = Some(50_000);
        let r = d.resolve(raw).unwrap();
        assert_eq!(r.req.max_snippet_chars, HARD_MAX_SNIPPET_CHARS);
    }

    #[test]
    fn resolve_rejects_empty_query() {
        let d = make_dispatcher(vec![]);
        assert!(d.resolve(raw_query("   ")).is_err());
        assert!(d.resolve(raw_query("")).is_err());
    }

    #[test]
    fn resolve_silently_drops_unknown_backend_names() {
        let d = make_dispatcher(vec![]);
        let mut raw = raw_query("hello");
        raw.backends = Some(vec!["wikipedia".into(), "nonexistent".into()]);
        let r = d.resolve(raw).unwrap();
        // Both end up in the filter; the dispatcher will drop the unknown when matching
        assert!(r.backends_filter.is_some());
    }

    #[test]
    fn resolve_silently_defaults_invalid_time_range() {
        let d = make_dispatcher(vec![]);
        let mut raw = raw_query("hello");
        raw.time_range = Some("never".into());
        let r = d.resolve(raw).unwrap();
        assert_eq!(r.req.time_range, TimeRange::All);
    }

    #[test]
    fn clamp_with_log_under_min_clamps_up() {
        let mut clamps = vec![];
        let v = clamp_with_log(0, 5, 100, "test", &mut clamps);
        assert_eq!(v, 5);
        assert_eq!(clamps.len(), 1);
    }

    #[test]
    fn clamp_with_log_within_range_no_log() {
        let mut clamps = vec![];
        let v = clamp_with_log(50, 5, 100, "test", &mut clamps);
        assert_eq!(v, 50);
        assert!(clamps.is_empty());
    }

    // ── dispatcher tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn dispatcher_partial_success_returns_ok() {
        let backends: Vec<Arc<dyn SearchBackend>> = vec![
            Arc::new(MockBackend {
                id: BackendId::Wikipedia,
                name: "wikipedia".into(),
                enabled: true,
                weight: 1.0,
                result: Ok(vec![mock_hit("ok", "https://example.com/1", "wikipedia")]),
            }),
            Arc::new(MockBackend {
                id: BackendId::HackerNews,
                name: "hackernews".into(),
                enabled: true,
                weight: 1.0,
                result: Err(BackendError::Network("simulated".into())),
            }),
        ];
        let config = DispatcherConfig {
            default_backends: vec!["wikipedia".into(), "hackernews".into()],
            ..Default::default()
        };
        let d = Dispatcher::new(
            backends,
            Arc::new(Governor::new(HashMap::new())),
            Arc::new(Cache::new(10, Duration::from_secs(60))),
            config,
        );
        let out = d.search(raw_query("test")).await;
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.backends_succeeded.len(), 1);
        assert_eq!(out.backends_failed.len(), 1);
    }

    #[tokio::test]
    async fn dispatcher_filters_by_requested_backends() {
        let backends: Vec<Arc<dyn SearchBackend>> = vec![
            Arc::new(MockBackend {
                id: BackendId::Wikipedia,
                name: "wikipedia".into(),
                enabled: true,
                weight: 1.0,
                result: Ok(vec![mock_hit("wiki", "https://example.com/w", "wikipedia")]),
            }),
            Arc::new(MockBackend {
                id: BackendId::HackerNews,
                name: "hackernews".into(),
                enabled: true,
                weight: 1.0,
                result: Ok(vec![mock_hit("hn", "https://example.com/h", "hackernews")]),
            }),
        ];
        let d = make_dispatcher(backends);
        let mut raw = raw_query("test");
        raw.backends = Some(vec!["wikipedia".into()]);
        let out = d.search(raw).await;
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].source_name, "wikipedia");
    }

    #[tokio::test]
    async fn dispatcher_skips_disabled_backends() {
        let backends: Vec<Arc<dyn SearchBackend>> = vec![
            Arc::new(MockBackend {
                id: BackendId::Wikipedia,
                name: "wikipedia".into(),
                enabled: true,
                weight: 1.0,
                result: Ok(vec![mock_hit("ok", "https://example.com", "wikipedia")]),
            }),
            Arc::new(MockBackend {
                id: BackendId::Exa,
                name: "exa".into(),
                enabled: false,
                weight: 1.0,
                result: Err(BackendError::Disabled),
            }),
        ];
        let config = DispatcherConfig {
            default_backends: vec!["wikipedia".into(), "exa".into()],
            ..Default::default()
        };
        let d = Dispatcher::new(
            backends,
            Arc::new(Governor::new(HashMap::new())),
            Arc::new(Cache::new(10, Duration::from_secs(60))),
            config,
        );
        let out = d.search(raw_query("test")).await;
        // exa is disabled so only wikipedia runs
        assert_eq!(out.backends_succeeded, vec!["wikipedia".to_string()]);
    }

    #[tokio::test]
    async fn dispatcher_dedupes_by_url() {
        // Two backends return the same URL — should be merged into one hit with also_in
        let backends: Vec<Arc<dyn SearchBackend>> = vec![
            Arc::new(MockBackend {
                id: BackendId::Wikipedia,
                name: "wikipedia".into(),
                enabled: true,
                weight: 1.0,
                result: Ok(vec![mock_hit(
                    "dup",
                    "https://example.com/dup",
                    "wikipedia",
                )]),
            }),
            Arc::new(MockBackend {
                id: BackendId::HackerNews,
                name: "hackernews".into(),
                enabled: true,
                weight: 1.0,
                result: Ok(vec![mock_hit(
                    "dup",
                    "https://example.com/dup",
                    "hackernews",
                )]),
            }),
        ];
        let config = DispatcherConfig {
            default_backends: vec!["wikipedia".into(), "hackernews".into()],
            ..Default::default()
        };
        let d = Dispatcher::new(
            backends,
            Arc::new(Governor::new(HashMap::new())),
            Arc::new(Cache::new(10, Duration::from_secs(60))),
            config,
        );
        let out = d.search(raw_query("test")).await;
        assert_eq!(out.hits.len(), 1);
        assert!(!out.hits[0].also_in.is_empty());
    }

    #[tokio::test]
    async fn dispatcher_caps_per_backend_raw_hits() {
        // Backend returns 100 hits, but per-backend cap = max_results × 2 = 20
        let mut hits = Vec::new();
        for i in 0..100 {
            hits.push(mock_hit(
                &format!("h{i}"),
                &format!("https://example.com/{i}"),
                "wikipedia",
            ));
        }
        let backends: Vec<Arc<dyn SearchBackend>> = vec![Arc::new(MockBackend {
            id: BackendId::Wikipedia,
            name: "wikipedia".into(),
            enabled: true,
            weight: 1.0,
            result: Ok(hits),
        })];
        let d = make_dispatcher(backends);
        let mut raw = raw_query("test");
        raw.max_results = Some(10);
        let out = d.search(raw).await;
        // After per-backend cap (20) and merge truncate (10) → 10 hits
        assert_eq!(out.hits.len(), 10);
        assert_eq!(out.total_candidates_before_truncation, 20);
    }

    #[tokio::test]
    async fn dispatcher_default_set_falls_back_when_preferred_disabled() {
        let backends: Vec<Arc<dyn SearchBackend>> = vec![Arc::new(MockBackend {
            id: BackendId::Pubmed,
            name: "pubmed".into(),
            enabled: true,
            weight: 1.0,
            result: Ok(vec![mock_hit("p", "https://example.com/p", "pubmed")]),
        })];
        // default_backends preference doesn't include pubmed, but it's the only enabled one
        let config = DispatcherConfig {
            default_backends: vec!["wikipedia".into()], // not present
            ..Default::default()
        };
        let d = Dispatcher::new(
            backends,
            Arc::new(Governor::new(HashMap::new())),
            Arc::new(Cache::new(10, Duration::from_secs(60))),
            config,
        );
        let out = d.search(raw_query("test")).await;
        assert_eq!(out.hits.len(), 1);
    }

    #[tokio::test]
    async fn dispatcher_catalog_partitions_correctly() {
        let backends: Vec<Arc<dyn SearchBackend>> = vec![
            Arc::new(MockBackend {
                id: BackendId::Wikipedia,
                name: "wikipedia".into(),
                enabled: true,
                weight: 1.0,
                result: Ok(vec![]),
            }),
            Arc::new(MockBackend {
                id: BackendId::Exa,
                name: "exa".into(),
                enabled: false,
                weight: 1.0,
                result: Err(BackendError::Disabled),
            }),
        ];
        let d = make_dispatcher(backends);
        let cat = d.catalog();
        assert!(cat.available.contains(&"wikipedia".to_string()));
        // exa is disabled but no env hint set in mock — won't show in disabled_with_hint
    }

    #[tokio::test]
    async fn dispatcher_input_error_for_empty_query() {
        let d = make_dispatcher(vec![]);
        let out = d.search(raw_query("")).await;
        assert!(out.input_error.is_some());
    }
}
