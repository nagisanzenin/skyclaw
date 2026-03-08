//! File tool — read, write, and list files within the session workspace.

use async_trait::async_trait;
use skyclaw_core::types::error::SkyclawError;
use skyclaw_core::{PathAccess, Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};

/// Maximum file read size (32 KB — keeps tool output within token budget).
const MAX_READ_SIZE: usize = 32 * 1024;

#[derive(Default)]
pub struct FileReadTool;

impl FileReadTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Returns the text content. \
         Paths are relative to the workspace directory."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path to read (relative to workspace or absolute)"
                }
            },
            "required": ["path"]
        })
    }

    fn declarations(&self) -> ToolDeclarations {
        ToolDeclarations {
            file_access: vec![PathAccess::Read(".".into())],
            network_access: Vec::new(),
            shell_access: false,
        }
    }

    async fn execute(
        &self,
        input: ToolInput,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, SkyclawError> {
        let path_str = input
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SkyclawError::Tool("Missing required parameter: path".into()))?;

        let path = resolve_path(path_str, &ctx.workspace_path);

        match tokio::fs::read_to_string(&path).await {
            Ok(mut content) => {
                if content.len() > MAX_READ_SIZE {
                    content.truncate(MAX_READ_SIZE);
                    content.push_str("\n... [file truncated]");
                }
                Ok(ToolOutput {
                    content,
                    is_error: false,
                })
            }
            Err(e) => Ok(ToolOutput {
                content: format!("Failed to read file '{}': {}", path_str, e),
                is_error: true,
            }),
        }
    }
}

#[derive(Default)]
pub struct FileWriteTool;

impl FileWriteTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file if it doesn't exist, \
         overwrites if it does. Creates parent directories automatically. \
         Paths are relative to the workspace directory."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path to write (relative to workspace or absolute)"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    fn declarations(&self) -> ToolDeclarations {
        ToolDeclarations {
            file_access: vec![PathAccess::ReadWrite(".".into())],
            network_access: Vec::new(),
            shell_access: false,
        }
    }

    async fn execute(
        &self,
        input: ToolInput,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, SkyclawError> {
        let path_str = input
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SkyclawError::Tool("Missing required parameter: path".into()))?;

        let content = input
            .arguments
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SkyclawError::Tool("Missing required parameter: content".into()))?;

        let path = resolve_path(path_str, &ctx.workspace_path);

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Ok(ToolOutput {
                    content: format!("Failed to create directories for '{}': {}", path_str, e),
                    is_error: true,
                });
            }
        }

        match tokio::fs::write(&path, content).await {
            Ok(()) => Ok(ToolOutput {
                content: format!("Written {} bytes to '{}'", content.len(), path_str),
                is_error: false,
            }),
            Err(e) => Ok(ToolOutput {
                content: format!("Failed to write file '{}': {}", path_str, e),
                is_error: true,
            }),
        }
    }
}

#[derive(Default)]
pub struct FileListTool;

impl FileListTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FileListTool {
    fn name(&self) -> &str {
        "file_list"
    }

    fn description(&self) -> &str {
        "List files and directories at a given path. Returns names with type indicators \
         (/ for directories). Paths are relative to the workspace directory."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory path to list (relative to workspace or absolute). Defaults to workspace root."
                }
            },
            "required": []
        })
    }

    fn declarations(&self) -> ToolDeclarations {
        ToolDeclarations {
            file_access: vec![PathAccess::Read(".".into())],
            network_access: Vec::new(),
            shell_access: false,
        }
    }

    async fn execute(
        &self,
        input: ToolInput,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, SkyclawError> {
        let path_str = input
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let path = resolve_path(path_str, &ctx.workspace_path);

        match tokio::fs::read_dir(&path).await {
            Ok(mut entries) => {
                let mut items = Vec::new();
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
                    if is_dir {
                        items.push(format!("{}/", name));
                    } else {
                        items.push(name);
                    }
                }
                items.sort();
                if items.is_empty() {
                    Ok(ToolOutput {
                        content: format!("Directory '{}' is empty", path_str),
                        is_error: false,
                    })
                } else {
                    Ok(ToolOutput {
                        content: items.join("\n"),
                        is_error: false,
                    })
                }
            }
            Err(e) => Ok(ToolOutput {
                content: format!("Failed to list directory '{}': {}", path_str, e),
                is_error: true,
            }),
        }
    }
}

/// Resolve a path string relative to the workspace directory.
fn resolve_path(path_str: &str, workspace: &std::path::Path) -> std::path::PathBuf {
    let path = std::path::Path::new(path_str);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    }
}
