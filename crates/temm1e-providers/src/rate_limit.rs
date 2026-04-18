//! Rate-limit retry helper shared by all provider backends.
//!
//! The providers detect HTTP 429 and use this helper to decide how long to
//! wait before retrying. Retry only happens at request-initiation — once a
//! streaming response has begun yielding tokens, 429 becomes a non-retry error
//! (SSE buffer state cannot be safely recovered).

use reqwest::Response;
use std::time::Duration;

/// Maximum number of retry attempts on HTTP 429 before surfacing the error.
/// Total worst-case wait ≈ 0.75s + 1.5s + 3s + 6s ≈ 11s at most, capped by
/// per-attempt max of 30s + jitter.
pub const MAX_RATELIMIT_RETRIES: u32 = 3;

/// Parse the HTTP `retry-after` header into a Duration.
///
/// Supports only the integer-seconds form (`retry-after: 30`). The
/// HTTP-date form is rare in practice for rate limits and we fall back to
/// exponential backoff when it cannot be parsed.
///
/// The returned value is capped at 60 seconds to bound worst-case waits
/// even if a server sends an egregious value.
pub fn parse_retry_after(response: &Response) -> Option<Duration> {
    let header = response.headers().get("retry-after")?;
    let s = header.to_str().ok()?;
    let secs: u64 = s.parse().ok()?;
    Some(Duration::from_secs(secs.min(60)))
}

/// Exponential backoff with deterministic jitter.
///
/// Formula: `min(30s, 1s * 2^attempt)` with ±25% deterministic jitter.
/// `attempt` is 0-indexed for the first retry.
pub fn default_backoff(attempt: u32) -> Duration {
    const MAX_BACKOFF: Duration = Duration::from_secs(30);
    let base_ms = 1000u64.saturating_mul(1u64.checked_shl(attempt).unwrap_or(u64::MAX));
    let capped_ms = base_ms.min(MAX_BACKOFF.as_millis() as u64);
    let jitter_pattern: [i64; 8] = [-25, 15, -10, 20, -5, 25, -20, 10];
    let jitter_pct = jitter_pattern[(attempt as usize) % jitter_pattern.len()];
    let jitter_ms = (capped_ms as i64 * jitter_pct) / 100;
    let final_ms = (capped_ms as i64 + jitter_ms).max(1) as u64;
    Duration::from_millis(final_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_backoff_increases_with_attempt() {
        let a0 = default_backoff(0);
        let a3 = default_backoff(3);
        assert!(
            a3 > a0,
            "backoff should grow with attempt number: a0={a0:?}, a3={a3:?}"
        );
    }

    #[test]
    fn default_backoff_caps_at_max_plus_jitter() {
        // 2^10 = 1024s, capped to 30s, jitter ±25% → worst case ~37.5s
        let d = default_backoff(10);
        assert!(
            d <= Duration::from_millis(37_500),
            "backoff should cap around MAX_BACKOFF + 25% jitter, got {d:?}"
        );
        assert!(
            d >= Duration::from_millis(22_500),
            "backoff should be at least MAX_BACKOFF - 25% jitter, got {d:?}"
        );
    }

    #[test]
    fn default_backoff_is_deterministic() {
        assert_eq!(default_backoff(3), default_backoff(3));
        assert_eq!(default_backoff(7), default_backoff(7));
    }

    #[test]
    fn default_backoff_always_positive() {
        for attempt in 0..20 {
            assert!(
                default_backoff(attempt) >= Duration::from_millis(1),
                "attempt {attempt} produced non-positive backoff"
            );
        }
    }

    #[test]
    fn max_retries_const_is_reasonable() {
        // Sanity check: we use this constant across providers.
        const { assert!(MAX_RATELIMIT_RETRIES >= 1) };
        const { assert!(MAX_RATELIMIT_RETRIES <= 10) };
    }
}
