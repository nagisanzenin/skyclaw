//! Per-backend rate limit governor.
//!
//! Tracks the last call time per backend and enforces a minimum interval
//! before the next call. Returns the remaining wait time when blocked.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Per-backend min-interval enforcer.
///
/// Uses `std::sync::Mutex` (not `tokio::sync::Mutex`) because the critical
/// section is microseconds — a HashMap insert. No await points held.
pub struct Governor {
    intervals: HashMap<String, Duration>,
    last_call: Mutex<HashMap<String, Instant>>,
}

impl Governor {
    pub fn new(intervals: HashMap<String, Duration>) -> Self {
        Self {
            intervals,
            last_call: Mutex::new(HashMap::new()),
        }
    }

    /// Try to acquire the right to call a backend now.
    /// Returns `Ok(())` on success, `Err(wait_ms)` if rate-limited.
    pub fn try_acquire(&self, backend_name: &str) -> Result<(), u64> {
        let interval = match self.intervals.get(backend_name) {
            Some(i) => *i,
            None => return Ok(()), // no governor for this backend
        };

        let mut guard = self.last_call.lock().expect("governor mutex poisoned");
        let now = Instant::now();

        if let Some(last) = guard.get(backend_name) {
            let elapsed = now.duration_since(*last);
            if elapsed < interval {
                let wait = interval - elapsed;
                return Err(wait.as_millis() as u64);
            }
        }

        guard.insert(backend_name.to_string(), now);
        Ok(())
    }
}

/// Default governor intervals per backend, baked in for safety.
/// All include a small buffer over the documented limit to be polite.
pub fn default_intervals() -> HashMap<String, Duration> {
    let mut m = HashMap::new();
    // No-limit backends — explicit zero-interval entries (not present = no governor)
    // hackernews: Algolia infrastructure, no documented limit
    // wikipedia:  reasonable use, ~200/sec/IP

    m.insert("github".into(), Duration::from_millis(6_600)); // 10/min + 10% buffer
    m.insert("stackoverflow".into(), Duration::from_millis(330)); // ~3/sec + buffer
    m.insert("reddit".into(), Duration::from_millis(6_600)); // 10/min hard
    m.insert("marginalia".into(), Duration::from_millis(1_100)); // shared anonymous
    m.insert("arxiv".into(), Duration::from_millis(3_300)); // ToS 3s + buffer
    m.insert("pubmed".into(), Duration::from_millis(366)); // 3/sec + buffer
    m.insert("duckduckgo".into(), Duration::from_millis(6_600)); // protect Chrome UA
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn governor_no_governor_means_unlimited() {
        let g = Governor::new(HashMap::new());
        for _ in 0..100 {
            assert!(g.try_acquire("anything").is_ok());
        }
    }

    #[test]
    fn governor_blocks_within_interval() {
        let mut intervals = HashMap::new();
        intervals.insert("test".into(), Duration::from_millis(500));
        let g = Governor::new(intervals);

        assert!(g.try_acquire("test").is_ok());
        let result = g.try_acquire("test");
        assert!(result.is_err());
        let wait = result.unwrap_err();
        assert!(
            wait > 0 && wait <= 500,
            "wait should be in (0, 500]: {wait}"
        );
    }

    #[test]
    fn governor_allows_after_interval_elapses() {
        let mut intervals = HashMap::new();
        intervals.insert("test".into(), Duration::from_millis(50));
        let g = Governor::new(intervals);

        assert!(g.try_acquire("test").is_ok());
        thread::sleep(Duration::from_millis(60));
        assert!(g.try_acquire("test").is_ok());
    }

    #[test]
    fn governor_per_backend_isolated() {
        let mut intervals = HashMap::new();
        intervals.insert("a".into(), Duration::from_millis(500));
        intervals.insert("b".into(), Duration::from_millis(500));
        let g = Governor::new(intervals);

        assert!(g.try_acquire("a").is_ok());
        // b should be unaffected by a
        assert!(g.try_acquire("b").is_ok());
    }

    #[test]
    fn governor_returns_retry_after_ms() {
        let mut intervals = HashMap::new();
        intervals.insert("test".into(), Duration::from_secs(60));
        let g = Governor::new(intervals);

        assert!(g.try_acquire("test").is_ok());
        let wait = g.try_acquire("test").unwrap_err();
        // Should be close to but less than 60_000ms
        assert!(wait > 59_000 && wait <= 60_000, "wait was {wait}");
    }

    #[test]
    fn default_intervals_has_known_backends() {
        let intervals = default_intervals();
        assert!(intervals.contains_key("github"));
        assert!(intervals.contains_key("reddit"));
        assert!(intervals.contains_key("arxiv"));
        assert!(!intervals.contains_key("hackernews")); // no governor
        assert!(!intervals.contains_key("wikipedia")); // no governor
    }
}
