//! # Perpetuum — Perpetual Time-Aware Entity Framework
//!
//! Transforms Tem from a reactive request-response agent into a perpetual,
//! time-aware, autonomous entity. Provides time awareness (Chronos),
//! concern scheduling (Cortex/Pulse), entity state machine (Conscience),
//! LLM-cognitive scheduling (Cognitive), and proactive agency (Volition).

pub mod chronos;
pub mod cognitive;
pub mod conscience;
pub mod cortex;
pub mod monitor;
pub mod parking;
pub mod pulse;
pub mod self_work;
pub mod store;
pub mod tools;
pub mod tracing_ext;
pub mod types;
pub mod volition;

pub use chronos::Chronos;
pub use cognitive::{Cognitive, LlmCaller, ProviderCaller};
pub use conscience::{Conscience, ConscienceState, SelfWorkKind, WakeTrigger};
pub use cortex::Cortex;
pub use pulse::{Pulse, PulseEvent};
pub use store::Store;
pub use types::*;

use futures::FutureExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Perpetuum runtime — the perpetual entity lifecycle manager.
pub struct Perpetuum {
    pub chronos: Arc<Chronos>,
    pub cortex: Arc<Cortex>,
    pub conscience: Arc<Conscience>,
    pub store: Arc<Store>,
    cancel: CancellationToken,
    pulse_notifier: Arc<tokio::sync::Notify>,
}

/// Configuration for Perpetuum.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PerpetualConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default = "default_max_concerns")]
    pub max_concerns: usize,
    #[serde(default)]
    pub conscience: ConscienceConfig,
    #[serde(default)]
    pub cognitive: CognitiveConfig,
    #[serde(default)]
    pub volition: VolitionConfig,
}

