use chrono::{DateTime, Datelike, Timelike, Utc};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::conscience::ConscienceState;
use crate::store::Store;
use crate::types::{ConcernSummary, InjectionDepth, NextEvent, ParkedTaskSummary, TemporalContext};

/// Internal clock and temporal cognition for Tem.
///
/// Not just `SystemTime::now()` — Chronos maintains Tem's subjective experience of time,
/// tracks user interaction patterns, and builds temporal context for LLM injection.
pub struct Chronos {
    timezone: chrono_tz::Tz,
    boot_time: DateTime<Utc>,
    last_user_interaction: Arc<RwLock<Option<DateTime<Utc>>>>,
    store: Arc<Store>,
}

impl Chronos {
    pub fn new(timezone: chrono_tz::Tz, store: Arc<Store>) -> Self {
        Self {
            timezone,
            boot_time: Utc::now(),
            last_user_interaction: Arc::new(RwLock::new(None)),
            store,
        }
    }

    /// Build temporal context snapshot for LLM injection.
    pub async fn build_context(
        &self,
        conscience_state: &ConscienceState,
        concerns: &[ConcernSummary],
        parked: &[ParkedTaskSummary],
        next_event: Option<NextEvent>,
    ) -> TemporalContext {
        let now = Utc::now();
        let local = now.with_timezone(&self.timezone);
        let uptime = (now - self.boot_time).to_std().unwrap_or(Duration::ZERO);
        let idle = self.idle_duration().await;
        let probability = self.user_active_probability().await;

        TemporalContext {
            now,
            local_time: local
                .format("%A %l:%M %p %Z")
                .to_string()
                .trim()
                .to_string(),
            uptime,
            idle_duration: idle,
            conscience_state: conscience_state.to_string(),
            active_concerns: concerns.to_vec(),
            parked_tasks: parked.to_vec(),
            next_event,
            user_active_probability: probability,
        }
    }

    /// Format temporal context as a string for system prompt injection.
    pub fn format_injection(ctx: &TemporalContext, depth: InjectionDepth) -> String {
        let mut lines = vec!["[Temporal Awareness]".to_string()];

        let idle_str = format_duration(ctx.idle_duration);
        let uptime_str = format_duration(ctx.uptime);

        lines.push(format!(
            "Time: {} | Uptime: {} | Idle: {} | State: {}",
            ctx.local_time, uptime_str, idle_str, ctx.conscience_state
        ));

        if depth == InjectionDepth::Minimal {
            return lines.join("\n");
        }

        // Standard: add concerns summary
        if !ctx.active_concerns.is_empty() {
            let concern_summary: Vec<String> = ctx
                .active_concerns
                .iter()
                .filter(|c| c.concern_type != "initiative")
                .take(5)
                .map(|c| {
                    let sched = c.schedule_desc.as_deref().unwrap_or("once");
                    format!("{}/{} {}", c.concern_type, c.name, sched)
                })
                .collect();
            lines.push(format!(
                "Concerns: {} active ({})",
                ctx.active_concerns.len(),
                concern_summary.join(", ")
            ));
        }

        if let Some(ref next) = ctx.next_event {
            let eta_str = format_duration(next.eta);
            lines.push(format!("Next event: {} in {}", next.name, eta_str));
        }

        if depth == InjectionDepth::Standard {
            return lines.join("\n");
        }

        // Full: add parked tasks and user pattern
        if !ctx.parked_tasks.is_empty() {
            let parked_summary: Vec<String> = ctx
                .parked_tasks
                .iter()
                .take(3)
                .map(|p| {
                    format!(
                        "{} ({})",
                        p.reason,
                        format_duration(
                            (Utc::now() - p.parked_since)
                                .to_std()
                                .unwrap_or(Duration::ZERO)
                        )
                    )
                })
                .collect();
            lines.push(format!(
                "Parked: {} tasks ({})",
                ctx.parked_tasks.len(),
                parked_summary.join(", ")
            ));
        }

        lines.push(format!(
            "User pattern: {:.0}% likely active now",
            ctx.user_active_probability * 100.0
        ));

        lines.join("\n")
    }

    /// Record a user interaction — updates idle tracking and activity log.
    pub async fn record_interaction(&self) {
        let now = Utc::now();
        *self.last_user_interaction.write().await = Some(now);

        if let Err(e) = self.store.record_activity(now).await {
            tracing::warn!(error = %e, "Failed to record activity");
        }
    }

