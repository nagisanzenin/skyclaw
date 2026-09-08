//! HTTP 429 retry policy. Retry-After is a minimum, never shortened.
use rand::Rng;
use reqwest::Response;
use std::time::{Duration, SystemTime};

pub const MAX_RATELIMIT_RETRIES: u32 = 3;
/// Longer waits are surfaced to the caller instead of holding an active request.
pub const MAX_INLINE_WAIT: Duration = Duration::from_secs(60);

/// RFC 9110 section 10.2.3: delay-seconds or HTTP-date.
pub fn parse_retry_after(response: &Response) -> Option<Duration> {
    parse_value(
        response.headers().get("retry-after")?.to_str().ok()?,
        SystemTime::now(),
    )
}

fn parse_value(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        // Overflow still denotes a long wait, not permission to retry early.
        return Some(Duration::from_secs(value.parse().unwrap_or(u64::MAX)));
    }
    Some(
        httpdate::parse_http_date(value)
            .ok()?
            .duration_since(now)
            .unwrap_or_default(),
    )
}

/// Equal jitter in [base / 2, base], base=min(30s, 1s * 2^attempt).
/// Independent randomness prevents synchronized clients retrying in lockstep.
pub fn default_backoff(attempt: u32) -> Duration {
    let base = 1000u64
        .saturating_mul(1u64.checked_shl(attempt).unwrap_or(u64::MAX))
        .min(30_000);
    Duration::from_millis(rand::thread_rng().gen_range(base / 2..=base))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_after_preserves_server_minimum_and_accepts_http_dates() {
        let now = httpdate::parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT").unwrap();
        assert_eq!(parse_value("120", now), Some(Duration::from_secs(120)));
        assert_eq!(
            parse_value("Sun, 06 Nov 1994 08:51:37 GMT", now),
            Some(Duration::from_secs(120))
        );
        assert_eq!(
            parse_value("Sunday, 06-Nov-94 08:51:37 GMT", now),
            Some(Duration::from_secs(120))
        );
        assert_eq!(
            parse_value("Sun Nov  6 08:51:37 1994", now),
            Some(Duration::from_secs(120))
        );
        assert_eq!(
            parse_value("Sun, 06 Nov 1994 08:48:37 GMT", now),
            Some(Duration::ZERO)
        );
        assert_eq!(
            parse_value("99999999999999999999999999", now),
            Some(Duration::from_secs(u64::MAX))
        );
        for malformed in ["", "-1", "+1", "1.5", "tomorrow"] {
            assert_eq!(parse_value(malformed, now), None);
        }
    }
    #[test]
    fn jitter_stays_inside_exponential_bounds_even_for_overflow() {
        for attempt in (0..40).chain([u32::MAX]) {
            let base = 1000u64
                .saturating_mul(1u64.checked_shl(attempt).unwrap_or(u64::MAX))
                .min(30_000);
            for _ in 0..50 {
                let wait = default_backoff(attempt);
                assert!(wait >= Duration::from_millis(base / 2));
                assert!(wait <= Duration::from_millis(base));
            }
        }
    }
}
