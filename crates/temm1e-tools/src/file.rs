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

        let path = resolve_path(path_str, &ctx.workspace_path, Operation::Read)?;

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
                let line_end = (start + limit).min(total_lines);
                let selected = &lines[start..line_end];

                // Format with line numbers
                let mut body = String::new();
                for (i, line) in selected.iter().enumerate() {
                    let line_num = start + i + 1;
                    body.push_str(&format!("{}\t{}\n", line_num, line));
                }

                // Check byte size limit (safe UTF-8 boundary). When the
                // truncation fires, recompute the actual last line emitted
                // by counting newlines in the truncated buffer — this keeps
                // the header/footer offset hint mathematically correct even
                // when the cap fires mid-line.
                let mut byte_capped = false;
                if body.len() > MAX_READ_SIZE {
                    let mut cut = MAX_READ_SIZE;
                    while cut > 0 && !body.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    body.truncate(cut);
                    byte_capped = true;
                }

                // Compute the actual last line number that appears in the
                // output. For line-limit truncation this is `line_end`. For
                // byte-cap truncation we count complete newlines in the
                // truncated body (an incomplete trailing line doesn't count).
                let actual_end_line = if byte_capped {
                    let nl_count = body.matches('\n').count();
                    start + nl_count
                } else {
                    line_end
                };

                let truncated = byte_capped || line_end < total_lines;

                // Build output: optional header + body + optional footer.
                // Header lets weak models notice truncation BEFORE reading
                // 32KB of content; footer keeps backward-compatible signal
                // for any tooling that scans for it.
                let mut output = String::new();
                if truncated {
                    output.push_str(&format!(
                        "[TRUNCATED — showing lines {}-{} of {} total. To continue, call file_read with offset={}]\n",
                        start + 1,
                        actual_end_line,
                        total_lines,
                        actual_end_line + 1,
                    ));
                }
                output.push_str(&body);
                if truncated {
                    if byte_capped {
                        output.push_str("\n... [output truncated at 32KB]");
                    }
                    output.push_str(&format!(
                        "\n[Showing lines {}-{} of {} total]",
                        start + 1,
                        actual_end_line,
                        total_lines
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

        let path = resolve_path(path_str, &ctx.workspace_path, Operation::Write)?;

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

        let path = resolve_path(path_str, &ctx.workspace_path, Operation::Read)?;

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

/// Operation type for `resolve_path`. Writes are checked against the
/// catastrophic-path block list; reads are not (the OS gates reads via
/// Unix permissions, and reading a file does not brick anything).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Read,
    Write,
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

/// Resolve a path string into a canonical absolute path.
///
/// Tem is designed for full computer use on the user's behalf. This function
/// does NOT enforce a workspace boundary — Tem can read and write anywhere
/// the user's UID can reach. The OS handles permission gating.
///
/// For `Operation::Write`, the resolved path is checked against the
/// catastrophic-write block list (system bootloader, auth databases, raw
/// disk devices, the running Tem binary, etc.) defined in
/// [`crate::file_safety`]. Catastrophic writes return an error.
///
/// `~/` and `$HOME/` are expanded. Relative paths are resolved against the
/// `workspace` parameter (typically the current working directory).
pub(crate) fn resolve_path(
    path_str: &str,
    workspace: &std::path::Path,
    op: Operation,
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

    // Block catastrophic writes (system bootloader, auth db, disk devices,
    // running Tem binary, watchdog binary). Reads are never blocked here.
    if op == Operation::Write {
        if let Some(reason) = crate::file_safety::is_catastrophic_write(&resolved_canonical) {
            tracing::warn!(
                path = %resolved_canonical.display(),
                reason = reason,
                "Blocked catastrophic file write"
            );
            return Err(Temm1eError::Tool(format!(
                "Refusing write to '{}': {reason}. \
                 If you are absolutely certain, perform this operation manually outside Tem.",
                resolved_canonical.display()
            )));
        }
    }

    Ok(resolved_canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::RwLock;

    fn make_ctx(workspace: &std::path::Path) -> ToolContext {
        ToolContext {
            user_id: "test-user".into(),
            role: temm1e_core::types::rbac::Role::Admin,
            channel: "cli".into(),
            workspace_path: workspace.to_path_buf(),
            session_id: "test-session".to_string(),
            chat_id: "test-chat".to_string(),
            read_tracker: Some(Arc::new(RwLock::new(std::collections::HashSet::new()))),
        }
    }

    async fn read_file(workspace: &std::path::Path, path: &str, args: serde_json::Value) -> String {
        let tool = FileReadTool::new();
        let ctx = make_ctx(workspace);
        let mut arguments = serde_json::Map::new();
        arguments.insert("path".into(), serde_json::Value::String(path.to_string()));
        if let serde_json::Value::Object(extra) = args {
            for (k, v) in extra {
                arguments.insert(k, v);
            }
        }
        let input = ToolInput {
            name: "file_read".to_string(),
            arguments: serde_json::Value::Object(arguments),
        };
        let out = tool.execute(input, &ctx).await.expect("read ok");
        assert!(!out.is_error, "read returned error: {}", out.content);
        out.content
    }

    #[tokio::test]
    async fn truncation_header_appears_on_line_limit() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("big.txt");
        let content: String = (1..=5000).map(|i| format!("line {i}\n")).collect();
        tokio::fs::write(&path, &content).await.unwrap();
        let out = read_file(dir.path(), "big.txt", serde_json::json!({"limit": 100})).await;
        assert!(
            out.starts_with("[TRUNCATED — showing lines 1-100 of 5000 total. To continue, call file_read with offset=101]"),
            "header missing: {}",
            &out[..200.min(out.len())]
        );
        assert!(
            out.contains("[Showing lines 1-100 of 5000 total]"),
            "footer missing"
        );
        assert!(out.contains("1\tline 1"), "first line missing");
        assert!(out.contains("100\tline 100"), "last selected line missing");
        assert!(!out.contains("101\t"), "should not include line 101");
    }

    #[tokio::test]
    async fn truncation_header_appears_on_byte_cap() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wide.txt");
        // 500 lines of 100 chars each ≈ 50KB; well over the 32KB cap.
        let line: String = "x".repeat(100);
        let content: String = (1..=500).map(|i| format!("{i:04} {line}\n")).collect();
        tokio::fs::write(&path, &content).await.unwrap();
        let out = read_file(dir.path(), "wide.txt", serde_json::json!({})).await;
        assert!(
            out.starts_with("[TRUNCATED —"),
            "header missing on byte cap"
        );
        assert!(
            out.contains("of 500 total"),
            "header should reference total=500"
        );
        assert!(
            out.contains("[output truncated at 32KB]"),
            "byte-cap footer missing"
        );
        assert!(
            out.contains("[Showing lines 1-"),
            "partial-read footer missing on byte cap"
        );
    }

    #[tokio::test]
    async fn no_truncation_no_header() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("small.txt");
        tokio::fs::write(&path, "hello\nworld\n").await.unwrap();
        let out = read_file(dir.path(), "small.txt", serde_json::json!({})).await;
        assert!(!out.starts_with("[TRUNCATED"), "header should not appear");
        assert!(!out.contains("[Showing lines"), "footer should not appear");
        assert!(
            !out.contains("[output truncated at 32KB]"),
            "byte-cap marker should not appear"
        );
        assert!(out.contains("1\thello"));
        assert!(out.contains("2\tworld"));
    }

    #[tokio::test]
    async fn byte_cap_offset_math_correct() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("offsetcheck.txt");
        // Each formatted line = ~100 bytes. 500 lines ≈ 50KB > 32KB cap.
        let line: String = "y".repeat(95);
        let content: String = (1..=500).map(|i| format!("{i:04} {line}\n")).collect();
        tokio::fs::write(&path, &content).await.unwrap();
        let out = read_file(dir.path(), "offsetcheck.txt", serde_json::json!({})).await;

        // Parse "showing lines 1-N" to get N.
        let header = out.lines().next().unwrap();
        let n_str = header
            .split("showing lines 1-")
            .nth(1)
            .and_then(|s| s.split(' ').next())
            .expect("header should have line range");
        let n: usize = n_str.parse().expect("N should be numeric");
        assert!(n > 0 && n < 500, "expected truncation in middle: N={n}");

        // Verify the offset hint = N+1.
        let expected_offset = format!("offset={}", n + 1);
        assert!(
            header.contains(&expected_offset),
            "offset hint should be N+1 ({}); got header: {}",
            expected_offset,
            header
        );

        // Verify the body actually contains line N but not line N+1.
        let line_n_marker = format!("\n{:04} ", n);
        let line_np1_marker = format!("\n{:04} ", n + 1);
        // First line in body has no leading newline; check both forms.
        assert!(
            out.contains(&line_n_marker) || out.contains(&format!("{:04} ", n)),
            "body should contain line {n}"
        );
        assert!(
            !out.contains(&line_np1_marker),
            "body should NOT contain line {} (was truncated)",
            n + 1
        );
    }
}
