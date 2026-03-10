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
use skyclaw_core::types::optimization::PromptTier;
use skyclaw_core::types::session::SessionContext;
use skyclaw_core::MemoryEntryType;
use skyclaw_core::{Memory, SearchOpts, Tool};
use tracing::{debug, warn};

use crate::learning;
use crate::prompt_optimizer::build_tiered_system_prompt;
use crate::runtime::model_supports_vision;

/// Minimum number of recent messages to always keep in context.
const MIN_RECENT_MESSAGES: usize = 30;

/// Maximum number of recent messages to keep before applying budget.
const MAX_RECENT_MESSAGES: usize = 60;

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
#[allow(clippy::too_many_arguments)]
pub async fn build_context(
    session: &SessionContext,
    memory: &dyn Memory,
    tools: &[Arc<dyn Tool>],
    model: &str,
    system_prompt: Option<&str>,
    max_turns: usize,
    max_context_tokens: usize,
    prompt_tier: Option<PromptTier>,
) -> CompletionRequest {
    let budget = max_context_tokens;

    // ── Category 1: System prompt ──────────────────────────────────
    let system = match prompt_tier {
        Some(tier) if system_prompt.is_none() => {
            // V2: Use tiered prompt from prompt_optimizer
            let config = skyclaw_core::types::config::AgentConfig::default();
            let tool_refs: Vec<&dyn Tool> = tools.iter().map(|t| t.as_ref()).collect();
            Some(build_tiered_system_prompt(
                &config,
                &tool_refs,
                &session.workspace_path,
                false, // done_criteria handled separately
                tier,
            ))
        }
        _ => build_system_prompt(system_prompt, tools, session),
    };
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

    // ── Category 5b: Persistent knowledge entries (auto-inject) ────
    // Inject top knowledge entries so the agent has context from previous
    // conversations without needing to explicitly recall them.
    let mut knowledge_messages: Vec<ChatMessage> = Vec::new();
    let mut knowledge_tokens_used = 0;
    {
        let knowledge_budget = memory_budget.saturating_sub(memory_tokens_used).min(2000);
        let knowledge_opts = SearchOpts {
            limit: 10,
            entry_type_filter: Some(MemoryEntryType::Knowledge),
            ..Default::default()
        };

        match memory.search("", knowledge_opts).await {
            Err(e) => {
                debug!(error = %e, "Knowledge search failed");
            }
            Ok(entries) => {
                debug!(
                    count = entries.len(),
                    knowledge_budget = knowledge_budget,
                    "Knowledge search returned entries"
                );
                let knowledge_entries: Vec<_> = entries
                    .iter()
                    .filter(|e| matches!(e.entry_type, MemoryEntryType::Knowledge))
                    .collect();
                if !knowledge_entries.is_empty() {
                    let knowledge_text: String = knowledge_entries
                        .iter()
                        .map(|e| {
                            let key = e
                                .metadata
                                .get("user_key")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?");
                            format!("- {}: {}", key, e.content)
                        })
                        .collect::<Vec<_>>()
                        .join("\n");

                    let tokens = estimate_tokens(&knowledge_text) + 10;
                    if tokens <= knowledge_budget && !knowledge_text.is_empty() {
                        knowledge_messages.push(ChatMessage {
                            role: Role::System,
                            content: MessageContent::Text(format!(
                                "=== YOUR PERSISTENT KNOWLEDGE ===\n\
                             These are facts you previously saved with memory_manage:\n\
                             {}\n\
                             === END KNOWLEDGE ===",
                                knowledge_text
                            )),
                        });
                        knowledge_tokens_used = tokens;
                    }
                }
            }
        }
    }

    // ── Category 6: Cross-task learnings (up to 5% of budget) ──────
    let learning_budget = ((budget as f32) * LEARNING_BUDGET_FRACTION) as usize;
    let remaining_for_learnings =
        available_after_fixed_and_recent.saturating_sub(memory_tokens_used + knowledge_tokens_used);
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
    let used_tokens = fixed_tokens
        + recent_tokens
        + memory_tokens_used
        + knowledge_tokens_used
        + learning_tokens_used;
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

    // ── Chat History Digest ────────────────────────────────────────
    // Extract a clean User ↔ Assistant conversation thread from the
    // full history (which is dominated by tool outputs). This is injected
    // as a System message so the LLM never loses track of what the human
    // actually said, even when tool outputs consume most of the context.
    let all_messages_for_digest: Vec<&ChatMessage> =
        kept_older.iter().chain(recent_messages.iter()).collect();
    let chat_digest = build_chat_digest(&all_messages_for_digest);

    // ── Assemble final message list ────────────────────────────────
    // Order: summary → chat digest → knowledge → memory → learnings → older history → recent messages
    let mut messages: Vec<ChatMessage> = Vec::new();
    messages.extend(summary_messages);
    if let Some(digest_msg) = chat_digest {
        messages.push(digest_msg);
    }
    messages.extend(knowledge_messages);
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
        knowledge = knowledge_tokens_used,
        learnings = learning_tokens_used,
        history = older_tokens_used,
        total = total_tokens,
        budget = budget,
        dropped = dropped_count,
        "Context budget allocation"
    );

    // ── Vision safety: strip image parts for non-vision models ─────
    // If the model doesn't support vision, remove all ImageUrl parts
    // from every message (including old history) so the provider never
    // receives unsupported content types.
    if !model_supports_vision(model) {
        let mut stripped = 0usize;
        for msg in &mut messages {
            if let MessageContent::Parts(parts) = &mut msg.content {
                let before = parts.len();
                parts.retain(|p| !matches!(p, ContentPart::Image { .. }));
                stripped += before - parts.len();
                // If only text parts remain, flatten to Text for cleanliness
                if parts.len() == 1 {
                    if let Some(ContentPart::Text { text }) = parts.first().cloned() {
                        msg.content = MessageContent::Text(text);
                    }
                }
            }
        }
        if stripped > 0 {
            warn!(
                model = model,
                images_stripped = stripped,
                "Stripped image parts from conversation history — model has no vision"
            );
        }
    }

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

