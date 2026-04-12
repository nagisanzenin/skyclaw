//! Tiny TTL+capacity cache for web_search results.
//!
//! Manual implementation to avoid pulling the `lru` crate. Insertion-order
//! eviction via `VecDeque<K>` (good enough for our access pattern — we don't
//! need true LRU semantics for a 256-entry, 5-minute-TTL cache).

use crate::web_search::types::{DispatcherOutput, SearchRequest, TimeRange};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct CacheKey {
    pub query: String,
    pub backends_filter: Option<Vec<String>>,
    pub time_range: TimeRange,
    pub max_results: usize,
    pub max_total_chars: usize,
    pub max_snippet_chars: usize,
    pub include_domains: Vec<String>,
    pub exclude_domains: Vec<String>,
    pub category: Option<String>,
    pub language: Option<String>,
    pub region: Option<String>,
}

impl CacheKey {
    pub fn from_request(req: &SearchRequest, backends_filter: &Option<Vec<String>>) -> Self {
        Self {
            query: req.query.clone(),
            backends_filter: backends_filter.clone(),
            time_range: req.time_range,
            max_results: req.max_results,
            max_total_chars: req.max_total_chars,
            max_snippet_chars: req.max_snippet_chars,
            include_domains: req.include_domains.clone(),
            exclude_domains: req.exclude_domains.clone(),
            category: req.category.map(|c| format!("{:?}", c)),
            language: req.language.clone(),
            region: req.region.clone(),
        }
    }
}

struct CacheEntry {
    output: DispatcherOutput,
    inserted_at: Instant,
}

pub struct Cache {
    inner: Mutex<CacheInner>,
    ttl: Duration,
    capacity: usize,
}

struct CacheInner {
    map: HashMap<CacheKey, CacheEntry>,
    order: VecDeque<CacheKey>,
}

impl Cache {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(CacheInner {
                map: HashMap::new(),
                order: VecDeque::new(),
            }),
            ttl,
            capacity,
        }
    }

    pub fn get(&self, key: &CacheKey) -> Option<DispatcherOutput> {
        if self.ttl.is_zero() {
            return None;
        }
        let mut guard = self.inner.lock().expect("cache mutex poisoned");
        let now = Instant::now();
        let expired = guard
            .map
            .get(key)
            .map(|e| now.duration_since(e.inserted_at) > self.ttl)
            .unwrap_or(true);
        if expired {
            // Drop expired entry
            if guard.map.remove(key).is_some() {
                guard.order.retain(|k| k != key);
            }
            return None;
        }
        guard.map.get(key).map(|e| e.output.clone())
    }

    pub fn put(&self, key: CacheKey, output: DispatcherOutput) {
        if self.ttl.is_zero() || self.capacity == 0 {
            return;
        }
        let mut guard = self.inner.lock().expect("cache mutex poisoned");
        // If key exists, update in place
        if guard.map.contains_key(&key) {
            guard.map.insert(
                key.clone(),
                CacheEntry {
                    output,
                    inserted_at: Instant::now(),
                },
            );
            return;
        }
        // Evict if at capacity
        while guard.order.len() >= self.capacity {
            if let Some(oldest) = guard.order.pop_front() {
                guard.map.remove(&oldest);
            } else {
                break;
            }
        }
        guard.order.push_back(key.clone());
        guard.map.insert(
            key,
            CacheEntry {
                output,
                inserted_at: Instant::now(),
            },
        );
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().map.len()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web_search::types::*;

    fn make_output(query: &str) -> DispatcherOutput {
        DispatcherOutput {
            query: query.into(),
            req: SearchRequest {
                query: query.into(),
                max_results: 10,
                max_total_chars: 8000,
                max_snippet_chars: 200,
                time_range: TimeRange::All,
                category: None,
                language: None,
                region: None,
                include_domains: vec![],
                exclude_domains: vec![],
                sort: SortOrder::Relevance,
            },
            hits: vec![],
            total_candidates_before_truncation: 0,
            backends_succeeded: vec![],
            backends_failed: vec![],
            backends_skipped: vec![],
            catalog: Catalog::default(),
            clamps_applied: vec![],
            input_error: None,
        }
    }

    fn make_key(query: &str) -> CacheKey {
        CacheKey {
            query: query.into(),
            backends_filter: None,
            time_range: TimeRange::All,
            max_results: 10,
            max_total_chars: 8000,
            max_snippet_chars: 200,
            include_domains: vec![],
            exclude_domains: vec![],
            category: None,
            language: None,
            region: None,
        }
    }

    #[test]
    fn cache_returns_hit_within_ttl() {
        let c = Cache::new(10, Duration::from_secs(60));
        let key = make_key("test");
        c.put(key.clone(), make_output("test"));
        let got = c.get(&key);
        assert!(got.is_some());
        assert_eq!(got.unwrap().query, "test");
    }

    #[test]
    fn cache_treats_expired_as_miss() {
        let c = Cache::new(10, Duration::from_millis(10));
        let key = make_key("test");
        c.put(key.clone(), make_output("test"));
        std::thread::sleep(Duration::from_millis(20));
        assert!(c.get(&key).is_none());
    }

    #[test]
    fn cache_evicts_at_capacity() {
        let c = Cache::new(3, Duration::from_secs(60));
        for i in 0..5 {
            c.put(make_key(&format!("q{i}")), make_output(&format!("q{i}")));
        }
        assert_eq!(c.len(), 3);
        // q0 and q1 should be evicted (oldest first)
        assert!(c.get(&make_key("q0")).is_none());
        assert!(c.get(&make_key("q1")).is_none());
        assert!(c.get(&make_key("q2")).is_some());
        assert!(c.get(&make_key("q3")).is_some());
        assert!(c.get(&make_key("q4")).is_some());
    }

    #[test]
    fn cache_returns_independent_clones() {
        let c = Cache::new(10, Duration::from_secs(60));
        let key = make_key("test");
        c.put(key.clone(), make_output("test"));
        let mut got = c.get(&key).unwrap();
        got.query = "mutated".into();
        // Should not affect what's in cache
        let again = c.get(&key).unwrap();
        assert_eq!(again.query, "test");
    }

    #[test]
    fn cache_key_distinguishes_backend_filter() {
        let c = Cache::new(10, Duration::from_secs(60));
        let mut k1 = make_key("test");
        let mut k2 = make_key("test");
        k1.backends_filter = Some(vec!["wikipedia".into()]);
        k2.backends_filter = Some(vec!["hackernews".into()]);
        c.put(k1.clone(), make_output("a"));
        c.put(k2.clone(), make_output("b"));
        assert_eq!(c.get(&k1).unwrap().query, "a");
        assert_eq!(c.get(&k2).unwrap().query, "b");
    }

    #[test]
    fn cache_key_distinguishes_size_params() {
        let c = Cache::new(10, Duration::from_secs(60));
        let mut k1 = make_key("test");
        let mut k2 = make_key("test");
        k1.max_results = 10;
        k2.max_results = 20;
        c.put(k1.clone(), make_output("ten"));
        c.put(k2.clone(), make_output("twenty"));
        assert_eq!(c.get(&k1).unwrap().query, "ten");
        assert_eq!(c.get(&k2).unwrap().query, "twenty");
    }

    #[test]
    fn cache_zero_ttl_disables() {
        let c = Cache::new(10, Duration::from_secs(0));
        let key = make_key("test");
        c.put(key.clone(), make_output("test"));
        assert!(c.get(&key).is_none());
    }
}
