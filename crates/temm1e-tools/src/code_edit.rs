//! Code edit tool — exact string replacement editing within files.

use async_trait::async_trait;
use temm1e_core::types::error::Temm1eError;
use temm1e_core::{PathAccess, Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};

/// Exact string replacement tool for editing files.
///
/// Finds an exact occurrence of `old_string` in a file and replaces it with
/// `new_string`. Requires the file to have been read first (via `file_read`)
/// when a read tracker is active. Writes are atomic via a temporary file and
/// rename.
#[derive(Default)]
pub struct CodeEditTool;

impl CodeEditTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for CodeEditTool {
    fn name(&self) -> &str {
        "code_edit"
    }

    fn description(&self) -> &str {
        "Perform an exact string replacement in a file. Finds `old_string` in the file \
         and replaces it with `new_string`. The old_string must be unique in the file \
         unless replace_all is true. The file must have been read first via file_read."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "Exact text to find in the file (must be unique unless replace_all is true)"
                },
                "new_string": {
                    "type": "string",
                    "description": "Replacement text"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences of old_string (default: false)"
                }
            },
            "required": ["file_path", "old_string", "new_string"]
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
        // ── Extract parameters ───────────────────────────────────────
        let path_str = input
            .arguments
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Temm1eError::Tool("Missing required parameter: file_path".into()))?;

        let old_string = input
            .arguments
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Temm1eError::Tool("Missing required parameter: old_string".into()))?;

        let new_string = input
            .arguments
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Temm1eError::Tool("Missing required parameter: new_string".into()))?;

        let replace_all = input
            .arguments
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // ── Resolve path ─────────────────────────────────────────────
        let path = crate::file::resolve_path(path_str, &ctx.workspace_path)?;

        // ── Read-tracker gate ────────────────────────────────────────
        if let Some(ref tracker) = ctx.read_tracker {
            let set = tracker.read().await;
            if !set.contains(&path) {
                return Err(Temm1eError::Tool(
                    "File must be read before editing. Use file_read to read the file first."
                        .into(),
                ));
            }
        }

        // ── Read current content ─────────────────────────────────────
        let content = tokio::fs::read_to_string(&path).await.map_err(|e| {
            Temm1eError::Tool(format!("Failed to read file '{}': {}", path.display(), e))
        })?;

        // ── Validate old != new ──────────────────────────────────────
        if old_string == new_string {
            return Err(Temm1eError::Tool(
                "old_string and new_string are identical — no edit needed.".into(),
            ));
        }

        // ── Find matches ─────────────────────────────────────────────
        let match_count = content.matches(old_string).count();

        if match_count == 0 {
            let preview: String = content.chars().take(100).collect();
            return Err(Temm1eError::Tool(format!(
                "old_string not found in '{}'. File starts with: {}",
                path.display(),
                preview,
            )));
        }

        if match_count > 1 && !replace_all {
            return Err(Temm1eError::Tool(format!(
                "old_string found {} times in '{}'. Use replace_all=true to replace all occurrences, \
                 or provide a more specific old_string that matches exactly once.",
                match_count,
                path.display(),
            )));
        }

        // ── Perform replacement ──────────────────────────────────────
        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        let replacements = if replace_all { match_count } else { 1 };

        // Estimate lines changed: count newlines in old vs new regions
        let old_lines = old_string.lines().count().max(1);
        let new_lines = new_string.lines().count().max(1);
        let lines_changed = old_lines.max(new_lines);

        let bytes_delta = new_content.len() as i64 - content.len() as i64;

        // ── Atomic write ─────────────────────────────────────────────
        let tmp_path = path.with_extension("temm1e.tmp");

        if let Err(e) = tokio::fs::write(&tmp_path, &new_content).await {
            // Clean up tmp on write failure
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(Temm1eError::Tool(format!(
                "Failed to write temporary file '{}': {}",
                tmp_path.display(),
                e,
            )));
        }

        if let Err(e) = tokio::fs::rename(&tmp_path, &path).await {
            // Clean up tmp on rename failure
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(Temm1eError::Tool(format!(
                "Failed to atomically replace '{}': {}",
                path.display(),
                e,
            )));
        }

        // ── Success ──────────────────────────────────────────────────
        Ok(ToolOutput {
            content: format!(
                "Edited '{}': {} replacement(s), ~{} line(s) changed, {:+} bytes",
                path_str, replacements, lines_changed, bytes_delta,
            ),
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn test_ctx(workspace: PathBuf) -> ToolContext {
        ToolContext {
            workspace_path: workspace,
            session_id: "test-session".into(),
            chat_id: "test-chat".into(),
            read_tracker: None,
        }
    }

    fn test_ctx_with_tracker(
        workspace: PathBuf,
    ) -> (
        ToolContext,
        std::sync::Arc<tokio::sync::RwLock<std::collections::HashSet<PathBuf>>>,
    ) {
        let tracker =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new()));
        let ctx = ToolContext {
            workspace_path: workspace,
            session_id: "test-session".into(),
            chat_id: "test-chat".into(),
            read_tracker: Some(tracker.clone()),
        };
        (ctx, tracker)
    }

    #[test]
    fn test_name() {
        let tool = CodeEditTool::new();
        assert_eq!(tool.name(), "code_edit");
    }

    #[test]
    fn test_parameters_schema_valid_json() {
        let tool = CodeEditTool::new();
        let schema = tool.parameters_schema();

        assert!(schema.is_object());
        let obj = schema.as_object().unwrap();
        assert_eq!(obj.get("type").and_then(|v| v.as_str()), Some("object"));

        let props = obj.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("file_path"));
        assert!(props.contains_key("old_string"));
        assert!(props.contains_key("new_string"));
        assert!(props.contains_key("replace_all"));

        let required = obj.get("required").unwrap().as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(required_strs.contains(&"file_path"));
        assert!(required_strs.contains(&"old_string"));
        assert!(required_strs.contains(&"new_string"));
    }

    #[test]
    fn test_declarations() {
        let tool = CodeEditTool::new();
        let decl = tool.declarations();

        assert!(!decl.file_access.is_empty());
        assert!(matches!(decl.file_access[0], PathAccess::ReadWrite(_)));
        assert!(!decl.shell_access);
        assert!(decl.network_access.is_empty());
    }

    #[tokio::test]
    async fn test_old_equals_new_rejected() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        tokio::fs::write(&file_path, "hello world").await.unwrap();

        let tool = CodeEditTool::new();
        let ctx = test_ctx(dir.path().to_path_buf());

        let input = ToolInput {
            name: "code_edit".into(),
            arguments: serde_json::json!({
                "file_path": file_path.to_str().unwrap(),
                "old_string": "hello",
                "new_string": "hello",
            }),
        };

        let result = tool.execute(input, &ctx).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("identical"),
            "Error should mention identical strings: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_successful_edit() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        tokio::fs::write(&file_path, "hello world").await.unwrap();

        let tool = CodeEditTool::new();
        let ctx = test_ctx(dir.path().to_path_buf());

        let input = ToolInput {
            name: "code_edit".into(),
            arguments: serde_json::json!({
                "file_path": file_path.to_str().unwrap(),
                "old_string": "hello",
                "new_string": "goodbye",
            }),
        };

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("1 replacement(s)"));

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "goodbye world");
    }

    #[tokio::test]
    async fn test_multiple_matches_without_replace_all() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        tokio::fs::write(&file_path, "foo bar foo").await.unwrap();

        let tool = CodeEditTool::new();
        let ctx = test_ctx(dir.path().to_path_buf());

        let input = ToolInput {
            name: "code_edit".into(),
            arguments: serde_json::json!({
                "file_path": file_path.to_str().unwrap(),
                "old_string": "foo",
                "new_string": "baz",
            }),
        };

        let result = tool.execute(input, &ctx).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("2 times"),
            "Error should mention match count: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_replace_all() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        tokio::fs::write(&file_path, "foo bar foo baz foo")
            .await
            .unwrap();

        let tool = CodeEditTool::new();
        let ctx = test_ctx(dir.path().to_path_buf());

        let input = ToolInput {
            name: "code_edit".into(),
            arguments: serde_json::json!({
                "file_path": file_path.to_str().unwrap(),
                "old_string": "foo",
                "new_string": "qux",
                "replace_all": true,
            }),
        };

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("3 replacement(s)"));

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "qux bar qux baz qux");
    }

    #[tokio::test]
    async fn test_not_found() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        tokio::fs::write(&file_path, "hello world").await.unwrap();

        let tool = CodeEditTool::new();
        let ctx = test_ctx(dir.path().to_path_buf());

        let input = ToolInput {
            name: "code_edit".into(),
            arguments: serde_json::json!({
                "file_path": file_path.to_str().unwrap(),
                "old_string": "nonexistent",
                "new_string": "something",
            }),
        };

        let result = tool.execute(input, &ctx).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("not found"),
            "Error should indicate not found: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_atomic_write() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        let tmp_path = dir.path().join("test.temm1e.tmp");
        tokio::fs::write(&file_path, "aaa bbb ccc").await.unwrap();

        let tool = CodeEditTool::new();
        let ctx = test_ctx(dir.path().to_path_buf());

        let input = ToolInput {
            name: "code_edit".into(),
            arguments: serde_json::json!({
                "file_path": file_path.to_str().unwrap(),
                "old_string": "bbb",
                "new_string": "ddd",
            }),
        };

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);

        // Verify the temp file does not persist
        assert!(
            !tmp_path.exists(),
            "Temporary file should not persist after successful edit"
        );

        // Verify the actual file was updated
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "aaa ddd ccc");
    }

    #[tokio::test]
    async fn test_read_tracker_blocks_unread_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        tokio::fs::write(&file_path, "hello world").await.unwrap();

        let tool = CodeEditTool::new();
        let (ctx, _tracker) = test_ctx_with_tracker(dir.path().to_path_buf());

        let input = ToolInput {
            name: "code_edit".into(),
            arguments: serde_json::json!({
                "file_path": file_path.to_str().unwrap(),
                "old_string": "hello",
                "new_string": "goodbye",
            }),
        };

        let result = tool.execute(input, &ctx).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("must be read before editing"),
            "Error should enforce read-before-write: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_read_tracker_allows_read_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        tokio::fs::write(&file_path, "hello world").await.unwrap();

        let tool = CodeEditTool::new();
        let (ctx, tracker) = test_ctx_with_tracker(dir.path().to_path_buf());

        // Simulate that file_read has been called — insert canonical path into
        // tracker (resolve_path returns canonical paths after workspace containment fix)
        {
            let mut set = tracker.write().await;
            set.insert(file_path.canonicalize().unwrap());
        }

        let input = ToolInput {
            name: "code_edit".into(),
            arguments: serde_json::json!({
                "file_path": file_path.to_str().unwrap(),
                "old_string": "hello",
                "new_string": "goodbye",
            }),
        };

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "goodbye world");
    }
}
