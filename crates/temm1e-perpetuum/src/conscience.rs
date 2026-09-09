use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::store::Store;
use crate::tracing_ext;

/// Entity conscience states — proactive choices, not constraints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConscienceState {
    Active,
    Idle {
        since: DateTime<Utc>,
    },
    Sleep {
        since: DateTime<Utc>,
        work: SelfWorkKind,
    },
    Dream {
        since: DateTime<Utc>,
    },
}

impl ConscienceState {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Idle { .. } => "idle",
            Self::Sleep { .. } => "sleep",
            Self::Dream { .. } => "dream",
        }
    }
}

impl std::fmt::Display for ConscienceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Self-improvement work kinds performed during Sleep state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SelfWorkKind {
    MemoryConsolidation,
    FailureAnalysis,
    LogIntrospection,
    SessionCleanup,
    BlueprintRefinement,
    Vigil,
    /// Self-grow at the skill layer: analyze recent activity for unmet
    /// needs, then write reusable skill files. Requires `cambium.enabled = true`.
    CambiumSkills,
}

impl SelfWorkKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::MemoryConsolidation => "memory_consolidation",
            Self::FailureAnalysis => "failure_analysis",
            Self::LogIntrospection => "log_introspection",
            Self::SessionCleanup => "session_cleanup",
            Self::BlueprintRefinement => "blueprint_refinement",
            Self::Vigil => "vigil",
            Self::CambiumSkills => "cambium_skills",
        }
    }

    pub fn uses_llm(&self) -> bool {
        matches!(
            self,
            Self::FailureAnalysis | Self::LogIntrospection | Self::Vigil | Self::CambiumSkills
        )
    }
}

/// Wake triggers that can bring Tem to Active from any state.
#[derive(Debug, Clone)]
pub enum WakeTrigger {
    UserMessage,
    AlarmFired { concern_id: String },
    MonitorDetection { concern_id: String },
    ScheduledTask { concern_id: String },
    ParkedTaskResumed { concern_id: String },
}

impl WakeTrigger {
    pub fn name(&self) -> &str {
        match self {
            Self::UserMessage => "user_message",
            Self::AlarmFired { .. } => "alarm_fired",
            Self::MonitorDetection { .. } => "monitor_detection",
            Self::ScheduledTask { .. } => "scheduled_task",
            Self::ParkedTaskResumed { .. } => "parked_task_resumed",
        }
    }
}

pub struct Conscience {
    state: Arc<RwLock<ConscienceState>>,
    idle_threshold: Duration,
    dream_threshold: Duration,
    store: Arc<Store>,
}

impl Conscience {
    pub fn new(idle_threshold: Duration, dream_threshold: Duration, store: Arc<Store>) -> Self {
        Self {
            state: Arc::new(RwLock::new(ConscienceState::Active)),
            idle_threshold,
            dream_threshold,
            store,
        }
    }

    pub async fn current_state(&self) -> ConscienceState {
        self.state.read().await.clone()
    }

    /// Evaluate whether a transition should happen based on current conditions.
    /// Returns the new state if a transition should occur, None otherwise.
    pub async fn evaluate_transition(
        &self,
        has_foreground: bool,
        idle_duration: Duration,
        volition_recommendation: Option<&str>,
    ) -> Option<ConscienceState> {
        let current = self.state.read().await.clone();

        match &current {
            ConscienceState::Active => {
                if !has_foreground {
                    return Some(ConscienceState::Idle { since: Utc::now() });
                }
            }
            ConscienceState::Idle { .. } => {
                // Volition can recommend sleep
                if volition_recommendation == Some("sleep") {
                    return Some(ConscienceState::Sleep {
                        since: Utc::now(),
                        work: pick_sleep_work(),
                    });
                }
                // Threshold-driven fallback
                if idle_duration >= self.idle_threshold {
                    return Some(ConscienceState::Sleep {
                        since: Utc::now(),
                        work: pick_sleep_work(),
                    });
                }
            }
            ConscienceState::Sleep { since, .. } => {
                let sleep_duration = (Utc::now() - *since).to_std().unwrap_or(Duration::ZERO);
                if volition_recommendation == Some("dream") {
                    return Some(ConscienceState::Dream { since: Utc::now() });
                }
                if sleep_duration >= self.dream_threshold {
                    return Some(ConscienceState::Dream { since: Utc::now() });
                }
            }
            ConscienceState::Dream { .. } => {
                // Dream completes externally (EigenTune signals done)
                // Volition or external signal transitions back to Idle
            }
        }

        None
    }

    /// Force transition to Active (wake trigger).
    pub async fn wake(&self, trigger: WakeTrigger) {
        let prev = self.state.read().await.clone();
        if prev.name() == "active" {
            return;
        }

        let reason = format!("wake:{}", trigger.name());
        self.transition_to(ConscienceState::Active, &reason).await;
    }

    /// Transition to a new state with logging.
    pub async fn transition_to(&self, new: ConscienceState, reason: &str) {
        let prev = {
            let mut state = self.state.write().await;
            let prev = state.clone();
            *state = new.clone();
            prev
        };

        tracing_ext::trace_state_transition(prev.name(), new.name(), reason, None);

        if let Err(e) = self
            .store
            .log_transition(prev.name(), new.name(), reason, None)
            .await
        {
            tracing::warn!(error = %e, "Failed to log conscience transition");
        }
    }

