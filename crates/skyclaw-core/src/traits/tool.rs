use crate::types::error::SkyclawError;
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

/// Context provided to tools during execution
pub struct ToolContext {
    pub workspace_path: std::path::PathBuf,
    pub session_id: String,
    pub chat_id: String,
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
    async fn execute(
        &self,
        input: ToolInput,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, SkyclawError>;
}
