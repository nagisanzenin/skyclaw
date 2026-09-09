//! JIT Swarm — `spawn_swarm` tool for mid-flight parallel work.
//!
//! Exposes Hive's `maybe_decompose` + `execute_order` as a tool the main
//! agent can call when it discovers N independent subtasks during its own
//! investigation. Workers run with:
//! - Fresh AgentRuntime per task (isolated budget, no parent history)
//! - Tool filter excluding `spawn_swarm` (recursion block)
//! - SharedContext injected as the worker's first user message
//! - Parent's BudgetTracker receives aggregated swarm usage exactly once
//!
//! The tool returns the aggregated text from Hive; the main agent synthesizes
//! the user-facing reply from the tool result in its next turn.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use temm1e_core::types::error::Temm1eError;
use temm1e_core::types::message::InboundMessage;
use temm1e_core::{Memory, Provider, Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::budget::BudgetTracker;
use crate::runtime::AgentRuntime;

/// Name of the JIT-swarm tool. Exact-match filtered out of worker toolsets
/// by the worker's tool filter to prevent nested recursion.
pub const SPAWN_SWARM_TOOL_NAME: &str = "spawn_swarm";

/// Environment variable workers set to signal "I am inside a swarm, don't
/// offer spawn_swarm". Redundant with the tool filter (defence in depth).
pub const IN_SWARM_ENV: &str = "TEMM1E_IN_SWARM";

/// Per-worker limits for AgentRuntime::with_limits.
/// Workers are lightweight (no session history, no full prompt stack) so
/// these values are tuned for compact, bounded subtask work.
const WORKER_MAX_TURNS: usize = 10;
const WORKER_MAX_CONTEXT_TOKENS: usize = 30_000;
/// 0 = unlimited (matches post-P4 behaviour). Workers still respect parent
/// BudgetTracker through per-call child accounting before further admission.
const WORKER_MAX_TOOL_ROUNDS: usize = 0;
/// Hard wall-clock cap per worker (5 minutes) — independent of parent agent.
const WORKER_MAX_TASK_DURATION: u64 = 300;

#[derive(Debug, Deserialize)]
struct SpawnArgs {
    goal: String,
    shared_context: String,
    #[serde(default)]
    subtasks: Option<Vec<SubtaskSpec>>,
}

#[derive(Debug, Deserialize)]
struct SubtaskSpec {
    /// v1: description parses but isn't yet routed through Hive's Queen
    /// (all subtasks still go through the Queen decomposition path). Retained
    /// in the schema for forward compatibility when accept_explicit_subtasks
    /// lands in Hive's public API.
    #[allow(dead_code)]
    description: String,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    writes_files: Vec<String>,
}

/// Runtime context the spawn_swarm tool needs to actually fire. Populated
/// asynchronously after Hive + provider + memory + tools are all ready.
/// When None, the tool returns a "swarm not available yet" message so the
/// model knows to continue with its own tools.
#[derive(Clone)]
pub struct SpawnSwarmContext {
    pub hive: Arc<temm1e_hive::Hive>,
    pub provider: Arc<dyn Provider>,
    pub memory: Arc<dyn Memory>,
    pub tools_template: Vec<Arc<dyn Tool>>,
    pub model: String,
    pub parent_budget: Arc<BudgetTracker>,
    pub policy: crate::runtime_policy::RuntimePolicy,
    pub cancel: CancellationToken,
    /// Legacy composition hint retained for source compatibility. The actual
    /// invocation's ToolContext workspace is authoritative for each worker.
    pub workspace_path: std::path::PathBuf,
    /// Parent's Witness attachments. When Some, each JIT swarm worker
    /// is constructed with `.with_witness_attachments(...)` so Oath
    /// sealing + verification happens per-worker-turn just like the
    /// main agent. None = JIT workers run without Witness oversight.
    pub witness_attachments: Option<crate::witness_init::WitnessAttachments>,
}

/// Shared handle to the swarm context. The tool is registered early in
/// tool-creation with an Arc<RwLock<Option<...>>> that's filled in later
/// once Hive + provider wiring is complete.
pub type SwarmHandle = Arc<tokio::sync::RwLock<Option<SpawnSwarmContext>>>;

/// The spawn_swarm tool. Reads its context from a shared handle at execute
/// time — this decouples tool registration (early) from dependency wiring
/// (later in startup, after Hive is initialized).
pub struct SpawnSwarmTool {
    handle: SwarmHandle,
    resources: Option<temm1e_core::runtime_resources::RuntimeResources>,
}

impl SpawnSwarmTool {
    pub fn new(handle: SwarmHandle) -> Self {
        Self {
            handle,
            resources: None,
        }
    }

    /// Convenience: create a fresh shared handle populated with nothing.
    /// The startup wiring fills this in via `handle.write().await = Some(ctx)`
    /// once dependencies are ready.
    pub fn fresh_handle() -> SwarmHandle {
        Arc::new(tokio::sync::RwLock::new(None))
    }
}

#[async_trait]
impl Tool for SpawnSwarmTool {
    fn bind_runtime(
        &self,
        resources: &temm1e_core::runtime_resources::RuntimeResources,
    ) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            handle: self.handle.clone(),
            resources: Some(resources.clone()),
        }))
    }

    fn name(&self) -> &str {
        SPAWN_SWARM_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Spawn parallel worker Tems to handle N independent subtasks in parallel. \
         Use ONLY when you have identified multiple units of work with no sequential \
         dependency. Each worker receives the shared_context you provide plus its \
         individual task description. Returns aggregated text from all workers; \
         you compose the final user-facing reply from it in your next turn.\n\n\
         Arguments:\n\
         - goal (required): one-sentence description of the overall goal.\n\
         - shared_context (required): everything workers need to know that you \
           have already discovered (files read, key findings, conventions, constraints). \
           Workers start blank — this is their only inheritance. Keep under 2000 tokens.\n\
         - subtasks (optional): your own decomposition into independent subtasks. \
           If provided, the Queen decomposition step is skipped. Each subtask has \
           {description, depends_on?, writes_files?}. If two subtasks write the \
           same file, one must depend_on the other.\n\n\
         Call this ONLY when:\n\
         (a) you can enumerate ≥2 truly independent units of work, AND\n\
         (b) running them in parallel is meaningfully faster than sequential."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["goal", "shared_context"],
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "Overall user-facing goal this swarm is serving."
                },
                "shared_context": {
                    "type": "string",
                    "description": "Discoveries and context from your investigation so far — what workers need to know."
                },
                "subtasks": {
                    "type": "array",
                    "description": "Optional explicit subtasks. When provided, skips the Queen decomposition LLM call.",
                    "items": {
                        "type": "object",
                        "required": ["description"],
                        "properties": {
                            "description": {"type": "string"},
                            "depends_on": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "IDs of other subtasks (0-indexed as strings, e.g. \"0\", \"1\") that must complete first."
                            },
                            "writes_files": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "Files this subtask will write — used for collision detection."
                            }
                        }
                    }
                }
            }
        })
    }

    fn declarations(&self) -> ToolDeclarations {
        ToolDeclarations {
            file_access: vec![],
            network_access: vec![],
            shell_access: false,
        }
    }

    async fn execute(
        &self,
        input: ToolInput,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let args: SpawnArgs = serde_json::from_value(input.arguments)
            .map_err(|e| Temm1eError::Tool(format!("spawn_swarm: invalid arguments: {e}")))?;

        // Pull the live context (Hive, provider, etc.). If the runtime hasn't
        // finished wiring yet, return a graceful fallback so the model can
        // continue with its own tools instead of crashing.
        let ctx_guard = self.handle.read().await;
        let mut swarm_ctx = match ctx_guard.as_ref() {
            Some(c) => c.clone(),
            None => {
                return Ok(ToolOutput {
                    content: "Swarm not available yet (Hive not initialized). \
                              Continue with your own sequential tools."
                        .into(),
                    is_error: false,
                });
            }
        };

        drop(ctx_guard);
        if let Some(resources) = &self.resources {
            swarm_ctx.provider = resources.provider.clone();
            swarm_ctx.memory = resources.memory.clone();
            swarm_ctx.model = resources.model.clone();
            swarm_ctx.parent_budget = resources.budget.clone();
            swarm_ctx.policy = resources.policy.clone();
        }

        // Writer-exclusion advisory check — reject obvious collisions up front
        // so the model can retry with a sequential decomposition.
        if let Some(ref subtasks) = args.subtasks {
            if let Some(collision) = detect_writer_collisions(subtasks) {
                return Ok(ToolOutput {
                    content: format!(
                        "spawn_swarm rejected: subtasks {collision} both write the same \
                         file. Either sequence them via `depends_on`, or serialize the \
                         work yourself instead of spawning a swarm."
                    ),
                    is_error: true,
                });
            }
        }

        // Build the worker execute_fn closure.
        let provider = swarm_ctx.provider.clone();
        let memory = swarm_ctx.memory.clone();
        let tools_template = swarm_ctx.tools_template.clone();
        let model = swarm_ctx.model.clone();
        let witness_attachments_for_closure = swarm_ctx.witness_attachments.clone();
        let parent_context = ctx.clone();
        let shared_context = args.shared_context.clone();
        let parent_budget = swarm_ctx.parent_budget.clone();
        let policy = swarm_ctx.policy.clone();

        let execute_fn = Arc::new(
            move |task: temm1e_hive::types::HiveTask, dep_results: Vec<(String, String)>| {
                let provider = provider.clone();
                let memory = memory.clone();
                let tools = tools_template.clone();
                let model = model.clone();
                let shared_context = shared_context.clone();
                let witness_for_worker = witness_attachments_for_closure.clone();
                let parent_context = parent_context.clone();
                let worker_budget = Arc::new(BudgetTracker::child(parent_budget.clone()));
                let policy = policy.clone();
                async move {
                    // Tool filter: strip spawn_swarm so the worker can't recurse.
                    let filter: crate::runtime::ToolFilter =
                        Arc::new(|t: &dyn Tool| t.name() != SPAWN_SWARM_TOOL_NAME);

                    let worker = AgentRuntime::with_limits(
                        provider.clone(),
                        memory.clone(),
                        tools,
                        model.clone(),
                        None,
                        WORKER_MAX_TURNS,
                        WORKER_MAX_CONTEXT_TOKENS,
                        WORKER_MAX_TOOL_ROUNDS,
                        WORKER_MAX_TASK_DURATION,
                        0.0,
                    )
                    .with_budget(worker_budget)
                    .with_policy(&policy)
                    .with_tool_filter(filter)
                    .with_witness_attachments(witness_for_worker.as_ref());

                    let deps_text = format_dep_results(&dep_results);
                    let initial_msg = format!(
                        "## Context from parent Tem\n{shared_context}\n\n\
                         ## Your task\n{}\n\n\
                         ## Results from dependency tasks\n{deps_text}",
                        task.description,
                    );

                    let mut session = parent_context
                        .delegated_session("jit-swarm", format!("jit-swarm-{}", task.id));
                    let inbound = InboundMessage {
                        id: uuid::Uuid::new_v4().to_string(),
                        chat_id: session.chat_id.clone(),
                        user_id: session.user_id.clone(),
                        username: None,
                        channel: session.channel.clone(),
                        text: Some(initial_msg),
                        attachments: vec![],
                        reply_to: None,
                        timestamp: chrono::Utc::now(),
                    };

                    match worker
                        .process_message(&inbound, &mut session, None, None, None, None, None)
                        .await
                    {
                        Ok((reply, usage)) => {
                            let snap = worker.budget_snapshot();
                            Ok(temm1e_hive::worker::TaskResult {
                                summary: reply.text,
                                tokens_used: usage.combined_tokens(),
                                input_tokens: snap.input_tokens,
                                output_tokens: snap.output_tokens,
                                cost_usd: snap.cost_usd,
                                artifacts: vec![],
                                success: true,
                                error: None,
                            })
                        }
                        Err(e) => {
                            let snap = worker.budget_snapshot();
                            Ok(temm1e_hive::worker::TaskResult {
                                summary: String::new(),
                                tokens_used: snap
                                    .input_tokens
                                    .saturating_add(snap.output_tokens)
                                    .min(u64::from(u32::MAX))
                                    as u32,
                                input_tokens: snap.input_tokens,
                                output_tokens: snap.output_tokens,
                                cost_usd: snap.cost_usd,
                                artifacts: vec![],
                                success: false,
                                error: Some(e.to_string()),
                            })
                        }
                    }
                }
            },
        );

        // Decompose via Queen. For v1, we always route through Queen even
        // when the caller provides explicit subtasks. `accept_explicit_subtasks`
        // is a planned v2 Hive API addition.
        if args.subtasks.is_some() {
            info!("spawn_swarm: caller provided subtasks, but Queen still runs (v1 behaviour)");
        }
        let order_id = match Self::decompose_with_provider(&swarm_ctx, &args.goal).await? {
            Some(oid) => oid,
            None => {
                return Ok(ToolOutput {
                    content: "Swarm not beneficial for this task (speedup \
                              threshold not met OR Queen cost too high). \
                              Continue with your own sequential tools."
                        .into(),
                    is_error: false,
                })
            }
        };

        // Execute the swarm.
        let swarm_result = swarm_ctx
            .hive
            .execute_order(&order_id, swarm_ctx.cancel.clone(), move |task, deps| {
                let exec = execute_fn.clone();
                async move { exec(task, deps).await }
            })
            .await?;

        // Workers now propagate typed usage/unknownness to the parent per
        // call. Re-adding the final scalar aggregate here would double-charge.

        info!(
            order_id = %order_id,
            completed = swarm_result.tasks_completed,
            escalated = swarm_result.tasks_escalated,
            workers = swarm_result.workers_used,
            wall_ms = swarm_result.wall_clock_ms,
            input_tokens = swarm_result.total_input_tokens,
            output_tokens = swarm_result.total_output_tokens,
            cost_usd = format!("{:.6}", swarm_result.total_cost_usd),
            "spawn_swarm completed"
        );

        let content = format!(
            "Swarm completed: {} tasks ({} escalated) in {}ms across {} workers.\n\n\
             Aggregated results:\n{}",
            swarm_result.tasks_completed,
            swarm_result.tasks_escalated,
            swarm_result.wall_clock_ms,
            swarm_result.workers_used,
            swarm_result.text,
        );

        Ok(ToolOutput {
            content,
            is_error: false,
        })
    }
}

