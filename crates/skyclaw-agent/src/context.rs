//! Context builder — assembles a CompletionRequest from session history,
//! memory search results, system prompt, and tool definitions.
//!
//! Uses priority-based token budgeting to allocate the context window
//! surgically across categories:
//!   1. System prompt (always included)
//!   2. Tool definitions (always included)
//!   3. Current task state / DONE criteria (always included if present)
//!   4. Most recent 2–4 messages (always kept)
//!   5. Memory search results (up to 15% of budget)
//!   6. Cross-task learnings (up to 5% of budget)
//!   7. Older conversation history (fill remaining budget, newest first)
//!
//! When older messages are dropped, a brief summary is injected so the
//! LLM retains awareness of earlier context.

use std::sync::Arc;

use skyclaw_core::types::message::{
    ChatMessage, CompletionRequest, ContentPart, MessageContent, Role, ToolDefinition,
};
use skyclaw_core::types::session::SessionContext;
use skyclaw_core::{Memory, SearchOpts, Tool};
use tracing::debug;

use crate::learning;

/// Minimum number of recent messages to always keep in context.
const MIN_RECENT_MESSAGES: usize = 4;

/// Maximum number of recent messages to keep before applying budget.
const MAX_RECENT_MESSAGES: usize = 8;

/// Fraction of total budget reserved for memory search results.
const MEMORY_BUDGET_FRACTION: f32 = 0.15;

/// Fraction of total budget reserved for cross-task learnings.
const LEARNING_BUDGET_FRACTION: f32 = 0.05;

/// Estimate token count from a string (rough: 1 token ≈ 4 chars).
fn estimate_tokens(s: &str) -> usize {
    s.len() / 4
}

/// Approximate token cost per image for vision models.
const IMAGE_TOKEN_ESTIMATE: usize = 1000;

/// Estimate token count for a ChatMessage.
fn estimate_message_tokens(msg: &ChatMessage) -> usize {
    match &msg.content {
        MessageContent::Text(t) => estimate_tokens(t),
        MessageContent::Parts(parts) => parts
            .iter()
            .map(|p| match p {
                ContentPart::Text { text } => estimate_tokens(text),
                ContentPart::ToolUse { input, .. } => estimate_tokens(&input.to_string()),
                ContentPart::ToolResult { content, .. } => estimate_tokens(content),
                ContentPart::Image { .. } => IMAGE_TOKEN_ESTIMATE,
            })
            .sum(),
    }
}