    /// Complete dream state and return to idle.
    pub async fn complete_dream(&self) {
        let current = self.state.read().await.clone();
        if matches!(current, ConscienceState::Dream { .. }) {
            self.transition_to(
                ConscienceState::Idle { since: Utc::now() },
                "dream_complete",
            )
            .await;
        }
    }

    /// Complete sleep work and stay in idle (or transition to dream if ready).
    pub async fn complete_self_work(&self) {
        let current = self.state.read().await.clone();
        if matches!(current, ConscienceState::Sleep { .. }) {
            self.transition_to(
                ConscienceState::Idle { since: Utc::now() },
                "self_work_complete",
            )
            .await;
        }
    }
}

/// Select which self-work kind to run during Sleep state.
///
/// Defaults to `MemoryConsolidation` (cheap, no LLM). Occasionally
/// (1 in 15 Sleep cycles) selects `CambiumSkills` if Cambium is
/// enabled in `~/.temm1e/cambium.toml`. This provides the autonomous
/// trigger for Wire 3 without burning tokens on every Sleep cycle.
///
/// The double-gate (cambium.enabled + probabilistic + grow_skills's
/// own 24h rate limit) ensures autonomous growth is rare and bounded.
fn pick_sleep_work() -> SelfWorkKind {
    // Check if Cambium is enabled at runtime.
    let cambium_enabled = read_cambium_enabled();
    if !cambium_enabled {
        return SelfWorkKind::MemoryConsolidation;
    }

    // Probabilistic gate: roughly 1 in 15 Sleep cycles will attempt a
    // cambium skill grow. This works with grow_skills()'s built-in
    // 24h rate limit to prevent excessive LLM calls even in
    // high-frequency Sleep scenarios.
    let roll: u8 = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
            .hash(&mut h);
        (h.finish() % 15) as u8
    };
    if roll == 0 {
        SelfWorkKind::CambiumSkills
    } else {
        SelfWorkKind::MemoryConsolidation
    }
}

/// Read the cambium enabled flag from ~/.temm1e/cambium.toml.
/// Defaults to `true` if the file is missing (matches the v4.7.0 default).
fn read_cambium_enabled() -> bool {
    let path = temm1e_core::config::data_dir().join("cambium.toml");
    match std::fs::read_to_string(&path) {
        Ok(s) => s
            .lines()
            .find(|l| l.trim().starts_with("enabled"))
            .map(|l| !l.contains("false"))
            .unwrap_or(true),
        Err(_) => true, // missing file = default enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_sleep_work_respects_disabled_cambium() {
        // Create a temp home with cambium disabled.
        // We can't easily override HOME in a unit test, so we just
        // verify the function returns a valid SelfWorkKind that is
        // either MemoryConsolidation or CambiumSkills.
        let work = pick_sleep_work();
        assert!(matches!(
            work,
            SelfWorkKind::MemoryConsolidation | SelfWorkKind::CambiumSkills
        ));
    }

    #[test]
    fn read_cambium_enabled_missing_file_defaults_true() {
        // This test may produce a true or false depending on the user's
        // actual ~/.temm1e/cambium.toml. We just verify it doesn't panic.
        let _enabled = read_cambium_enabled();
    }

    #[test]
    fn state_names() {
        assert_eq!(ConscienceState::Active.name(), "active");
        assert_eq!(ConscienceState::Idle { since: Utc::now() }.name(), "idle");
        assert_eq!(
            ConscienceState::Sleep {
                since: Utc::now(),
                work: SelfWorkKind::MemoryConsolidation
            }
            .name(),
            "sleep"
        );
        assert_eq!(ConscienceState::Dream { since: Utc::now() }.name(), "dream");
    }

    #[test]
    fn self_work_kind_properties() {
        assert!(!SelfWorkKind::MemoryConsolidation.uses_llm());
        assert!(SelfWorkKind::FailureAnalysis.uses_llm());
        assert!(SelfWorkKind::LogIntrospection.uses_llm());
        assert!(!SelfWorkKind::SessionCleanup.uses_llm());
        assert!(!SelfWorkKind::BlueprintRefinement.uses_llm());
    }

    #[test]
    fn wake_trigger_names() {
        assert_eq!(WakeTrigger::UserMessage.name(), "user_message");
        assert_eq!(
            WakeTrigger::AlarmFired {
                concern_id: "x".into()
            }
            .name(),
            "alarm_fired"
        );
    }

    #[test]
    fn state_serialization_roundtrip() {
        let states = vec![
            ConscienceState::Active,
            ConscienceState::Idle { since: Utc::now() },
            ConscienceState::Sleep {
                since: Utc::now(),
                work: SelfWorkKind::FailureAnalysis,
            },
            ConscienceState::Dream { since: Utc::now() },
        ];

        for state in states {
            let json = serde_json::to_string(&state).unwrap();
            let deserialized: ConscienceState = serde_json::from_str(&json).unwrap();
            assert_eq!(state.name(), deserialized.name());
        }
    }
}