impl Default for PerpetualConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            timezone: default_timezone(),
            max_concerns: default_max_concerns(),
            conscience: ConscienceConfig::default(),
            cognitive: CognitiveConfig::default(),
            volition: VolitionConfig::default(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConscienceConfig {
    #[serde(default = "default_idle_threshold")]
    pub idle_threshold_secs: u64,
    #[serde(default = "default_dream_threshold")]
    pub dream_threshold_secs: u64,
}

impl Default for ConscienceConfig {
    fn default() -> Self {
        Self {
            idle_threshold_secs: default_idle_threshold(),
            dream_threshold_secs: default_dream_threshold(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CognitiveConfig {
    #[serde(default = "default_review_every_n")]
    pub review_every_n_checks: u32,
    #[serde(default = "default_true")]
    pub interpret_changes: bool,
}

impl Default for CognitiveConfig {
    fn default() -> Self {
        Self {
            review_every_n_checks: default_review_every_n(),
            interpret_changes: true,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VolitionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_volition_interval")]
    pub interval_secs: u64,
    #[serde(default = "default_max_actions")]
    pub max_actions_per_cycle: usize,
    #[serde(default = "default_true")]
    pub event_triggered: bool,
}

impl Default for VolitionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: default_volition_interval(),
            max_actions_per_cycle: default_max_actions(),
            event_triggered: true,
        }
    }
}

fn default_timezone() -> String {
    "UTC".to_string()
}
fn default_max_concerns() -> usize {
    100
}
fn default_idle_threshold() -> u64 {
    900
}
fn default_dream_threshold() -> u64 {
    3600
}
fn default_review_every_n() -> u32 {
    20
}
fn default_volition_interval() -> u64 {
    900
}
fn default_max_actions() -> usize {
    2
}
fn default_true() -> bool {
    true
}

impl Perpetuum {
    /// Create and initialize the Perpetuum runtime.
    pub async fn new(
        config: PerpetualConfig,
        provider: Arc<dyn temm1e_core::traits::Provider>,
        model: String,
        channel_map: Arc<HashMap<String, Arc<dyn temm1e_core::traits::Channel>>>,
        db_path: &str,
    ) -> Result<Self, temm1e_core::types::error::Temm1eError> {
        // Config validation — catch bad values before they cause runtime issues
        let max_concerns = config.max_concerns.max(1);
        let idle_secs = config.conscience.idle_threshold_secs.max(60);
        let dream_secs = config.conscience.dream_threshold_secs.max(60);

        let store = Arc::new(Store::new(db_path).await?);

        let tz: chrono_tz::Tz = config.timezone.parse().unwrap_or(chrono_tz::UTC);

        let chronos = Arc::new(Chronos::new(tz, store.clone()));

        let conscience = Arc::new(Conscience::new(
            Duration::from_secs(idle_secs),
            Duration::from_secs(dream_secs),
            store.clone(),
        ));

        let caller: Arc<dyn LlmCaller> = Arc::new(ProviderCaller::new(provider, model));

        let volition_config = if config.volition.enabled {
            Some((config.volition.max_actions_per_cycle,))
        } else {
            None
        };

        let cortex = Arc::new(Cortex::new(
            store.clone(),
            chronos.clone(),
            conscience.clone(),
            caller,
            channel_map,
            max_concerns,
            config.cognitive.review_every_n_checks,
            volition_config,
        ));

        let cancel = CancellationToken::new();
        let (pulse, _) = Pulse::new(store.clone(), cancel.clone());
        let pulse_notifier = pulse.schedule_notifier();

        Ok(Self {
            chronos,
            cortex,
            conscience,
            store,
            cancel,
            pulse_notifier,
        })
    }

    /// Start the Perpetuum runtime: spawns Pulse timer + concern dispatch loop.
    pub fn start(&self) -> tokio::task::JoinHandle<()> {
        let store = self.store.clone();
        let cortex = self.cortex.clone();
        let cancel = self.cancel.clone();
        let notifier = self.pulse_notifier.clone();

        tokio::spawn(async move {
            // Resilience: restart Pulse + dispatch loop if either panics.
            // Perpetuum is meant to run 24/7/365 — a single panic must not kill scheduling.
            loop {
                if cancel.is_cancelled() {
                    break;
                }

                let (pulse, mut pulse_rx) = Pulse::new(store.clone(), cancel.clone());
                let pulse_cancel = cancel.clone();
                let pulse_handle = tokio::spawn({
                    let pulse = pulse;
                    async move {
                        let result = std::panic::AssertUnwindSafe(pulse.run())
                            .catch_unwind()
                            .await;
                        if result.is_err() {
                            tracing::error!(target: "perpetuum", "Pulse panicked — will restart");
                        }
                    }
                });

                // Concern dispatch loop with panic recovery
                // Bounded: max 20 concurrent dispatches to prevent resource exhaustion
                let dispatch_semaphore = Arc::new(tokio::sync::Semaphore::new(20));
                let dispatch_result = std::panic::AssertUnwindSafe(async {
                    loop {
                        tokio::select! {
                            _ = cancel.cancelled() => break,
                            event = pulse_rx.recv() => {
                                match event {
                                    Some(PulseEvent::ConcernDue(id)) => {
                                        let cortex = cortex.clone();
                                        let sem = dispatch_semaphore.clone();
                                        tokio::spawn(async move {
                                            let _permit = sem.acquire().await;
                                            cortex.dispatch(id).await;
                                        });
                                    }
                                    None => {
                                        tracing::warn!(target: "perpetuum", "Pulse channel closed — restarting");
                                        break;
                                    }
                                }
                            }
                            _ = notifier.notified() => {}
                        }
                    }
                })
                .catch_unwind()
                .await;

                pulse_cancel.cancel();
                pulse_handle.await.ok();

                if cancel.is_cancelled() {
                    break;
                }

                if dispatch_result.is_err() {
                    tracing::error!(target: "perpetuum", "Dispatch loop panicked — restarting in 5s");
                } else {
                    tracing::warn!(target: "perpetuum", "Perpetuum loop exited — restarting in 5s");
                }

                // Brief pause before restart to avoid tight panic loops
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }

            tracing::info!(target: "perpetuum", "Perpetuum runtime stopped");
        })
    }

    /// Get all Perpetuum agent tools for registration.
    pub fn tools(self: &Arc<Self>) -> Vec<Arc<dyn temm1e_core::traits::Tool>> {
        tools::create_tools(self.cortex.clone())
    }

    /// Build temporal context for injection into LLM calls.
    pub async fn temporal_context(&self) -> TemporalContext {
        let concerns = self.cortex.list_concerns().await;
        let state = self.conscience.current_state().await;
        self.chronos
            .build_context(&state, &concerns, &[], None)
            .await
    }

    /// Format temporal context as string for system prompt injection.
    pub async fn temporal_injection(&self, depth: &str) -> String {
        let ctx = self.temporal_context().await;
        Chronos::format_injection(&ctx, InjectionDepth::parse(depth))
    }

    /// Record a user interaction for idle tracking.
    pub async fn record_user_interaction(&self) {
        self.chronos.record_interaction().await;
        self.conscience.wake(WakeTrigger::UserMessage).await;
    }

    /// Graceful shutdown.
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }

    /// Notify pulse that schedule changed (new concern added/adjusted).
    pub fn notify_schedule_change(&self) {
        self.pulse_notifier.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults() {
        let config = PerpetualConfig::default();
        assert!(config.enabled); // ON by default
        assert_eq!(config.timezone, "UTC");
        assert_eq!(config.max_concerns, 100);
        assert!(!config.volition.enabled);
    }

    #[test]
    fn config_serialization_roundtrip() {
        let config = PerpetualConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let _: PerpetualConfig = serde_json::from_str(&json).unwrap();
    }
}