/// Build a CompletionRequest from all available context using priority-based
/// token budgeting.
pub async fn build_context(
    session: &SessionContext,
    memory: &dyn Memory,
    tools: &[Arc<dyn Tool>],
    model: &str,
    system_prompt: Option<&str>,
    max_turns: usize,
    max_context_tokens: usize,
) -> CompletionRequest {
    let budget = max_context_tokens;

    // ── Category 1: System prompt ──────────────────────────────────
    let system = build_system_prompt(system_prompt, tools, session);
    let system_tokens = system.as_ref().map_or(0, |s| estimate_tokens(s));

    // ── Category 2: Tool definitions ───────────────────────────────
    let tool_defs: Vec<ToolDefinition> = tools
        .iter()
        .map(|t| ToolDefinition {
            name: t.name().to_string(),
            description: t.description().to_string(),
            parameters: t.parameters_schema(),
        })
        .collect();
    let tool_def_tokens: usize = tool_defs
        .iter()
        .map(|t| {
            estimate_tokens(&t.name)
                + estimate_tokens(&t.description)
                + estimate_tokens(&t.parameters.to_string())
        })
        .sum();

    // Fixed overhead (message framing, etc.)
    let overhead = 500;
    let fixed_tokens = system_tokens + tool_def_tokens + overhead;

    // ── Category 3: Task state / DONE criteria ─────────────────────
    // These are already in session.history as System messages injected by
    // the DONE Definition Engine. They will be included via the recent
    // messages or history pass, so we don't double-count them here.

    // ── Category 4: Recent messages (always kept) ──────────────────
    let history = &session.history;

    // Determine how many recent messages to keep (at least MIN, up to MAX)
    let recent_count = history
        .len()
        .min(MAX_RECENT_MESSAGES)
        .max(history.len().min(MIN_RECENT_MESSAGES));
    let recent_start = history.len().saturating_sub(recent_count);
    let recent_messages: Vec<ChatMessage> = history[recent_start..].to_vec();
    let recent_tokens: usize = recent_messages.iter().map(estimate_message_tokens).sum();

    let available_after_fixed_and_recent = budget.saturating_sub(fixed_tokens + recent_tokens);

    // ── Category 5: Memory search results (up to 15% of budget) ────
    let memory_budget = ((budget as f32) * MEMORY_BUDGET_FRACTION) as usize;
    let memory_budget = memory_budget.min(available_after_fixed_and_recent);

    let query = extract_latest_query(history);
    let mut memory_messages: Vec<ChatMessage> = Vec::new();
    let mut memory_tokens_used = 0;

    if !query.is_empty() {
        let opts = SearchOpts {
            limit: 5,
            session_filter: Some(session.session_id.clone()),
            ..Default::default()
        };

        if let Ok(entries) = memory.search(&query, opts).await {
            if !entries.is_empty() {
                let memory_text: String = entries
                    .iter()
                    .map(|e| format!("[{}] {}", e.timestamp.format("%Y-%m-%d %H:%M"), e.content))
                    .collect::<Vec<_>>()
                    .join("\n");

                let tokens = estimate_tokens(&memory_text) + 10; // +10 for prefix
                if tokens <= memory_budget {
                    memory_messages.push(ChatMessage {
                        role: Role::System,
                        content: MessageContent::Text(format!(
                            "Relevant context from memory:\n{}",
                            memory_text
                        )),
                    });
                    memory_tokens_used = tokens;
                }
            }
        }
    }

    // ── Category 6: Cross-task learnings (up to 5% of budget) ──────
    let learning_budget = ((budget as f32) * LEARNING_BUDGET_FRACTION) as usize;
    let remaining_for_learnings =
        available_after_fixed_and_recent.saturating_sub(memory_tokens_used);
    let learning_budget = learning_budget.min(remaining_for_learnings);

    let mut learning_messages: Vec<ChatMessage> = Vec::new();
    let mut learning_tokens_used = 0;

    if !query.is_empty() {
        // Search for past learnings stored with the "learning:" prefix
        let learning_opts = SearchOpts {
            limit: 5,
            session_filter: None, // learnings are cross-session
            ..Default::default()
        };

        if let Ok(entries) = memory.search("learning:", learning_opts).await {
            if !entries.is_empty() {
                // Parse learnings and format them
                let learnings: Vec<learning::TaskLearning> = entries
                    .iter()
                    .filter_map(|e| serde_json::from_str(&e.content).ok())
                    .collect();

                if !learnings.is_empty() {
                    let formatted = learning::format_learnings_context(&learnings);
                    let tokens = estimate_tokens(&formatted);
                    if tokens <= learning_budget && !formatted.is_empty() {
                        learning_messages.push(ChatMessage {
                            role: Role::System,
                            content: MessageContent::Text(formatted),
                        });
                        learning_tokens_used = tokens;
                    }
                }
            }
        }
    }

    // ── Category 7: Older conversation history ─────────────────────
    let used_tokens = fixed_tokens + recent_tokens + memory_tokens_used + learning_tokens_used;
    let history_budget = budget.saturating_sub(used_tokens);

    // Trim to max_turns first
    let older_end = recent_start;
    let older_history: Vec<ChatMessage> = if max_turns > 0 && older_end > max_turns * 2 {
        history[older_end - max_turns * 2..older_end].to_vec()
    } else {
        history[..older_end].to_vec()
    };

    // Walk from newest to oldest, accumulate until budget exceeded
    let mut kept_older: Vec<ChatMessage> = Vec::new();
    let mut older_tokens_used = 0;
    let mut dropped_count = 0;
    let dropped_total = older_history.len();

    for msg in older_history.iter().rev() {
        let msg_tokens = estimate_message_tokens(msg);
        if older_tokens_used + msg_tokens > history_budget {
            dropped_count = dropped_total - kept_older.len();
            break;
        }
        older_tokens_used += msg_tokens;
        kept_older.push(msg.clone());
    }
    kept_older.reverse();

    // If we dropped messages, inject a summary marker
    let mut summary_messages: Vec<ChatMessage> = Vec::new();
    if dropped_count > 0 {
        let summary = generate_dropped_summary(
            &history[..older_end.saturating_sub(kept_older.len())],
            dropped_count,
        );
        summary_messages.push(ChatMessage {
            role: Role::System,
            content: MessageContent::Text(summary),
        });
    }

    // ── Assemble final message list ────────────────────────────────
    // Order: summary → memory → learnings → older history → recent messages
    let mut messages: Vec<ChatMessage> = Vec::new();
    messages.extend(summary_messages);
    messages.extend(memory_messages);
    messages.extend(learning_messages);
    messages.extend(kept_older);
    messages.extend(recent_messages);

    let total_tokens = fixed_tokens + messages.iter().map(estimate_message_tokens).sum::<usize>();

    debug!(
        system = system_tokens,
        tools = tool_def_tokens,
        recent = recent_tokens,
        memory = memory_tokens_used,
        learnings = learning_tokens_used,
        history = older_tokens_used,
        total = total_tokens,
        budget = budget,
        dropped = dropped_count,
        "Context budget allocation"
    );

    CompletionRequest {
        model: model.to_string(),
        messages,
        tools: tool_defs,
        max_tokens: Some(4096),
        temperature: Some(0.7),
        system,
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Extract the latest user query text from history.
fn extract_latest_query(history: &[ChatMessage]) -> String {
    history
        .iter()
        .rev()
        .find_map(|m| match &m.content {
            MessageContent::Text(t) => Some(t.clone()),
            MessageContent::Parts(parts) => parts.iter().find_map(|p| match p {
                ContentPart::Text { text } => Some(text.clone()),
                _ => None,
            }),
        })
        .unwrap_or_default()
}

/// Generate a brief summary of dropped messages for context continuity.
fn generate_dropped_summary(dropped_msgs: &[ChatMessage], count: usize) -> String {
    // Extract tool names used in dropped context
    let mut tools_used: Vec<String> = Vec::new();
    let mut topics: Vec<String> = Vec::new();

    for msg in dropped_msgs {
        match &msg.content {
            MessageContent::Text(t) => {
                if matches!(msg.role, Role::User) && t.len() > 5 {
                    // Take first 50 chars as a topic hint
                    let topic = if t.len() > 50 { &t[..50] } else { t };
                    topics.push(topic.to_string());
                }
            }
            MessageContent::Parts(parts) => {
                for part in parts {
                    if let ContentPart::ToolUse { name, .. } = part {
                        if !tools_used.contains(name) {
                            tools_used.push(name.clone());
                        }
                    }
                }
            }
        }
    }

    let mut summary_parts = Vec::new();
    summary_parts.push(format!("[Earlier context: {} messages dropped", count));

    if !topics.is_empty() {
        let topic_str = topics
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        summary_parts.push(format!("discussed: {}", topic_str));
    }

    if !tools_used.is_empty() {
        summary_parts.push(format!("tools used: {}", tools_used.join(", ")));
    }

    format!("{}]", summary_parts.join(", "))
}

/// Build the system prompt, using a custom one or generating the default.
fn build_system_prompt(
    custom: Option<&str>,
    tools: &[Arc<dyn Tool>],
    session: &SessionContext,
) -> Option<String> {
    custom.map(|s| s.to_string()).or_else(|| {
        let tool_names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        Some(format!(
            "You are SkyClaw, a cloud-native AI agent runtime. You control a computer through messaging apps.\n\
             \n\
             You have access to these tools: {}\n\
             \n\
             Workspace: All file operations use the workspace directory at {}.\n\
             Files sent by the user are automatically saved here.\n\
             \n\
             File protocol:\n\
             - Received files are saved to the workspace automatically — use file_read to read them\n\
             - To send a file to the user, use send_file with just the path (chat_id is automatic)\n\
             - Use file_write to create files in the workspace, then send_file to deliver them\n\
             - Paths are relative to the workspace directory\n\
             \n\
             Guidelines:\n\
             - Use the shell tool to run commands, install packages, manage services, check system status\n\
             - Use file tools to read, write, and list files in the workspace\n\
             - Use web_fetch to look up documentation, check APIs, or research information\n\
             - Be concise in responses — the user is on a messaging app\n\
             - When a task requires multiple steps, execute them sequentially using tools\n\
             - If a command fails, read the error and try to fix it\n\
             - Never expose secrets, API keys, or sensitive data in responses\n\
             \n\
             Verification:\n\
             After every tool execution, you MUST verify the result before proceeding:\n\
             - Check that commands succeeded (exit code 0, expected output)\n\
             - Verify file operations by reading back what was written\n\
             - Test endpoints after deployment\n\
             - Never assume success — verify with evidence\n\
             \n\
             DONE criteria:\n\
             For compound tasks (multiple steps), define what DONE looks like before executing:\n\
             - List specific, verifiable conditions that must ALL be true when complete\n\
             - After completing all steps, verify each condition before declaring done\n\
             - Report completion with evidence for each condition\n\
             \n\
             Self-correction:\n\
             If an approach fails repeatedly, do NOT retry the same way:\n\
             - Analyze why the approach fails\n\
             - Generate alternative approaches\n\
             - Execute the most promising alternative\n\
             - If no alternatives exist, ask the user for guidance",
            tool_names.join(", "),
            session.workspace_path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use skyclaw_test_utils::{make_session, MockMemory, MockTool};

    #[tokio::test]
    async fn context_includes_system_prompt() {
        let memory = MockMemory::new();
        let tools: Vec<Arc<dyn Tool>> = vec![];
        let session = make_session();

        let req = build_context(
            &session,
            &memory,
            &tools,
            "test-model",
            Some("Custom prompt"),
            6,
            30_000,
        )
        .await;
        assert_eq!(req.system.as_deref(), Some("Custom prompt"));
        assert_eq!(req.model, "test-model");
    }

    #[tokio::test]
    async fn context_default_system_prompt() {
        let memory = MockMemory::new();
        let tools: Vec<Arc<dyn Tool>> = vec![];
        let session = make_session();

        let req = build_context(&session, &memory, &tools, "test-model", None, 6, 30_000).await;
        assert!(req.system.is_some());
        assert!(req.system.unwrap().contains("SkyClaw"));
    }

    #[tokio::test]
    async fn context_includes_tool_definitions() {
        let memory = MockMemory::new();
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(MockTool::new("shell")),
            Arc::new(MockTool::new("browser")),
        ];
        let session = make_session();

        let req = build_context(&session, &memory, &tools, "model", None, 6, 30_000).await;
        assert_eq!(req.tools.len(), 2);
        assert_eq!(req.tools[0].name, "shell");
        assert_eq!(req.tools[1].name, "browser");
    }

    #[tokio::test]
    async fn context_includes_conversation_history() {
        let memory = MockMemory::new();
        let tools: Vec<Arc<dyn Tool>> = vec![];
        let mut session = make_session();
        session.history.push(ChatMessage {
            role: Role::User,
            content: MessageContent::Text("Hello".to_string()),
        });
        session.history.push(ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Text("Hi there".to_string()),
        });

        let req = build_context(&session, &memory, &tools, "model", None, 6, 30_000).await;
        // Messages should include the history
        assert!(req.messages.len() >= 2);
    }

    #[tokio::test]
    async fn recent_messages_always_kept() {
        let memory = MockMemory::new();
        let tools: Vec<Arc<dyn Tool>> = vec![];
        let mut session = make_session();

        // Add many messages
        for i in 0..20 {
            session.history.push(ChatMessage {
                role: Role::User,
                content: MessageContent::Text(format!("Message {i}")),
            });
            session.history.push(ChatMessage {
                role: Role::Assistant,
                content: MessageContent::Text(format!("Reply {i}")),
            });
        }

        // Use a very small budget to force dropping older messages
        let req = build_context(&session, &memory, &tools, "model", None, 200, 2_000).await;

        // The most recent messages should always be present
        let last_msg = req.messages.last().unwrap();
        match &last_msg.content {
            MessageContent::Text(t) => assert!(t.contains("Reply 19")),
            _ => panic!("Expected text message"),
        }
    }

    #[tokio::test]
    async fn dropped_messages_generate_summary() {
        let memory = MockMemory::new();
        let tools: Vec<Arc<dyn Tool>> = vec![];
        let mut session = make_session();

        // Add many messages with enough content to exceed a small budget.
        // Each message is ~200 chars = ~50 tokens. 50 pairs = 100 messages = ~5000 tokens.
        let padding = "x".repeat(180);
        for i in 0..50 {
            session.history.push(ChatMessage {
                role: Role::User,
                content: MessageContent::Text(format!("User message {i}: {padding}")),
            });
            session.history.push(ChatMessage {
                role: Role::Assistant,
                content: MessageContent::Text(format!("Reply {i}: {padding}")),
            });
        }

        // Budget of 2000 tokens can't fit all 5000 tokens of messages + system prompt
        let req = build_context(&session, &memory, &tools, "model", None, 200, 2_000).await;

        // Check that a summary message was injected
        let has_summary = req.messages.iter().any(|m| {
            if let MessageContent::Text(t) = &m.content {
                t.contains("[Earlier context:")
            } else {
                false
            }
        });
        assert!(has_summary);
    }

    #[test]
    fn generate_dropped_summary_with_tools() {
        let msgs = vec![
            ChatMessage {
                role: Role::User,
                content: MessageContent::Text("Deploy the application to production".to_string()),
            },
            ChatMessage {
                role: Role::Assistant,
                content: MessageContent::Parts(vec![ContentPart::ToolUse {
                    id: "t1".to_string(),
                    name: "shell".to_string(),
                    input: serde_json::json!({}),
                }]),
            },
        ];
        let summary = generate_dropped_summary(&msgs, 5);
        assert!(summary.contains("5 messages dropped"));
        assert!(summary.contains("Deploy"));
        assert!(summary.contains("shell"));
    }

    #[test]
    fn generate_dropped_summary_empty() {
        let summary = generate_dropped_summary(&[], 0);
        assert!(summary.contains("0 messages dropped"));
    }
}
