use chrono::Utc;
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use temm1e_core::traits::Channel;
use temm1e_core::types::error::Temm1eError;
use temm1e_core::types::message::OutboundMessage;

use crate::chronos::Chronos;
use crate::cognitive::{Cognitive, LlmCaller};
use crate::conscience::{Conscience, ConscienceState, SelfWorkKind, WakeTrigger};
use crate::monitor;
use crate::pulse;
use crate::self_work;
use crate::store::{MonitorResultInput, Store, StoredConcern};
use crate::tracing_ext;
use crate::types::{ConcernConfig, ConcernId, ConcernSummary, MonitorCheck, Schedule};
use crate::volition::Volition;

/// Central concern dispatcher and lifecycle manager.
pub struct Cortex {
    store: Arc<Store>,
    chronos: Arc<Chronos>,
    conscience: Arc<Conscience>,
    cognitive: Cognitive,
    volition: Option<Volition>,
    channel_map: Arc<HashMap<String, Arc<dyn Channel>>>,
    caller: Arc<dyn LlmCaller>,
    max_concerns: usize,
    review_every_n: u32,
}

impl Cortex {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<Store>,
        chronos: Arc<Chronos>,
        conscience: Arc<Conscience>,
        caller: Arc<dyn LlmCaller>,
        channel_map: Arc<HashMap<String, Arc<dyn Channel>>>,
        max_concerns: usize,
        review_every_n: u32,
        volition_config: Option<(usize,)>,
    ) -> Self {
        let cognitive = Cognitive::new(caller.clone());
        let volition = volition_config
            .map(|(max_actions,)| Volition::new(caller.clone(), store.clone(), max_actions));

        Self {
            store,
            chronos,
            conscience,
            cognitive,
            volition,
            channel_map,
            caller,
            max_concerns,
            review_every_n,
        }
    }

    /// Dispatch a concern that came due.
    pub async fn dispatch(&self, concern_id: ConcernId) {
        let concern = match self.store.get_concern(&concern_id).await {
            Ok(Some(c)) => c,
            Ok(None) => {
                tracing::warn!(target: "perpetuum", concern_id, "Concern not found for dispatch");
                return;
            }
            Err(e) => {
                tracing::error!(target: "perpetuum", concern_id, error = %e, "Failed to load concern");
                return;
            }
        };

        if concern.state != "active" && concern.state != "firing" {
            return;
        }

        tracing_ext::trace_concern_fire(&concern_id, &concern.concern_type);

        // Wrap dispatch in catch_unwind for isolation (resilience architecture)
        let result = AssertUnwindSafe(self.dispatch_inner(&concern))
            .catch_unwind()
            .await;

        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                tracing::error!(
                    target: "perpetuum",
                    concern_id,
                    concern_type = %concern.concern_type,
                    error = %e,
                    "Concern dispatch error"
                );
                self.record_error(&concern_id).await;
            }
            Err(_panic) => {
                tracing::error!(
                    target: "perpetuum",
                    concern_id,
                    concern_type = %concern.concern_type,
                    "Concern dispatch panicked — isolated"
                );
                self.record_error(&concern_id).await;
            }
        }
    }

    async fn dispatch_inner(&self, concern: &StoredConcern) -> Result<(), Temm1eError> {
        match concern.concern_type.as_str() {
            "alarm" => self.fire_alarm(concern).await,
            "monitor" => self.fire_monitor(concern).await,
            "recurring" => self.fire_recurring(concern).await,
            "initiative" => self.fire_initiative(concern).await,
            "self_work" => self.fire_self_work(concern).await,
            other => {
                tracing::warn!(target: "perpetuum", concern_type = other, "Unknown concern type");
                Ok(())
            }
        }
    }

    async fn fire_alarm(&self, concern: &StoredConcern) -> Result<(), Temm1eError> {
        let config: serde_json::Value = serde_json::from_str(&concern.config_json)
            .map_err(|e| Temm1eError::Config(format!("Parse alarm config: {e}")))?;

        let message = config
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Alarm!");

        let alarm_name = &concern.name;
        let notification = format!("⏰ Alarm \"{alarm_name}\": {message}");

        self.notify_user(concern, &notification).await?;
        self.conscience
            .wake(WakeTrigger::AlarmFired {
                concern_id: concern.id.clone(),
            })
            .await;

        // Alarm is one-shot: mark completed and remove
        self.store.delete_concern(&concern.id).await?;
        tracing_ext::trace_concern_event(&concern.id, "alarm", "fired_and_completed");

        Ok(())
    }

    async fn fire_monitor(&self, concern: &StoredConcern) -> Result<(), Temm1eError> {
        let config: serde_json::Value = serde_json::from_str(&concern.config_json)
            .map_err(|e| Temm1eError::Config(format!("Parse monitor config: {e}")))?;

        let check: MonitorCheck = serde_json::from_value(
            config
                .get("check")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        )
        .map_err(|e| Temm1eError::Config(format!("Parse monitor check: {e}")))?;

        let user_intent = config
            .get("user_intent")
            .and_then(|s| s.as_str())
            .unwrap_or("");

        // 1. Execute the check
        let result = monitor::execute_check(&check).await?;

        // 2. Get last hash for change detection
        let history = self.store.monitor_history(&concern.id, 1).await?;
        let last_hash = history.first().and_then(|h| h.raw_content_hash.as_deref());
        let change_detected = last_hash.is_none_or(|h| h != result.content_hash);

        // 3. LLM interpretation if content changed
        let mut notified = false;
        let mut interpretation_json = None;

        if change_detected && !result.content.is_empty() {
            let last_content = history
                .first()
                .and_then(|h| h.raw_content_preview.as_deref());

            let interp = self
                .cognitive
                .interpret(
                    &concern.id,
                    &concern.name,
                    user_intent,
                    &result.content,
                    last_content,
                )
                .await?;

            interpretation_json = serde_json::to_string(&interp)
                .map_err(|e| tracing::warn!(target: "perpetuum", error = %e, "Failed to serialize interpretation"))
                .ok();

            if interp.notify {
                let summary = interp.summary.as_deref().unwrap_or("New content detected");
                let notification = format!("📡 Monitor \"{}\": {summary}", concern.name);
                self.notify_user(concern, &notification).await?;
                self.conscience
                    .wake(WakeTrigger::MonitorDetection {
                        concern_id: concern.id.clone(),
                    })
                    .await;
                notified = true;
            }
        }

        // 4. Record result
        let preview = result.content.chars().take(500).collect::<String>();
        self.store
            .insert_monitor_result(&MonitorResultInput {
                concern_id: concern.id.clone(),
                checked_at: Utc::now(),
                content_hash: Some(result.content_hash),
                content_preview: Some(preview),
                change_detected,
                interpretation: interpretation_json,
                notified,
            })
            .await?;

        tracing_ext::trace_monitor_check(&concern.id, &concern.name, change_detected, notified);

        // 5. Schedule review (every N checks)
        let check_count = self.store.monitor_check_count(&concern.id).await?;
        if self.review_every_n > 0 && check_count % self.review_every_n == 0 && check_count > 0 {
            self.run_schedule_review(concern, user_intent).await?;
        }

        // 6. Reschedule
        self.reschedule_concern(concern).await?;

        Ok(())
    }

    async fn run_schedule_review(
        &self,
        concern: &StoredConcern,
        user_intent: &str,
    ) -> Result<(), Temm1eError> {
        let history = self.store.monitor_history(&concern.id, 20).await?;
        let config: serde_json::Value =
            serde_json::from_str(&concern.config_json).unwrap_or_default();
        let schedule: Schedule = serde_json::from_value(
            config
                .get("schedule")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        )
        .unwrap_or(Schedule::Every(Duration::from_secs(300)));

        let current_secs = match &schedule {
            Schedule::Every(d) => d.as_secs(),
            _ => 300,
        };

        let temporal = self
            .chronos
            .build_context(&self.conscience.current_state().await, &[], &[], None)
            .await;
        let temporal_str =
            Chronos::format_injection(&temporal, crate::types::InjectionDepth::Standard);

        let review = self
            .cognitive
            .review_schedule(
                &concern.id,
                &concern.name,
                user_intent,
                &history,
                current_secs,
                &temporal_str,
            )
            .await?;

        if review.action == "adjust" {
            if let Some(new_secs) = review.new_interval_secs {
                let new_secs = new_secs.clamp(10, 86400); // 10s to 24h
                let mut updated_config = config;
                updated_config["schedule"] =
                    serde_json::to_value(Schedule::Every(Duration::from_secs(new_secs)))
                        .unwrap_or_default();

                let mut updated = concern.clone();
                updated.config_json = serde_json::to_string(&updated_config).unwrap_or_default();
                updated.updated_at = Utc::now().to_rfc3339();
                self.store.update_concern(&updated).await?;

                tracing::info!(
                    target: "perpetuum",
                    concern_id = %concern.id,
                    old_interval = current_secs,
                    new_interval = new_secs,
                    reason = %review.reasoning,
                    "Schedule adjusted by cognitive review"
                );
            }
        }

        if let Some(rec) = &review.user_recommendation {
            let msg = format!("💡 Monitor \"{}\": {rec}", concern.name);
            self.notify_user(concern, &msg).await?;
        }

        Ok(())
    }

    async fn fire_recurring(&self, concern: &StoredConcern) -> Result<(), Temm1eError> {
        let config: serde_json::Value =
            serde_json::from_str(&concern.config_json).unwrap_or_default();
        let description = config
            .get("action_description")
            .and_then(|s| s.as_str())
            .unwrap_or("Recurring task");

        let notification = format!("🔄 Recurring \"{}\": {description}", concern.name);
        self.notify_user(concern, &notification).await?;

        self.reschedule_concern(concern).await?;
        tracing_ext::trace_concern_event(&concern.id, "recurring", "fired");
        Ok(())
    }

    async fn fire_initiative(&self, concern: &StoredConcern) -> Result<(), Temm1eError> {
        let volition = match &self.volition {
            Some(v) => v,
            None => return Ok(()),
        };

        let concerns = self.list_concerns().await;
        let temporal = self
            .chronos
            .build_context(&self.conscience.current_state().await, &concerns, &[], None)
            .await;

        let decision = volition.run_cycle(&concerns, &temporal).await?;

        // Execute decisions
        for create in &decision.create_concerns {
            if self.store.count_active().await.unwrap_or(0) >= self.max_concerns {
                tracing::warn!(target: "perpetuum", "Max concerns reached, skipping volition creation");
                break;
            }
            let config = ConcernConfig::Monitor {
                name: create.name.clone(),
                user_intent: format!("Volition-created: {}", create.name),
                schedule: Schedule::Every(Duration::from_secs(300)),
                check: MonitorCheck::Web {
                    url: create
                        .config
                        .get("url")
                        .and_then(|u| u.as_str())
                        .unwrap_or("about:blank")
                        .to_string(),
                    selector: None,
                    extract: crate::types::ExtractMode::FullText,
                },
                notify_chat_id: create
                    .config
                    .get("chat_id")
                    .and_then(|c| c.as_str())
                    .unwrap_or("system")
                    .to_string(),
                notify_channel: None,
            };
            if let Err(e) = self.create_concern(config, "volition").await {
                tracing::warn!(target: "perpetuum", error = %e, "Volition concern creation failed");
            }
        }

        for cancel_id in &decision.cancel_concerns {
            if let Err(e) = self.cancel_concern(cancel_id).await {
                tracing::warn!(target: "perpetuum", error = %e, id = cancel_id, "Volition cancellation failed");
            }
        }

        for notif in &decision.notifications {
            self.send_notification(&notif.chat_id, None, &notif.message)
                .await
                .ok();
        }

        // Apply state recommendation
        if let Some(ref rec) = decision.state_recommendation {
            match rec.as_str() {
                "sleep" => {
                    self.conscience
                        .transition_to(
                            ConscienceState::Sleep {
                                since: Utc::now(),
                                work: SelfWorkKind::MemoryConsolidation,
                            },
                            "volition_recommendation",
                        )
                        .await;
                }
                "dream" => {
                    self.conscience
                        .transition_to(
                            ConscienceState::Dream { since: Utc::now() },
                            "volition_recommendation",
                        )
                        .await;
                }
                _ => {}
            }
        }

        self.reschedule_concern(concern).await?;
        Ok(())
    }

    async fn fire_self_work(&self, concern: &StoredConcern) -> Result<(), Temm1eError> {
        let config: serde_json::Value =
            serde_json::from_str(&concern.config_json).unwrap_or_default();
        let kind_str = config
            .get("kind")
            .and_then(|s| s.as_str())
            .unwrap_or("memory_consolidation");

        let kind = match kind_str {
            "failure_analysis" => SelfWorkKind::FailureAnalysis,
            "log_introspection" => SelfWorkKind::LogIntrospection,
            "session_cleanup" => SelfWorkKind::SessionCleanup,
            "blueprint_refinement" => SelfWorkKind::BlueprintRefinement,
            _ => SelfWorkKind::MemoryConsolidation,
        };

        let llm_caller: Option<Arc<dyn LlmCaller>> = if kind.uses_llm() {
            Some(self.caller.clone())
        } else {
            None
        };

        let result = self_work::execute_self_work(&kind, &self.store, llm_caller.as_ref()).await?;

        tracing::info!(target: "perpetuum", work = %kind.name(), result = %result, "Self-work complete");

        // Self-work is one-shot: remove
        self.store.delete_concern(&concern.id).await?;
        self.conscience.complete_self_work().await;
        Ok(())
    }

    // ---- Public API ----

    /// Create a new concern.
    pub async fn create_concern(
        &self,
        config: ConcernConfig,
        source: &str,
    ) -> Result<ConcernId, Temm1eError> {
        if self.store.count_active().await? >= self.max_concerns {
            return Err(Temm1eError::Config(format!(
                "Max concerns ({}) reached",
                self.max_concerns
            )));
        }

        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();

        let (concern_type, name, config_json, notify_chat_id, notify_channel, next_fire) =
            match &config {
                ConcernConfig::Alarm {
                    name,
                    fire_at,
                    notify_chat_id,
                    notify_channel,
                    ..
                } => (
                    "alarm",
                    name.clone(),
                    serde_json::to_string(&config)?,
                    Some(notify_chat_id.clone()),
                    notify_channel.clone(),
                    Some(fire_at.to_rfc3339()),
                ),
                ConcernConfig::Monitor {
                    name,
                    schedule,
                    notify_chat_id,
                    notify_channel,
                    ..
                } => {
                    let next = pulse::next_fire_time(schedule, self.chronos.timezone())
                        .map(|dt| dt.to_rfc3339());
                    (
                        "monitor",
                        name.clone(),
                        serde_json::to_string(&config)?,
                        Some(notify_chat_id.clone()),
                        notify_channel.clone(),
                        next,
                    )
                }
                ConcernConfig::Recurring {
                    name,
                    cron_expr,
                    notify_chat_id,
                    notify_channel,
                    ..
                } => {
                    let schedule = Schedule::Cron(cron_expr.clone());
                    let next = pulse::next_fire_time(&schedule, self.chronos.timezone())
                        .map(|dt| dt.to_rfc3339());
                    (
                        "recurring",
                        name.clone(),
                        serde_json::to_string(&config)?,
                        Some(notify_chat_id.clone()),
                        notify_channel.clone(),
                        next,
                    )
                }
                ConcernConfig::Initiative { interval_secs } => {
                    let next =
                        (now + chrono::Duration::seconds(*interval_secs as i64)).to_rfc3339();
                    (
                        "initiative",
                        "volition".to_string(),
                        serde_json::to_string(&config)?,
                        None,
                        None,
                        Some(next),
                    )
                }
                ConcernConfig::SelfWork { kind } => (
                    "self_work",
                    kind.clone(),
                    serde_json::to_string(&config)?,
                    None,
                    None,
                    Some(now.to_rfc3339()), // Fire immediately
                ),
            };

        let stored = StoredConcern {
            id: id.clone(),
            concern_type: concern_type.to_string(),
            name,
            source: source.to_string(),
            state: "active".to_string(),
            config_json,
            notify_chat_id,
            notify_channel,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
            last_fired_at: None,
            next_fire_at: next_fire,
            error_count: 0,
            consecutive_errors: 0,
        };

        self.store.insert_concern(&stored).await?;
        tracing_ext::trace_concern_event(&id, concern_type, "created");

        Ok(id)
    }

    /// Cancel a concern by ID.
    pub async fn cancel_concern(&self, id: &str) -> Result<(), Temm1eError> {
        self.store.delete_concern(id).await?;
        tracing_ext::trace_concern_event(id, "unknown", "cancelled");
        Ok(())
    }

    /// List active concerns as summaries.
    pub async fn list_concerns(&self) -> Vec<ConcernSummary> {
        let concerns = self.store.list_active_concerns().await.unwrap_or_default();
        concerns
            .into_iter()
            .map(|c| {
                let schedule_desc = extract_schedule_desc(&c.config_json);
                ConcernSummary {
                    id: c.id,
                    concern_type: c.concern_type,
                    name: c.name,
                    source: c.source,
                    state: c.state,
                    schedule_desc,
                    last_fired: c
                        .last_fired_at
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&Utc)),
                    next_fire: c
                        .next_fire_at
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&Utc)),
                }
            })
            .collect()
    }

    /// Adjust schedule for a monitor concern.
    pub async fn adjust_schedule(
        &self,
        id: &str,
        new_schedule: Schedule,
    ) -> Result<(), Temm1eError> {
        let concern = self
            .store
            .get_concern(id)
            .await?
            .ok_or_else(|| Temm1eError::Config(format!("Concern {id} not found")))?;

        let mut config: serde_json::Value =
            serde_json::from_str(&concern.config_json).unwrap_or_default();
        config["schedule"] = serde_json::to_value(&new_schedule)?;

        let next_fire =
            pulse::next_fire_time(&new_schedule, self.chronos.timezone()).map(|dt| dt.to_rfc3339());

        let mut updated = concern;
        updated.config_json = serde_json::to_string(&config)?;
        updated.next_fire_at = next_fire;
        updated.updated_at = Utc::now().to_rfc3339();
        self.store.update_concern(&updated).await?;

        tracing_ext::trace_concern_event(id, "monitor", "schedule_adjusted");
        Ok(())
    }

    // ---- Internal helpers ----

    async fn reschedule_concern(&self, concern: &StoredConcern) -> Result<(), Temm1eError> {
        let config: serde_json::Value =
            serde_json::from_str(&concern.config_json).unwrap_or_default();

        let schedule_value = config.get("schedule").or_else(|| config.get("cron_expr"));
        let next_fire = if let Some(sv) = schedule_value {
            let schedule: Schedule = if let Some(cron_str) = sv.as_str() {
                Schedule::Cron(cron_str.to_string())
            } else {
                serde_json::from_value(sv.clone())
                    .unwrap_or(Schedule::Every(Duration::from_secs(300)))
            };
            pulse::next_fire_after(&schedule, Utc::now(), self.chronos.timezone())
                .map(|dt| dt.to_rfc3339())
        } else if concern.concern_type == "initiative" {
            let interval = config
                .get("interval_secs")
                .and_then(|n| n.as_u64())
                .unwrap_or(900);
            Some((Utc::now() + chrono::Duration::seconds(interval as i64)).to_rfc3339())
        } else {
            None
        };

        if let Some(next) = next_fire {
            let mut updated = concern.clone();
            updated.state = "active".to_string(); // Reset from 'firing' to 'active'
            updated.last_fired_at = Some(Utc::now().to_rfc3339());
            updated.next_fire_at = Some(next);
            updated.updated_at = Utc::now().to_rfc3339();
            updated.consecutive_errors = 0;
            self.store.update_concern(&updated).await?;
        }

        Ok(())
    }

    async fn record_error(&self, concern_id: &str) {
        if let Ok(Some(mut concern)) = self.store.get_concern(concern_id).await {
            concern.error_count += 1;
            concern.consecutive_errors += 1;
            concern.updated_at = Utc::now().to_rfc3339();

            // Disable after 3 consecutive errors
            if concern.consecutive_errors >= 3 {
                concern.state = "disabled".to_string();
                tracing::warn!(
                    target: "perpetuum",
                    concern_id,
                    errors = concern.consecutive_errors,
                    "Concern disabled after 3 consecutive errors"
                );
            }

            self.store.update_concern(&concern).await.ok();
        }
    }

    async fn notify_user(&self, concern: &StoredConcern, text: &str) -> Result<(), Temm1eError> {
        let chat_id = concern.notify_chat_id.as_deref().unwrap_or("system");
        let channel_name = concern.notify_channel.as_deref();
        self.send_notification(chat_id, channel_name, text).await
    }

    async fn send_notification(
        &self,
        chat_id: &str,
        channel_name: Option<&str>,
        text: &str,
    ) -> Result<(), Temm1eError> {
        // Try specified channel first, then try all channels
        let channels_to_try: Vec<&Arc<dyn Channel>> = if let Some(name) = channel_name {
            self.channel_map
                .get(name)
                .map(|c| vec![c])
                .unwrap_or_default()
        } else {
            self.channel_map.values().collect()
        };

        let msg = OutboundMessage {
            chat_id: chat_id.to_string(),
            text: text.to_string(),
            reply_to: None,
            parse_mode: None,
        };

        for channel in channels_to_try {
            if channel.send_message(msg.clone()).await.is_ok() {
                return Ok(());
            }
        }

        tracing::warn!(
            target: "perpetuum",
            chat_id,
            "No channel could deliver notification"
        );
        Ok(())
    }
}

