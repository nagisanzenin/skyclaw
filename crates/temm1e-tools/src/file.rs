//! File tool — read, write, and list files within the session workspace.

use async_trait::async_trait;
use temm1e_core::types::error::Temm1eError;
use temm1e_core::{PathAccess, Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};

/// Maximum file read size (32 KB — keeps tool output within token budget).
const MAX_READ_SIZE: usize = 32 * 1024;

/// Default line limit for file_read (matches industry standard).
const DEFAULT_LINE_LIMIT: usize = 2000;

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
        "Read the contents of a file with line numbers. Supports offset and limit \
         for reading specific sections of large files. Returns line-numbered content. \
         Paths are relative to the workspace directory."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path to read (relative to workspace or absolute)"
                },
                "offset": {
                    "type": "integer",
                    "description": "Start line number (1-indexed, default: 1)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum lines to return (default: 2000)"
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
    ) -> Result<ToolOutput, Temm1eError> {
        let path_str = input
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Temm1eError::Tool("Missing required parameter: path".into()))?;

        let offset = input
            .arguments
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| v.max(1) as usize)
            .unwrap_or(1);

        let limit = input
            .arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_LINE_LIMIT);

        let path = resolve_path(path_str, &ctx.workspace_path)?;

        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                // Track this read for the read-before-write gate
                if let Some(ref tracker) = ctx.read_tracker {
                    tracker.write().await.insert(path.clone());
                }

                let lines: Vec<&str> = content.lines().collect();
                let total_lines = lines.len();

                // Apply offset (1-indexed) and limit
                let start = (offset - 1).min(total_lines);
                let end = (start + limit).min(total_lines);
                let selected = &lines[start..end];

                // Format with line numbers
                let mut output = String::new();
                for (i, line) in selected.iter().enumerate() {
                    let line_num = start + i + 1;
                    output.push_str(&format!("{}\t{}\n", line_num, line));
                }

                // Check byte size limit (safe UTF-8 boundary)
                if output.len() > MAX_READ_SIZE {
                    let mut end = MAX_READ_SIZE;
                    while end > 0 && !output.is_char_boundary(end) {
                        end -= 1;
                    }
                    output.truncate(end);
                    output.push_str("\n... [output truncated at 32KB]");
                }

                // Add metadata if partial read
                if end < total_lines {
                    output.push_str(&format!(
                        "\n[Showing lines {}-{} of {} total]",
                        offset, end, total_lines
                    ));
                }

                Ok(ToolOutput {
                    content: output,
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
    ) -> Result<ToolOutput, Temm1eError> {
        let path_str = input
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Temm1eError::Tool("Missing required parameter: path".into()))?;

        let content = input
            .arguments
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Temm1eError::Tool("Missing required parameter: content".into()))?;

        let path = resolve_path(path_str, &ctx.workspace_path)?;

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
    ) -> Result<ToolOutput, Temm1eError> {
        let path_str = input
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let path = resolve_path(path_str, &ctx.workspace_path)?;

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

/// Normalize a path by resolving `.` and `..` components without filesystem access.
fn normalize_path(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut result = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            other => result.push(other),
        }
    }
    result
}

/// Resolve a path string relative to the workspace directory.
///
/// All resolved paths are validated to remain within the workspace boundary.
/// Paths that escape the workspace via `..`, absolute paths, or `~` expansion
/// are rejected with an error.
pub(crate) fn resolve_path(
    path_str: &str,
    workspace: &std::path::Path,
) -> Result<std::path::PathBuf, Temm1eError> {
    let resolved = if path_str.starts_with("~/") || path_str == "~" {
        // Expand ~ to user's home directory
        let suffix = if path_str.len() > 2 {
            &path_str[2..]
        } else {
            ""
        };
        if let Some(home) = dirs::home_dir() {
            home.join(suffix)
        } else if let Ok(home) = std::env::var("HOME") {
            std::path::PathBuf::from(home).join(suffix)
        } else {
            workspace.join(path_str)
        }
    } else if path_str.starts_with("$HOME/") || path_str.starts_with("$HOME\\") {
        // Expand $HOME/... if used explicitly in path
        if let Ok(home) = std::env::var("HOME") {
            std::path::PathBuf::from(home).join(&path_str[6..])
        } else {
            workspace.join(path_str)
        }
    } else {
        let path = std::path::Path::new(path_str);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            workspace.join(path)
        }
    };

    // Canonicalize workspace (resolves symlinks like /var -> /private/var on macOS)
    let workspace_canonical = workspace
        .canonicalize()
        .unwrap_or_else(|_| normalize_path(workspace));

    // For existing paths, canonicalize to resolve symlinks and ..
    // For new paths (file_write), canonicalize the parent then append the filename.
    let resolved_canonical = if resolved.exists() {
        resolved
            .canonicalize()
            .unwrap_or_else(|_| normalize_path(&resolved))
    } else if let Some(parent) = resolved.parent() {
        let canonical_parent = if parent.exists() {
            parent
                .canonicalize()
                .unwrap_or_else(|_| normalize_path(parent))
        } else {
            normalize_path(parent)
        };
        match resolved.file_name() {
            Some(name) => canonical_parent.join(name),
            None => canonical_parent,
        }
    } else {
        normalize_path(&resolved)
    };

    if !resolved_canonical.starts_with(&workspace_canonical) {
        return Err(Temm1eError::Tool(format!(
            "Access denied: path '{}' is outside workspace '{}'",
            path_str,
            workspace.display()
        )));
    }

    Ok(resolved_canonical)
}
