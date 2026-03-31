use chrono::{DateTime, Utc};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Notify};
use tokio_util::sync::CancellationToken;

use crate::store::Store;
use crate::tracing_ext;
use crate::types::{ConcernId, Schedule};

/// Events emitted by Pulse when concerns come due.
#[derive(Debug)]
pub enum PulseEvent {
    ConcernDue(ConcernId),
}

/// Timer engine: computes next due concern, sleeps until it, fires it.
///
/// No timing wheel crate — uses `tokio::time::sleep` with `cron::Schedule::upcoming()`.
/// Tokio internally uses a hierarchical timing wheel.
pub struct Pulse {
    store: Arc<Store>,
    concern_tx: mpsc::Sender<PulseEvent>,
    cancel: CancellationToken,
    schedule_changed: Arc<Notify>,
}

impl Pulse {
    pub fn new(store: Arc<Store>, cancel: CancellationToken) -> (Self, mpsc::Receiver<PulseEvent>) {
        let (tx, rx) = mpsc::channel(64);
        let pulse = Self {
            store,
            concern_tx: tx,
            cancel,
            schedule_changed: Arc::new(Notify::new()),
        };
        (pulse, rx)
    }

    /// Get a handle to notify pulse that the schedule changed.
    pub fn schedule_notifier(&self) -> Arc<Notify> {
        self.schedule_changed.clone()
    }

    /// Main loop: sleep until next due concern, fire all due concerns, repeat.
    pub async fn run(&self) {
        tracing::info!(target: "perpetuum", "Pulse timer engine started");

        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => {
                    tracing::info!(target: "perpetuum", "Pulse shutting down");
                    break;
                }
                _ = self.sleep_until_next() => {
                    self.fire_due_concerns().await;
                }
                _ = self.schedule_changed.notified() => {
                    // Schedule changed — recompute next fire time
                    tracing::debug!(target: "perpetuum", "Schedule changed, recomputing");
                    continue;
                }
            }
        }
    }

    async fn sleep_until_next(&self) {
        // 10s timeout on DB query — prevents Pulse from hanging on slow I/O
        let next = tokio::time::timeout(Duration::from_secs(10), self.store.next_fire_time())
            .await
            .unwrap_or(Ok(None))
            .unwrap_or(None);

        match next {
            Some(fire_at) => {
                let until = fire_at - Utc::now();
                let duration = until.to_std().unwrap_or(Duration::ZERO);

                tracing_ext::trace_pulse_tick(0, Some(duration.as_secs_f64()));

                if duration.is_zero() {
                    // Already due — don't sleep
                    return;
                }

                tokio::time::sleep(duration).await;
            }
            None => {
                // No concerns scheduled — poll every 60s
                tracing_ext::trace_pulse_tick(0, None);
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        }
    }

    async fn fire_due_concerns(&self) {
        let due = match self.store.claim_due_concerns(Utc::now()).await {
            Ok(ids) => ids,
            Err(e) => {
                tracing::error!(target: "perpetuum", error = %e, "Failed to get due concerns");
                return;
            }
        };

        if due.is_empty() {
            return;
        }

        tracing_ext::trace_pulse_tick(due.len(), None);

        for concern_id in due {
            if self
                .concern_tx
                .send(PulseEvent::ConcernDue(concern_id))
                .await
                .is_err()
            {
                tracing::warn!(target: "perpetuum", "Pulse event channel closed");
                break;
            }
        }
    }
}

/// Convert 5-field cron expression to 7-field format used by the `cron` crate.
///
/// Standard: `*/5 * * * *` (min hr dom mon dow)
/// Cron crate: `0 */5 * * * * *` (sec min hr dom mon dow year)
pub fn cron5_to_cron7(expr: &str) -> String {
    format!("0 {} *", expr.trim())
}

/// Parse a schedule and compute the next fire time.
pub fn next_fire_time(schedule: &Schedule, tz: &chrono_tz::Tz) -> Option<DateTime<Utc>> {
    match schedule {
        Schedule::At(dt) => {
            if *dt > Utc::now() {
                Some(*dt)
            } else {
                None // Already past
            }
        }
        Schedule::Every(dur) => Some(Utc::now() + chrono::Duration::from_std(*dur).ok()?),
        Schedule::Cron(expr) => {
            let cron7 = cron5_to_cron7(expr);
            let schedule = cron::Schedule::from_str(&cron7).ok()?;
            schedule
                .upcoming(*tz)
                .next()
                .map(|dt| dt.with_timezone(&Utc))
        }
    }
}

/// Compute the next fire time after a specific base time (for recurring reschedule).
pub fn next_fire_after(
    schedule: &Schedule,
    after: DateTime<Utc>,
    tz: &chrono_tz::Tz,
) -> Option<DateTime<Utc>> {
    match schedule {
        Schedule::At(_) => None, // One-shot, no next
        Schedule::Every(dur) => Some(after + chrono::Duration::from_std(*dur).ok()?),
        Schedule::Cron(expr) => {
            let cron7 = cron5_to_cron7(expr);
            let sched = cron::Schedule::from_str(&cron7).ok()?;
            // Find next occurrence after the given time
            sched
                .upcoming(*tz)
                .find(|t| t.with_timezone(&Utc) > after)
                .map(|dt| dt.with_timezone(&Utc))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron5_to_cron7_conversion() {
        assert_eq!(cron5_to_cron7("*/5 * * * *"), "0 */5 * * * * *");
        assert_eq!(cron5_to_cron7("0 9 * * 1-5"), "0 0 9 * * 1-5 *");
        assert_eq!(cron5_to_cron7("30 2 * * *"), "0 30 2 * * * *");
    }

    #[test]
    fn next_fire_at_schedule() {
        let tz: chrono_tz::Tz = "America/Los_Angeles".parse().unwrap();

        // At: future time
        let future = Utc::now() + chrono::Duration::hours(1);
        let at = Schedule::At(future);
        assert!(next_fire_time(&at, &tz).is_some());

        // At: past time
        let past = Utc::now() - chrono::Duration::hours(1);
        let at_past = Schedule::At(past);
        assert!(next_fire_time(&at_past, &tz).is_none());

        // Every: always returns future
        let every = Schedule::Every(Duration::from_secs(300));
        let next = next_fire_time(&every, &tz).unwrap();
        assert!(next > Utc::now());

        // Cron: valid expression
        let cron = Schedule::Cron("*/5 * * * *".to_string());
        let next = next_fire_time(&cron, &tz);
        assert!(next.is_some());

        // Cron: invalid expression
        let bad_cron = Schedule::Cron("not a cron".to_string());
        assert!(next_fire_time(&bad_cron, &tz).is_none());
    }

    #[test]
    fn next_fire_after_recurring() {
        let tz: chrono_tz::Tz = "UTC".parse().unwrap();
        let now = Utc::now();

        let every = Schedule::Every(Duration::from_secs(60));
        let next = next_fire_after(&every, now, &tz).unwrap();
        assert!(next > now);

        // At: one-shot has no next
        let at = Schedule::At(now);
        assert!(next_fire_after(&at, now, &tz).is_none());
    }
}
