use crate::types::error::Temm1eError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Tool capability declarations — what resources a tool needs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDeclarations {
    /// File paths this tool needs access to
    pub file_access: Vec<PathAccess>,
    /// Network domains this tool needs to reach
    pub network_access: Vec<String>,
    /// Whether this tool needs shell execution
    pub shell_access: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PathAccess {
    Read(String),
    Write(String),
    ReadWrite(String),
}

/// Input to a tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInput {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Output from a tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

/// Image data produced by a tool execution (e.g., browser screenshot).
/// Used to feed vision data back to the LLM for visual reasoning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutputImage {
    /// MIME type (e.g., "image/png")
    pub media_type: String,
    /// Base64-encoded image data
    pub data: String,
}

/// Context provided to tools during execution
#[derive(Clone)]
pub struct ToolContext {
    /// Authenticated user identifier from the active session, never model input.
    pub user_id: String,
    /// Parent authority; delegated work must not replace it with Admin.
    pub role: crate::types::rbac::Role,
    /// Authenticated transport identity, carried from the active session.
    pub channel: String,
    pub workspace_path: std::path::PathBuf,
    pub session_id: String,
    pub chat_id: String,
    /// Tracks which files have been read in this session.
    /// Used by code_edit to enforce read-before-write gate.
    /// None for non-coding contexts (backwards compatible).
    pub read_tracker:
        Option<std::sync::Arc<tokio::sync::RwLock<std::collections::HashSet<std::path::PathBuf>>>>,
}

impl ToolContext {
    pub fn from_session(session: &crate::types::session::SessionContext) -> Self {
        Self {
            user_id: session.user_id.clone(),
            role: session.role,
            channel: session.channel.clone(),
            workspace_path: session.workspace_path.clone(),
            session_id: session.session_id.clone(),
            chat_id: session.chat_id.clone(),
            read_tracker: Some(session.read_tracker.clone()),
        }
    }

    /// Create isolated worker history/read tracking while retaining the caller's
    /// user, authority and workspace. The private worker route is not delivery.
    pub fn delegated_session(
        &self,
        channel: &str,
        session_id: String,
    ) -> crate::types::session::SessionContext {
        crate::types::session::SessionContext {
            user_id: self.user_id.clone(),
            role: self.role,
            channel: channel.into(),
            chat_id: session_id.clone(),
            session_id,
            workspace_path: self.workspace_path.clone(),
            history: Vec::new(),
            read_tracker: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashSet::new(),
            )),
        }
    }
}

/// Tool trait — agent capabilities like shell, file ops, browser, etc.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Tool name (e.g., "shell", "browser", "file_read")
    fn name(&self) -> &str;

    /// Human-readable description for the AI model
    fn description(&self) -> &str;

    /// JSON Schema for tool parameters
    fn parameters_schema(&self) -> serde_json::Value;

    /// What resources this tool needs (for sandboxing enforcement)
    fn declarations(&self) -> ToolDeclarations;

    /// Execute the tool with given input
    async fn execute(&self, input: ToolInput, ctx: &ToolContext)
        -> Result<ToolOutput, Temm1eError>;

    /// Consume image data produced by the last execution.
    /// Called by the runtime after execute() to inject vision data into the
    /// conversation. Default: returns None (most tools produce no images).
    fn take_last_image(&self) -> Option<ToolOutputImage> {
        None
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;
    #[test]
    fn delegation_retains_identity_authority_workspace_but_isolates_history_and_reads() {
        for role in [
            crate::types::rbac::Role::User,
            crate::types::rbac::Role::Admin,
        ] {
            let parent = crate::types::session::SessionContext {
                session_id: "parent".into(),
                channel: "telegram".into(),
                chat_id: "room".into(),
                user_id: "caller-alice".into(),
                role,
                workspace_path: "/caller-workspace".into(),
                history: Vec::new(),
                read_tracker: std::sync::Arc::new(tokio::sync::RwLock::new(
                    std::collections::HashSet::from([std::path::PathBuf::from("read-by-parent")]),
                )),
            };
            let context = ToolContext::from_session(&parent);
            let worker = context.delegated_session("hive", "private-worker".into());
            assert_eq!(worker.user_id, parent.user_id);
            assert_eq!(worker.role, role);
            assert_eq!(worker.workspace_path, parent.workspace_path);
            assert_eq!(worker.chat_id, "private-worker");
            assert_eq!(worker.channel, "hive");
            assert!(worker.history.is_empty());
            assert!(worker.read_tracker.try_read().unwrap().is_empty());
            assert!(!std::sync::Arc::ptr_eq(
                &worker.read_tracker,
                &parent.read_tracker
            ));
        }
    }
}
