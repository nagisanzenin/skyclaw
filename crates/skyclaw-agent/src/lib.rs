//! SkyClaw Agent crate — the core agent runtime that processes messages
//! through AI providers with tool execution support.

pub mod circuit_breaker;
pub mod context;
pub mod delegation;
pub mod done_criteria;
pub mod executor;
pub mod history_pruning;
pub mod learning;
pub mod model_router;
pub mod output_compression;
pub mod proactive;
pub mod prompt_optimizer;
pub mod prompt_patches;
pub mod recovery;
pub mod runtime;
pub mod self_correction;
pub mod startup;
pub mod streaming;
pub mod task_decomposition;
pub mod task_queue;
pub mod watchdog;

pub use circuit_breaker::CircuitBreaker;
pub use delegation::{DelegationManager, SubAgent, SubAgentResult, SubAgentStatus};
pub use done_criteria::DoneCriteria;
pub use executor::{detect_dependencies, execute_tools_parallel, ToolCall, ToolCallResult};
pub use history_pruning::{
    prune_history, score_message, MessageImportance, PrunedHistory, ScoredMessage,
};
pub use learning::TaskLearning;
pub use model_router::{ModelRouter, ModelRouterConfig, ModelTier, TaskComplexity};
pub use proactive::{ProactiveAction, ProactiveManager, Trigger, TriggerRule};
pub use prompt_optimizer::{build_system_prompt, estimate_prompt_tokens, SystemPromptBuilder};
pub use prompt_patches::{PatchStatus, PatchType, PromptPatch, PromptPatchManager};
pub use recovery::{RecoveryAction, RecoveryManager, RecoveryPlan};
pub use runtime::AgentRuntime;
pub use self_correction::FailureTracker;
pub use startup::{LazyResource, StartupMetrics};
pub use streaming::{StreamBuffer, StreamingConfig, StreamingNotifier};
pub use task_decomposition::{SubTask, SubTaskStatus, TaskGraph};
pub use task_queue::{TaskQueue, TaskStatus};
pub use watchdog::{HealthReport, SubsystemStatus, Watchdog, WatchdogConfig};
