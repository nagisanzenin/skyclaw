//! InvokeCoreTool — the Tool trait implementation that bridges TemDOS cores
//! into the main agent's tool loop.
//!
//! When the main agent calls `invoke_core`, this tool:
//! 1. Looks up the core definition in the registry
//! 2. Filters `invoke_core` out of the tool list (recursion prevention)
//! 3. Substitutes `<task>` and `<context>` placeholders in the system prompt
//! 4. Constructs a CoreRuntime and runs it to completion
//! 5. Returns the core's output as a ToolOutput

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use temm1e_agent::budget::{BudgetTracker, ModelPricing};
use temm1e_core::types::error::Temm1eError;
use temm1e_core::{Memory, Provider, Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};
use tracing::{debug, info, warn};

use crate::registry::CoreRegistry;
use crate::runtime::CoreRuntime;
use crate::types::CoreStats;

/// The `invoke_core` tool — invokes a TemDOS specialist core from the main
/// agent's tool loop.
pub struct InvokeCoreTool {
    /// Core registry (behind RwLock for hot-loading).
    registry: Arc<RwLock<CoreRegistry>>,
    /// Shared AI provider.
    provider: Arc<dyn Provider>,
    /// All tools from the main agent (invoke_core is filtered at invocation time).
    all_tools: Vec<Arc<dyn Tool>>,
    /// Shared budget tracker.
    budget: Arc<BudgetTracker>,
    /// Model pricing for cost calculation.
    model_pricing: ModelPricing,
    /// Model name to use for cores.
    model: String,
    /// Maximum context tokens for core LLM calls.
    max_context_tokens: usize,
    /// Memory backend for persisting core stats (v4.6.0 self-learning).
    memory: Arc<dyn Memory>,
}

impl InvokeCoreTool {
    /// Create a new InvokeCoreTool.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: Arc<RwLock<CoreRegistry>>,
        provider: Arc<dyn Provider>,
        all_tools: Vec<Arc<dyn Tool>>,
        budget: Arc<BudgetTracker>,
        model_pricing: ModelPricing,
        model: String,
        max_context_tokens: usize,
        memory: Arc<dyn Memory>,
    ) -> Self {
        Self {
            registry,
            provider,
            all_tools,
            budget,
            model_pricing,
            model,
            max_context_tokens,
            memory,
        }
    }

    /// Build the filtered tool list — all tools EXCEPT invoke_core.
    ///
    /// This is the structural recursion prevention: cores cannot invoke
    /// other cores because the invoke_core tool is not in their tool set.
    fn filtered_tools(&self) -> Vec<Arc<dyn Tool>> {
        self.all_tools
            .iter()
            .filter(|t| t.name() != "invoke_core")
            .cloned()
            .collect()
    }
}

#[async_trait]
impl Tool for InvokeCoreTool {
    fn name(&self) -> &str {
        "invoke_core"
    }