fn extract_schedule_desc(config_json: &str) -> Option<String> {
    let config: serde_json::Value = serde_json::from_str(config_json).ok()?;
    if let Some(schedule) = config.get("schedule") {
        if let Ok(s) = serde_json::from_value::<Schedule>(schedule.clone()) {
            return Some(match s {
                Schedule::At(dt) => format!("at {}", dt.format("%H:%M")),
                Schedule::Every(d) => {
                    let secs = d.as_secs();
                    if secs < 60 {
                        format!("every {secs}s")
                    } else if secs < 3600 {
                        format!("every {}m", secs / 60)
                    } else {
                        format!("every {}h", secs / 3600)
                    }
                }
                Schedule::Cron(expr) => format!("cron {expr}"),
            });
        }
    }
    if let Some(cron) = config.get("cron_expr").and_then(|s| s.as_str()) {
        return Some(format!("cron {cron}"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_schedule_desc_every() {
        let json = r#"{"schedule":{"Every":300}}"#;
        assert_eq!(extract_schedule_desc(json), Some("every 5m".to_string()));
    }

    #[test]
    fn extract_schedule_desc_cron() {
        let json = r#"{"cron_expr":"0 9 * * 1-5"}"#;
        assert_eq!(
            extract_schedule_desc(json),
            Some("cron 0 9 * * 1-5".to_string())
        );
    }

    #[test]
    fn extract_schedule_desc_none() {
        assert_eq!(extract_schedule_desc("{}"), None);
    }
}