impl SpawnSwarmTool {
    /// Run Queen decomposition via the parent provider.
    async fn decompose_with_provider(
        swarm_ctx: &SpawnSwarmContext,
        goal: &str,
    ) -> Result<Option<String>, Temm1eError> {
        let provider: Arc<dyn Provider> = Arc::new(crate::metered_provider::MeteredProvider::new(
            swarm_ctx.provider.clone(),
            swarm_ctx.parent_budget.clone(),
        ));
        let model = swarm_ctx.model.clone();
        let provider_call = move |prompt: String| {
            let provider = provider.clone();
            let model = model.clone();
            async move {
                let request = temm1e_core::types::message::CompletionRequest {
                    model,
                    messages: vec![temm1e_core::types::message::ChatMessage {
                        role: temm1e_core::types::message::Role::User,
                        content: temm1e_core::types::message::MessageContent::Text(prompt),
                    }],
                    tools: vec![],
                    // Per project rule (feedback_no_max_tokens): no hardcoded caps.
                    max_tokens: None,
                    temperature: Some(0.0),
                    system: None,
                    system_volatile: None,
                };
                match provider.complete(request).await {
                    Ok(resp) => {
                        // Extract the text content.
                        let text = resp
                            .content
                            .iter()
                            .filter_map(|p| match p {
                                temm1e_core::types::message::ContentPart::Text { text } => {
                                    Some(text.clone())
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        let total_tokens = u64::from(resp.usage.input_tokens)
                            + u64::from(resp.usage.output_tokens);
                        Ok((text, total_tokens))
                    }
                    Err(e) => Err(e),
                }
            }
        };
        swarm_ctx
            .hive
            .maybe_decompose(goal, "jit-swarm", provider_call)
            .await
    }
}

fn format_dep_results(deps: &[(String, String)]) -> String {
    if deps.is_empty() {
        "(no dependency results)".into()
    } else {
        deps.iter()
            .map(|(id, summary)| format!("- [{id}]: {summary}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Return `Some("i and j")` if subtasks i and j both declare overlapping
/// `writes_files` without one depending on the other. `None` if no collision.
fn detect_writer_collisions(subtasks: &[SubtaskSpec]) -> Option<String> {
    for i in 0..subtasks.len() {
        for j in (i + 1)..subtasks.len() {
            let a = &subtasks[i];
            let b = &subtasks[j];
            let overlap: Vec<&String> = a
                .writes_files
                .iter()
                .filter(|f| b.writes_files.contains(f))
                .collect();
            if overlap.is_empty() {
                continue;
            }
            let i_str = i.to_string();
            let j_str = j.to_string();
            let a_waits = a.depends_on.contains(&j_str);
            let b_waits = b.depends_on.contains(&i_str);
            if !a_waits && !b_waits {
                warn!(
                    subtask_a = i,
                    subtask_b = j,
                    overlap = ?overlap,
                    "spawn_swarm: detected writer-file collision"
                );
                return Some(format!("{i} and {j}"));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_collision_no_overlap() {
        let subtasks = vec![
            SubtaskSpec {
                description: "a".into(),
                depends_on: vec![],
                writes_files: vec!["a.rs".into()],
            },
            SubtaskSpec {
                description: "b".into(),
                depends_on: vec![],
                writes_files: vec!["b.rs".into()],
            },
        ];
        assert!(detect_writer_collisions(&subtasks).is_none());
    }

    #[test]
    fn detect_collision_unsequenced_overlap() {
        let subtasks = vec![
            SubtaskSpec {
                description: "a".into(),
                depends_on: vec![],
                writes_files: vec!["shared.rs".into()],
            },
            SubtaskSpec {
                description: "b".into(),
                depends_on: vec![],
                writes_files: vec!["shared.rs".into()],
            },
        ];
        let collision = detect_writer_collisions(&subtasks);
        assert!(collision.is_some());
        assert!(collision.unwrap().contains("0"));
    }

    #[test]
    fn detect_collision_sequenced_overlap_ok() {
        // When subtask 1 depends_on 0, they don't run in parallel → no collision.
        let subtasks = vec![
            SubtaskSpec {
                description: "a".into(),
                depends_on: vec![],
                writes_files: vec!["shared.rs".into()],
            },
            SubtaskSpec {
                description: "b".into(),
                depends_on: vec!["0".into()],
                writes_files: vec!["shared.rs".into()],
            },
        ];
        assert!(detect_writer_collisions(&subtasks).is_none());
    }

    #[test]
    fn spawn_swarm_tool_name_constant() {
        assert_eq!(SPAWN_SWARM_TOOL_NAME, "spawn_swarm");
    }

    #[test]
    fn format_dep_results_empty() {
        assert_eq!(format_dep_results(&[]), "(no dependency results)");
    }

    #[test]
    fn format_dep_results_populated() {
        let deps = vec![
            ("task1".to_string(), "done".to_string()),
            ("task2".to_string(), "result".to_string()),
        ];
        let text = format_dep_results(&deps);
        assert!(text.contains("task1"));
        assert!(text.contains("task2"));
        assert!(text.contains("done"));
    }
    struct ContextProbe {
        name: &'static str,
        calls: std::sync::atomic::AtomicUsize,
        seen: tokio::sync::Mutex<Option<ToolContext>>,
    }
    #[async_trait::async_trait]
    impl Tool for ContextProbe {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "local identity test probe"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object","properties":{}})
        }
        fn declarations(&self) -> ToolDeclarations {
            ToolDeclarations {
                file_access: vec![],
                network_access: vec![],
                shell_access: false,
            }
        }
        async fn execute(
            &self,
            _: ToolInput,
            context: &ToolContext,
        ) -> Result<ToolOutput, Temm1eError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *self.seen.lock().await = Some(context.clone());
            let content =
                tokio::fs::read_to_string(context.workspace_path.join("caller-marker.txt"))
                    .await
                    .map_err(|e| Temm1eError::Tool(e.to_string()))?;
            Ok(ToolOutput {
                content,
                is_error: false,
            })
        }
    }

    #[tokio::test]
    async fn real_jit_worker_inherits_user_role_and_invocation_workspace() {
        use temm1e_test_utils::{MockMemory, QueuedMockProvider};
        for bind_current in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let workspace = directory.path().join("caller-workspace");
            std::fs::create_dir(&workspace).unwrap();
            std::fs::write(workspace.join("caller-marker.txt"), "caller-owned-marker").unwrap();
            let config = temm1e_hive::config::HiveConfig {
                min_workers: 1,
                max_workers: 1,
                swarm_threshold_speedup: 1.0,
                queen_cost_ratio_max: 1.0,
                ..Default::default()
            };
            let hive = Arc::new(
                temm1e_hive::Hive::new(
                    &config,
                    &format!(
                        "sqlite://{}?mode=rwc",
                        directory.path().join("hive.db").display()
                    ),
                )
                .await
                .unwrap(),
            );
            let provider = Arc::new(QueuedMockProvider::with_responses(vec![
                QueuedMockProvider::text_response(
                    r#"{"tasks":[{"id":"t1","description":"Inspect caller identity","dependencies":[],"context_tags":[],"estimated_tokens":2000},{"id":"t2","description":"Inspect again","dependencies":["t1"],"context_tags":[],"estimated_tokens":2000}],"single_agent_recommended":false,"reasoning":"fixture"}"#,
                ),
                QueuedMockProvider::tool_use_response(
                    "forged-shell",
                    "shell",
                    serde_json::json!({}),
                ),
                QueuedMockProvider::tool_use_response(
                    "inspect-context",
                    "identity_probe",
                    serde_json::json!({}),
                ),
                QueuedMockProvider::text_response("worker returned"),
                QueuedMockProvider::tool_use_response(
                    "inspect-second-context",
                    "identity_probe",
                    serde_json::json!({}),
                ),
                QueuedMockProvider::text_response("second worker returned"),
            ]));
            let probe = Arc::new(ContextProbe {
                name: "identity_probe",
                calls: std::sync::atomic::AtomicUsize::new(0),
                seen: tokio::sync::Mutex::new(None),
            });
            let shell = Arc::new(ContextProbe {
                name: "shell",
                calls: std::sync::atomic::AtomicUsize::new(0),
                seen: tokio::sync::Mutex::new(None),
            });
            let mut policy_config = temm1e_core::types::config::Temm1eConfig::default();
            policy_config.agent.v2_optimizations = false;
            policy_config.memory.engram.enabled = false;
            let obsolete = Arc::new(QueuedMockProvider::with_responses(vec![]));
            let obsolete_budget = Arc::new(BudgetTracker::new(0.0));
            let current_budget = Arc::new(BudgetTracker::new(0.0));
            let handle = Arc::new(tokio::sync::RwLock::new(Some(SpawnSwarmContext {
                hive,
                provider: if bind_current {
                    obsolete.clone()
                } else {
                    provider.clone()
                },
                memory: Arc::new(MockMemory::new()),
                tools_template: vec![probe.clone(), shell.clone()],
                model: if bind_current {
                    "obsolete".into()
                } else {
                    "fixture".into()
                },
                parent_budget: obsolete_budget.clone(),
                policy: crate::runtime_policy::RuntimePolicy::from_config(&policy_config),
                cancel: CancellationToken::new(),
                workspace_path: directory.path().join("wrong-composition-workspace"),
                witness_attachments: None,
            })));
            let template = SpawnSwarmTool::new(handle.clone());
            let tool: Arc<dyn Tool> = if bind_current {
                template
                    .bind_runtime(&temm1e_core::runtime_resources::RuntimeResources {
                        provider: provider.clone(),
                        memory: Arc::new(MockMemory::new()),
                        budget: current_budget.clone(),
                        model: "current-model".into(),
                        pricing: crate::budget::ModelPricing::Unknown,
                        max_context_tokens: 30000,
                        policy: crate::runtime_policy::RuntimePolicy::from_config(&policy_config),
                    })
                    .unwrap()
            } else {
                Arc::new(template)
            };
            let context = ToolContext {
                user_id: "caller-alice".into(),
                role: temm1e_core::types::rbac::Role::User,
                channel: "telegram".into(),
                workspace_path: workspace.clone(),
                session_id: "parent-session".into(),
                chat_id: "parent-room".into(),
                read_tracker: None,
            };
            let result = tokio::time::timeout(std::time::Duration::from_secs(10), tool.execute(ToolInput {
            name: SPAWN_SWARM_TOOL_NAME.into(), arguments: serde_json::json!({"goal":"inspect identity", "shared_context":"local fixture"}),
        }, &context)).await.unwrap().unwrap();
            assert!(!result.is_error, "{}", result.content);
            assert_eq!(shell.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
            assert_eq!(probe.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
            let seen = probe.seen.lock().await;
            let seen = seen.as_ref().unwrap();
            assert_eq!(seen.user_id, "caller-alice");
            assert_eq!(seen.role, temm1e_core::types::rbac::Role::User);
            assert_eq!(seen.workspace_path, workspace);
            assert_ne!(seen.chat_id, context.chat_id);
            if bind_current {
                assert_eq!(obsolete.calls().await, 0);
                assert_eq!(obsolete_budget.snapshot().recorded_calls, 0);
                assert!(current_budget.snapshot().recorded_calls > 0);
                assert!(provider
                    .captured_requests
                    .lock()
                    .await
                    .iter()
                    .all(|r| r.model == "current-model"));
                let original = handle.read().await;
                assert_eq!(
                    original.as_ref().unwrap().model,
                    "obsolete",
                    "binding mutated shared startup state"
                );
            }
        }
    }
}