    fn description(&self) -> &str {
        "Invoke a TemDOS specialist core. Cores are independent AI agents with \
         full tool access that run until completion and return a detailed answer. \
         Use for complex analysis tasks that would take many tool rounds."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "core": {
                    "type": "string",
                    "description": "Name of the core to invoke (e.g., 'architecture', 'security', 'test', 'debug', 'research', 'creative')"
                },
                "task": {
                    "type": "string",
                    "description": "The specific task or question for the core. Be detailed and specific — the core cannot ask follow-up questions."
                },
                "context": {
                    "type": "string",
                    "description": "Optional. Relevant context: conversation excerpts, previous core findings, constraints, or pre-read file contents. Reduces cold start."
                }
            },
            "required": ["core", "task"]
        })
    }

    fn declarations(&self) -> ToolDeclarations {
        // Cores inherit full tool access from the main agent
        ToolDeclarations {
            file_access: Vec::new(),
            network_access: Vec::new(),
            shell_access: true,
        }
    }

    async fn execute(
        &self,
        input: ToolInput,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        // Parse arguments
        let core_name = input
            .arguments
            .get("core")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Temm1eError::Tool("Missing required 'core' parameter".to_string()))?
            .to_string();

        let task = input
            .arguments
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Temm1eError::Tool("Missing required 'task' parameter".to_string()))?
            .to_string();

        let context = input
            .arguments
            .get("context")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Look up core definition
        let registry = self.registry.read().await;
        let core_def = registry.get_core(&core_name).ok_or_else(|| {
            let available: Vec<_> = registry
                .list_cores()
                .iter()
                .map(|c| c.name.clone())
                .collect();
            Temm1eError::Tool(format!(
                "Unknown core '{}'. Available cores: {}",
                core_name,
                available.join(", ")
            ))
        })?;

        // Substitute placeholders in system prompt
        let mut system_prompt = core_def.system_prompt.clone();
        system_prompt = system_prompt.replace("<task>", &task);
        if !context.is_empty() {
            system_prompt = system_prompt.replace("<context>", &context);
        } else {
            system_prompt = system_prompt.replace("<context>", "(no additional context provided)");
        }

        let temperature = core_def.temperature.unwrap_or(0.0);
        let core_name_owned = core_def.name.clone();

        // Drop registry lock before running the core
        drop(registry);

        info!(core = %core_name_owned, "TemDOS invoking core");

        // Build filtered tools (no invoke_core — recursion prevention)
        let core_tools = self.filtered_tools();

        // Construct and run the core runtime
        let runtime = CoreRuntime::new(
            core_name_owned.clone(),
            system_prompt,
            self.provider.clone(),
            core_tools,
            self.budget.clone(),
            self.model_pricing,
            self.model.clone(),
            self.max_context_tokens,
            temperature,
        );

        // Load existing core stats
        let stats_id = format!("core_stats:{}", core_name_owned);
        let mut stats: CoreStats = self
            .memory
            .search(
                &stats_id,
                temm1e_core::SearchOpts {
                    limit: 1,
                    ..Default::default()
                },
            )
            .await
            .ok()
            .and_then(|entries| {
                entries
                    .first()
                    .and_then(|e| serde_json::from_str(&e.content).ok())
            })
            .unwrap_or_default();

        // Use the actual working directory for the core so it can find project files.
        // Fall back to the tool context workspace, then the configured workspace.
        let core_workspace = std::env::current_dir().unwrap_or_else(|_| ctx.workspace_path.clone());
        let run_result = runtime.run(&task, core_workspace).await;

        // Record outcome in stats
        match &run_result {
            Ok(r) => stats.record_success(r.rounds, r.cost_usd),
            Err(_) => stats.record_failure(0.0),
        }

        // Persist updated stats (fire-and-forget)
        let entry = temm1e_core::MemoryEntry {
            id: stats_id,
            content: serde_json::to_string(&stats).unwrap_or_default(),
            entry_type: temm1e_core::MemoryEntryType::Knowledge,
            metadata: serde_json::json!({
                "type": "core_stats",
                "core_name": core_name_owned,
            }),
            timestamp: chrono::Utc::now(),
            session_id: None,
        };
        if let Err(e) = self.memory.store(entry).await {
            warn!(error = %e, core = %core_name_owned, "Failed to persist core stats");
        } else {
            debug!(
                core = %core_name_owned,
                success_rate = stats.success_rate(),
                invocations = stats.invocations,
                "Core stats updated"
            );
        }

        let result = run_result?;

        // Format output with metadata + stats
        let output = format!(
            "{}\n\n---\n[TemDOS:{} | {} rounds | ${:.4} | lifetime: {:.0}% success (N={})]",
            result.output,
            core_name_owned,
            result.rounds,
            result.cost_usd,
            stats.success_rate() * 100.0,
            stats.invocations,
        );

        Ok(ToolOutput {
            content: output,
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtered_tools_excludes_invoke_core() {
        // Create a minimal mock tool named "invoke_core"
        struct MockInvokeCore;

        #[async_trait]
        impl Tool for MockInvokeCore {
            fn name(&self) -> &str {
                "invoke_core"
            }
            fn description(&self) -> &str {
                "mock"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            fn declarations(&self) -> ToolDeclarations {
                ToolDeclarations {
                    file_access: Vec::new(),
                    network_access: Vec::new(),
                    shell_access: false,
                }
            }
            async fn execute(
                &self,
                _input: ToolInput,
                _ctx: &ToolContext,
            ) -> Result<ToolOutput, Temm1eError> {
                unreachable!()
            }
        }

        struct MockOtherTool;

        #[async_trait]
        impl Tool for MockOtherTool {
            fn name(&self) -> &str {
                "shell"
            }
            fn description(&self) -> &str {
                "mock shell"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            fn declarations(&self) -> ToolDeclarations {
                ToolDeclarations {
                    file_access: Vec::new(),
                    network_access: Vec::new(),
                    shell_access: true,
                }
            }
            async fn execute(
                &self,
                _input: ToolInput,
                _ctx: &ToolContext,
            ) -> Result<ToolOutput, Temm1eError> {
                unreachable!()
            }
        }

        let all_tools: Vec<Arc<dyn Tool>> = vec![Arc::new(MockInvokeCore), Arc::new(MockOtherTool)];

        let tool = InvokeCoreTool {
            registry: Arc::new(RwLock::new(CoreRegistry::new())),
            provider: Arc::new(MockProvider),
            all_tools,
            budget: Arc::new(BudgetTracker::new(10.0)),
            model_pricing: ModelPricing::custom(3.0, 15.0),
            model: "test".to_string(),
            max_context_tokens: 30_000,
            memory: Arc::new(MockMemory),
        };

        let filtered = tool.filtered_tools();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name(), "shell");
    }

    // Minimal mock memory for compile
    struct MockMemory;

    #[async_trait]
    impl Memory for MockMemory {
        async fn store(&self, _entry: temm1e_core::MemoryEntry) -> Result<(), Temm1eError> {
            Ok(())
        }
        async fn search(
            &self,
            _query: &str,
            _opts: temm1e_core::SearchOpts,
        ) -> Result<Vec<temm1e_core::MemoryEntry>, Temm1eError> {
            Ok(vec![])
        }
        async fn get(&self, _id: &str) -> Result<Option<temm1e_core::MemoryEntry>, Temm1eError> {
            Ok(None)
        }
        async fn delete(&self, _id: &str) -> Result<(), Temm1eError> {
            Ok(())
        }
        async fn list_sessions(&self) -> Result<Vec<String>, Temm1eError> {
            Ok(vec![])
        }
        async fn get_session_history(
            &self,
            _session_id: &str,
            _limit: usize,
        ) -> Result<Vec<temm1e_core::MemoryEntry>, Temm1eError> {
            Ok(vec![])
        }
        fn backend_name(&self) -> &str {
            "mock"
        }
    }

    // Minimal mock provider for compile
    struct MockProvider;

    #[async_trait]
    impl Provider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }
        async fn complete(
            &self,
            _request: temm1e_core::types::message::CompletionRequest,
        ) -> Result<temm1e_core::types::message::CompletionResponse, Temm1eError> {
            Err(Temm1eError::Provider("Mock".to_string()))
        }
        async fn stream(
            &self,
            _request: temm1e_core::types::message::CompletionRequest,
        ) -> Result<
            futures::stream::BoxStream<
                '_,
                Result<temm1e_core::types::message::StreamChunk, Temm1eError>,
            >,
            Temm1eError,
        > {
            Err(Temm1eError::Provider("Mock".to_string()))
        }
        async fn health_check(&self) -> Result<bool, Temm1eError> {
            Ok(true)
        }
        async fn list_models(&self) -> Result<Vec<String>, Temm1eError> {
            Ok(vec![])
        }
    }
}