/// Build a chat history digest that separates human conversation from tool
/// execution logs. Returns `None` if there are fewer than 2 user messages
/// (no point summarizing a single exchange).
///
/// The digest extracts User and Assistant TEXT messages only, ignoring tool
/// calls, tool results, system injections, and images. This gives the LLM
/// a clean "what did the human say and what did I reply" view that doesn't
/// get buried under shell outputs, browser HTML, and file contents.
fn build_chat_digest(messages: &[&ChatMessage]) -> Option<ChatMessage> {
    let mut entries: Vec<String> = Vec::new();
    let mut user_count = 0;

    for msg in messages {
        let role_label = match msg.role {
            Role::User => {
                user_count += 1;
                "User"
            }
            Role::Assistant => "Assistant",
            _ => continue, // Skip System, Tool
        };

        // Extract text content only (skip tool_use, tool_result, images)
        let text = match &msg.content {
            MessageContent::Text(t) => t.clone(),
            MessageContent::Parts(parts) => {
                let texts: Vec<&str> = parts
                    .iter()
                    .filter_map(|p| match p {
                        ContentPart::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                if texts.is_empty() {
                    continue; // Skip messages that are pure tool_use / tool_result
                }
                texts.join(" ")
            }
        };

        // Skip empty or trivial messages
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Truncate long assistant replies to keep the digest compact
        let display = if role_label == "Assistant" && trimmed.len() > 200 {
            // Find a char boundary at or before byte 200
            let end = trimmed
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| i <= 200)
                .last()
                .unwrap_or(0);
            format!("{}...", &trimmed[..end])
        } else {
            trimmed.to_string()
        };

        entries.push(format!("{}: {}", role_label, display));
    }

    // Not worth injecting if the conversation is trivial
    if user_count < 2 {
        return None;
    }

    // Cap digest to last 30 exchanges to keep token cost bounded
    let max_entries = 30;
    let start = entries.len().saturating_sub(max_entries);
    let digest_text = entries[start..].join("\n");

    Some(ChatMessage {
        role: Role::System,
        content: MessageContent::Text(format!(
            "=== CHAT HISTORY (human conversation thread) ===\n\
             Below is the User ↔ Assistant conversation WITHOUT tool outputs.\n\
             Use this to stay grounded in what the user asked and what you replied.\n\
             The full tool execution logs follow in the message history below.\n\
             \n\
             {}\n\
             \n\
             === END CHAT HISTORY ===",
            digest_text
        )),
    })
}

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
                    // Take first ~50 bytes as a topic hint (safe on char boundary)
                    let topic = if t.len() > 50 {
                        let end = t
                            .char_indices()
                            .map(|(i, _)| i)
                            .take_while(|&i| i <= 50)
                            .last()
                            .unwrap_or(0);
                        &t[..end]
                    } else {
                        t
                    };
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
            None,
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

        let req = build_context(
            &session,
            &memory,
            &tools,
            "test-model",
            None,
            6,
            30_000,
            None,
        )
        .await;
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

        let req = build_context(&session, &memory, &tools, "model", None, 6, 30_000, None).await;
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

        let req = build_context(&session, &memory, &tools, "model", None, 6, 30_000, None).await;
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
        let req = build_context(&session, &memory, &tools, "model", None, 200, 2_000, None).await;

        // The most recent messages should always be present
        let last_msg = req.messages.last().expect("messages should not be empty");
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
        let req = build_context(&session, &memory, &tools, "model", None, 200, 2_000, None).await;

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

    #[test]
    fn chat_digest_extracts_user_assistant_only() {
        let m1 = ChatMessage {
            role: Role::User,
            content: MessageContent::Text("Deploy the app".to_string()),
        };
        let m2 = ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Parts(vec![ContentPart::ToolUse {
                id: "t1".to_string(),
                name: "shell".to_string(),
                input: serde_json::json!({"command": "docker build ."}),
            }]),
        };
        let m3 = ChatMessage {
            role: Role::Tool,
            content: MessageContent::Parts(vec![ContentPart::ToolResult {
                tool_use_id: "t1".to_string(),
                content: "Successfully built image abc123\nStep 1/10 : FROM node:20\n...lots of output...".to_string(),
                is_error: false,
            }]),
        };
        let m4 = ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Text("Done! The app is deployed.".to_string()),
        };
        let m5 = ChatMessage {
            role: Role::User,
            content: MessageContent::Text("Great, now check the logs".to_string()),
        };

        let refs: Vec<&ChatMessage> = vec![&m1, &m2, &m3, &m4, &m5];
        let digest = build_chat_digest(&refs);
        assert!(digest.is_some());

        let text = match &digest
            .expect("digest should be Some for multi-message input")
            .content
        {
            MessageContent::Text(t) => t.clone(),
            _ => panic!("Expected text"),
        };

        // Should contain user and assistant text
        assert!(text.contains("User: Deploy the app"));
        assert!(text.contains("Assistant: Done! The app is deployed."));
        assert!(text.contains("User: Great, now check the logs"));

        // Should NOT contain tool output
        assert!(!text.contains("docker build"));
        assert!(!text.contains("Successfully built"));
        assert!(!text.contains("abc123"));

        // Should have the section headers
        assert!(text.contains("CHAT HISTORY"));
    }

    #[test]
    fn chat_digest_skips_pure_tool_use_messages() {
        // An assistant message that is ONLY tool_use (no text) should be skipped
        let m1 = ChatMessage {
            role: Role::User,
            content: MessageContent::Text("Run ls".to_string()),
        };
        let m2 = ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Parts(vec![ContentPart::ToolUse {
                id: "t1".to_string(),
                name: "shell".to_string(),
                input: serde_json::json!({"command": "ls"}),
            }]),
        };
        let m3 = ChatMessage {
            role: Role::User,
            content: MessageContent::Text("Now run pwd".to_string()),
        };

        let refs: Vec<&ChatMessage> = vec![&m1, &m2, &m3];
        let digest = build_chat_digest(&refs);
        assert!(digest.is_some());

        let text = match &digest
            .expect("digest should be Some for multi-message input")
            .content
        {
            MessageContent::Text(t) => t.clone(),
            _ => panic!("Expected text"),
        };

        // Should have user messages but no tool_use content
        assert!(text.contains("User: Run ls"));
        assert!(text.contains("User: Now run pwd"));
        assert!(!text.contains("shell"));
    }

    #[test]
    fn chat_digest_none_for_single_user_message() {
        let m1 = ChatMessage {
            role: Role::User,
            content: MessageContent::Text("Hello".to_string()),
        };

        let refs: Vec<&ChatMessage> = vec![&m1];
        assert!(build_chat_digest(&refs).is_none());
    }

    #[test]
    fn chat_digest_truncates_long_assistant_replies() {
        let long_reply = "A".repeat(500);
        let m1 = ChatMessage {
            role: Role::User,
            content: MessageContent::Text("Question 1".to_string()),
        };
        let m2 = ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Text(long_reply),
        };
        let m3 = ChatMessage {
            role: Role::User,
            content: MessageContent::Text("Question 2".to_string()),
        };

        let refs: Vec<&ChatMessage> = vec![&m1, &m2, &m3];
        let digest =
            build_chat_digest(&refs).expect("digest should be Some for multi-message input");

        let text = match &digest.content {
            MessageContent::Text(t) => t.clone(),
            _ => panic!("Expected text"),
        };

        // Assistant reply should be truncated to ~200 chars + "..."
        assert!(text.contains("..."));
        // But should NOT contain the full 500-char reply
        assert!(!text.contains(&"A".repeat(500)));
    }

    #[tokio::test]
    async fn context_includes_chat_digest_when_enough_messages() {
        let memory = MockMemory::new();
        let tools: Vec<Arc<dyn Tool>> = vec![];
        let mut session = make_session();

        // Simulate a realistic conversation with tool calls interleaved
        for i in 0..5 {
            session.history.push(ChatMessage {
                role: Role::User,
                content: MessageContent::Text(format!("User request {i}")),
            });
            session.history.push(ChatMessage {
                role: Role::Assistant,
                content: MessageContent::Parts(vec![ContentPart::ToolUse {
                    id: format!("t{i}"),
                    name: "shell".to_string(),
                    input: serde_json::json!({"command": format!("cmd {i}")}),
                }]),
            });
            session.history.push(ChatMessage {
                role: Role::Tool,
                content: MessageContent::Parts(vec![ContentPart::ToolResult {
                    tool_use_id: format!("t{i}"),
                    content: format!("output line {i}\nmore output\neven more output"),
                    is_error: false,
                }]),
            });
            session.history.push(ChatMessage {
                role: Role::Assistant,
                content: MessageContent::Text(format!("Done with task {i}")),
            });
        }

        let req = build_context(&session, &memory, &tools, "model", None, 200, 100_000, None).await;

        // Should have a chat digest in the messages
        let has_digest = req.messages.iter().any(|m| {
            if let MessageContent::Text(t) = &m.content {
                t.contains("CHAT HISTORY")
            } else {
                false
            }
        });
        assert!(has_digest, "Expected chat digest in context messages");
    }
}
