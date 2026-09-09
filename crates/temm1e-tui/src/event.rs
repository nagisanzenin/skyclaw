//! Event types for the TUI event loop (TEA architecture).

use temm1e_agent::agent_task_status::AgentTaskStatus;
use temm1e_core::types::message::OutboundMessage;

/// All events the TUI can process.
#[derive(Debug)]
pub enum Event {
    /// Keyboard/mouse/resize from crossterm.
    Terminal(crossterm::event::Event),
    /// Agent status changed (via watch channel).
    AgentStatus(AgentTaskStatus),
    /// Ordered tool events; watch snapshots cannot serve as a history.
    ToolLifecycle(temm1e_agent::agent_task_status::AgentToolEvent),
    /// Streaming text chunk from the agent.
    TextLifecycle(temm1e_agent::agent_task_status::AgentTextEvent),
    /// User submitted input (from input widget).
    UserSubmit(String),
    /// Agent finished processing — final response.
    AgentResponse(AgentResponseEvent),
    /// Read-only committed transcript page; opaque provider reasoning is not rendered.
    HistoryPage {
        page: temm1e_agent::conversation::ConversationPage,
        reset: bool,
        completes_command: bool,
    },
    /// Tick for animations (spinner, elapsed time).
    Tick,
}

/// Agent response event with full message and usage info.
#[derive(Debug, Clone)]
pub struct AgentResponseEvent {
    pub kind: ResponseKind,
    pub message: OutboundMessage,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_usd: f64,
}

/// Lifecycle comes from the producer, never inferred from token usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseKind {
    Interim,
    Final,
    Failed,
    Notice,
}

/// Tool execution notification for the activity panel.
#[derive(Debug, Clone)]
pub enum ToolNotification {
    Started { name: String, args_summary: String },
    OutputLine { line: String },
    Completed { name: String, success: bool },
}