    /// Get idle duration since last user interaction.
    pub async fn idle_duration(&self) -> Duration {
        let last = *self.last_user_interaction.read().await;
        match last {
            Some(t) => (Utc::now() - t).to_std().unwrap_or(Duration::ZERO),
            None => (Utc::now() - self.boot_time)
                .to_std()
                .unwrap_or(Duration::ZERO),
        }
    }

    /// Get user active probability for current hour (0.0-1.0).
    pub async fn user_active_probability(&self) -> f64 {
        let now = Utc::now().with_timezone(&self.timezone);
        let hour = now.hour();
        let weekday = now.weekday().num_days_from_monday();

        self.store
            .activity_probability_at(hour, weekday, self.timezone, now.with_timezone(&Utc))
            .await
            .unwrap_or(0.5)
    }

    /// Get the timezone.
    pub fn timezone(&self) -> &chrono_tz::Tz {
        &self.timezone
    }

    /// Get boot time.
    pub fn boot_time(&self) -> DateTime<Utc> {
        self.boot_time
    }
}

/// Format a Duration as human-readable (e.g., "2h14m", "45s", "3d2h").
fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    if total_secs < 60 {
        return format!("{}s", total_secs);
    }
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let minutes = (total_secs % 3600) / 60;

    if days > 0 {
        format!("{}d{}h", days, hours)
    } else if hours > 0 {
        format!("{}h{}m", hours, minutes)
    } else {
        format!("{}m", minutes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_works() {
        assert_eq!(format_duration(Duration::from_secs(30)), "30s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1h1m");
        assert_eq!(format_duration(Duration::from_secs(90000)), "1d1h");
    }

    #[test]
    fn injection_minimal() {
        let ctx = TemporalContext {
            now: Utc::now(),
            local_time: "Monday 3:47 PM PST".into(),
            uptime: Duration::from_secs(3600),
            idle_duration: Duration::from_secs(600),
            conscience_state: "active".into(),
            active_concerns: vec![],
            parked_tasks: vec![],
            next_event: None,
            user_active_probability: 0.75,
        };

        let result = Chronos::format_injection(&ctx, InjectionDepth::Minimal);
        assert!(result.contains("[Temporal Awareness]"));
        assert!(result.contains("Monday 3:47 PM PST"));
        assert!(!result.contains("Concerns:"));
        assert!(!result.contains("User pattern:"));
    }

    #[test]
    fn injection_standard_includes_concerns() {
        let ctx = TemporalContext {
            now: Utc::now(),
            local_time: "Monday 3:47 PM PST".into(),
            uptime: Duration::from_secs(3600),
            idle_duration: Duration::from_secs(600),
            conscience_state: "active".into(),
            active_concerns: vec![ConcernSummary {
                id: "mon-001".into(),
                concern_type: "monitor".into(),
                name: "reddit".into(),
                source: "user".into(),
                state: "active".into(),
                schedule_desc: Some("every 5m".into()),
                last_fired: None,
                next_fire: None,
            }],
            parked_tasks: vec![],
            next_event: Some(NextEvent {
                name: "reddit check".into(),
                eta: Duration::from_secs(120),
            }),
            user_active_probability: 0.75,
        };

        let result = Chronos::format_injection(&ctx, InjectionDepth::Standard);
        assert!(result.contains("Concerns: 1 active"));
        assert!(result.contains("Next event: reddit check in 2m"));
        assert!(!result.contains("User pattern:"));
    }

    #[test]
    fn injection_full_includes_everything() {
        let ctx = TemporalContext {
            now: Utc::now(),
            local_time: "Monday 3:47 PM PST".into(),
            uptime: Duration::from_secs(3600),
            idle_duration: Duration::from_secs(600),
            conscience_state: "active".into(),
            active_concerns: vec![],
            parked_tasks: vec![],
            next_event: None,
            user_active_probability: 0.32,
        };

        let result = Chronos::format_injection(&ctx, InjectionDepth::Full);
        assert!(result.contains("User pattern: 32% likely active now"));
    }

    #[test]
    fn injection_depth_from_str() {
        assert_eq!(InjectionDepth::parse("minimal"), InjectionDepth::Minimal);
        assert_eq!(InjectionDepth::parse("full"), InjectionDepth::Full);
        assert_eq!(InjectionDepth::parse("anything"), InjectionDepth::Standard);
    }
}
