//! Code patch tool — atomic multi-file edits with dry-run validation and rollback.

use async_trait::async_trait;
use serde::Deserialize;
use temm1e_core::types::error::Temm1eError;
use temm1e_core::{PathAccess, Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};

/// A single text replacement within a file.
#[derive(Debug, Clone, Deserialize)]
struct Edit {
    old_string: String,
    new_string: String,
}

/// All edits for a single file.
#[derive(Debug, Clone, Deserialize)]
struct FileChange {
    file_path: String,
    edits: Vec<Edit>,
}

#[derive(Default)]
pub struct CodePatchTool;

impl CodePatchTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for CodePatchTool {
    fn name(&self) -> &str {
        "code_patch"
    }

    fn description(&self) -> &str {
        "Apply atomic multi-file text edits. All edits are validated before any \
         changes are written. If any edit fails validation, nothing is modified. \
         Each edit replaces exactly one occurrence of old_string with new_string."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "changes": {
                    "type": "array",
                    "description": "Array of file changes to apply atomically",
                    "items": {
                        "type": "object",
                        "properties": {
                            "file_path": {
                                "type": "string",
                                "description": "Path to the file (relative to workspace or absolute)"
                            },
                            "edits": {
                                "type": "array",
                                "description": "Edits to apply to this file, in order",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "old_string": {
                                            "type": "string",
                                            "description": "Text to find (must appear exactly once)"
                                        },
                                        "new_string": {
                                            "type": "string",
                                            "description": "Replacement text"
                                        }
                                    },
                                    "required": ["old_string", "new_string"]
                                }
                            }
                        },
                        "required": ["file_path", "edits"]
                    }
                }
            },
            "required": ["changes"]
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
        let changes_value = input
            .arguments
            .get("changes")
            .ok_or_else(|| Temm1eError::Tool("Missing required parameter: changes".into()))?;

        let changes: Vec<FileChange> = serde_json::from_value(changes_value.clone())
            .map_err(|e| Temm1eError::Tool(format!("Invalid changes format: {}", e)))?;

        if changes.is_empty() {
            return Err(Temm1eError::Tool("changes array is empty".into()));
        }

        // ── Dry run: validate every edit across all files ──────────────
        let mut failures: Vec<String> = Vec::new();

        for change in &changes {
            let path = crate::file::resolve_path(&change.file_path, &ctx.workspace_path)?;

            let content = match tokio::fs::read_to_string(&path).await {
                Ok(c) => c,
                Err(e) => {
                    failures.push(format!("{}: cannot read file: {}", change.file_path, e));
                    continue;
                }
            };

            // Walk edits against a simulated content to validate sequentially
            let mut simulated = content.clone();
            for (i, edit) in change.edits.iter().enumerate() {
                if edit.old_string == edit.new_string {
                    failures.push(format!(
                        "{}: edit {}: old_string equals new_string",
                        change.file_path,
                        i + 1
                    ));
                    continue;
                }

                let count = simulated.matches(&edit.old_string).count();
                if count == 0 {
                    failures.push(format!(
                        "{}: edit {}: old_string not found in file",
                        change.file_path,
                        i + 1
                    ));
                } else if count > 1 {
                    failures.push(format!(
                        "{}: edit {}: old_string found {} times (expected exactly 1)",
                        change.file_path,
                        i + 1,
                        count
                    ));
                } else {
                    // Apply in simulation so subsequent edits see the updated content
                    simulated = simulated.replacen(&edit.old_string, &edit.new_string, 1);
                }
            }
        }

        if !failures.is_empty() {
            return Err(Temm1eError::Tool(format!(
                "Validation failed ({} error{}):\n{}",
                failures.len(),
                if failures.len() == 1 { "" } else { "s" },
                failures.join("\n")
            )));
        }

        // ── Backup phase: read fresh content for all files ─────────────
        // Collect (resolved_path, original_path_str, content) for each file.
        let mut backups: Vec<(std::path::PathBuf, String, String)> = Vec::new();

        for change in &changes {
            let path = crate::file::resolve_path(&change.file_path, &ctx.workspace_path)?;
            let content = tokio::fs::read_to_string(&path).await.map_err(|e| {
                Temm1eError::Tool(format!(
                    "{}: cannot read file for apply: {}",
                    change.file_path, e
                ))
            })?;
            backups.push((path, change.file_path.clone(), content));
        }

        // ── Apply phase ────────────────────────────────────────────────
        let mut files_modified: usize = 0;
        let mut total_edits: usize = 0;
        let mut bytes_changed: usize = 0;

        for (idx, change) in changes.iter().enumerate() {
            let (ref path, ref _path_str, ref original) = backups[idx];
            let mut content = original.clone();

            for edit in &change.edits {
                content = content.replacen(&edit.old_string, &edit.new_string, 1);
                total_edits += 1;
            }

            let old_len = original.len();
            let new_len = content.len();
            bytes_changed += new_len.abs_diff(old_len);

            // Atomic write: write to temp file then rename
            let dir = path.parent().ok_or_else(|| {
                Temm1eError::Tool(format!(
                    "{}: cannot determine parent directory",
                    change.file_path
                ))
            })?;

            let temp_path = dir.join(format!(".code_patch_tmp_{}_{}", std::process::id(), idx));

            if let Err(e) = tokio::fs::write(&temp_path, &content).await {
                // Rollback: restore all files written so far
                for (rollback_idx, (ref rb_path, ref rb_path_str, ref rb_content)) in
                    backups.iter().enumerate()
                {
                    if rollback_idx >= idx {
                        break;
                    }
                    if let Err(re) = tokio::fs::write(rb_path, rb_content).await {
                        tracing::error!(
                            path = %rb_path_str,
                            error = %re,
                            "Rollback write failed"
                        );
                    }
                }
                // Clean up the failed temp file
                let _ = tokio::fs::remove_file(&temp_path).await;
                return Err(Temm1eError::Tool(format!(
                    "Write failed for {}: {} (rolled back {} file{})",
                    change.file_path,
                    e,
                    idx,
                    if idx == 1 { "" } else { "s" }
                )));
            }

            if let Err(e) = tokio::fs::rename(&temp_path, path).await {
                // Rename failed — remove temp, restore this file from backup, rollback prior
                let _ = tokio::fs::remove_file(&temp_path).await;
                // Restore current file (temp write succeeded but rename didn't)
                let _ = tokio::fs::write(path, original).await;
                // Rollback earlier files
                for (rollback_idx, (ref rb_path, ref rb_path_str, ref rb_content)) in
                    backups.iter().enumerate()
                {
                    if rollback_idx >= idx {
                        break;
                    }
                    if let Err(re) = tokio::fs::write(rb_path, rb_content).await {
                        tracing::error!(
                            path = %rb_path_str,
                            error = %re,
                            "Rollback write failed"
                        );
                    }
                }
                return Err(Temm1eError::Tool(format!(
                    "Rename failed for {}: {} (rolled back {} file{})",
                    change.file_path,
                    e,
                    idx,
                    if idx == 1 { "" } else { "s" }
                )));
            }

            files_modified += 1;
        }

        Ok(ToolOutput {
            content: format!(
                "{} file{} modified, {} edit{} applied, {} bytes changed",
                files_modified,
                if files_modified == 1 { "" } else { "s" },
                total_edits,
                if total_edits == 1 { "" } else { "s" },
                bytes_changed,
            ),
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_ctx(workspace: &std::path::Path) -> ToolContext {
        ToolContext {
            workspace_path: workspace.to_path_buf(),
            session_id: "test-session".into(),
            chat_id: "test-chat".into(),
            read_tracker: None,
        }
    }

    fn make_input(changes: serde_json::Value) -> ToolInput {
        ToolInput {
            name: "code_patch".into(),
            arguments: serde_json::json!({ "changes": changes }),
        }
    }

    #[test]
    fn test_name() {
        let tool = CodePatchTool::new();
        assert_eq!(tool.name(), "code_patch");
    }

    #[test]
    fn test_schema() {
        let tool = CodePatchTool::new();
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["changes"].is_object());
        assert_eq!(schema["properties"]["changes"]["type"], "array");
        let item_props = &schema["properties"]["changes"]["items"]["properties"];
        assert!(item_props["file_path"].is_object());
        assert!(item_props["edits"].is_object());
        let edit_props = &item_props["edits"]["items"]["properties"];
        assert!(edit_props["old_string"].is_object());
        assert!(edit_props["new_string"].is_object());
    }

    #[tokio::test]
    async fn test_single_file_single_edit() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("hello.txt");
        tokio::fs::write(&file_path, "Hello, world!").await.unwrap();

        let tool = CodePatchTool::new();
        let ctx = make_ctx(dir.path());
        let input = make_input(serde_json::json!([
            {
                "file_path": "hello.txt",
                "edits": [
                    { "old_string": "world", "new_string": "Rust" }
                ]
            }
        ]));

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("1 file modified"));
        assert!(output.content.contains("1 edit applied"));

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "Hello, Rust!");
    }

    #[tokio::test]
    async fn test_multi_file_edit() {
        let dir = tempdir().unwrap();
        let file_a = dir.path().join("a.txt");
        let file_b = dir.path().join("b.txt");
        tokio::fs::write(&file_a, "alpha beta").await.unwrap();
        tokio::fs::write(&file_b, "gamma delta").await.unwrap();

        let tool = CodePatchTool::new();
        let ctx = make_ctx(dir.path());
        let input = make_input(serde_json::json!([
            {
                "file_path": "a.txt",
                "edits": [
                    { "old_string": "alpha", "new_string": "ALPHA" }
                ]
            },
            {
                "file_path": "b.txt",
                "edits": [
                    { "old_string": "delta", "new_string": "DELTA" }
                ]
            }
        ]));

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("2 files modified"));
        assert!(output.content.contains("2 edits applied"));

        let content_a = tokio::fs::read_to_string(&file_a).await.unwrap();
        let content_b = tokio::fs::read_to_string(&file_b).await.unwrap();
        assert_eq!(content_a, "ALPHA beta");
        assert_eq!(content_b, "gamma DELTA");
    }

    #[tokio::test]
    async fn test_validation_failure_no_changes() {
        let dir = tempdir().unwrap();
        let file_a = dir.path().join("a.txt");
        let file_b = dir.path().join("b.txt");
        tokio::fs::write(&file_a, "alpha beta").await.unwrap();
        tokio::fs::write(&file_b, "gamma delta").await.unwrap();

        let tool = CodePatchTool::new();
        let ctx = make_ctx(dir.path());
        // First edit is valid, second references text not in file_b
        let input = make_input(serde_json::json!([
            {
                "file_path": "a.txt",
                "edits": [
                    { "old_string": "alpha", "new_string": "ALPHA" }
                ]
            },
            {
                "file_path": "b.txt",
                "edits": [
                    { "old_string": "NONEXISTENT", "new_string": "replaced" }
                ]
            }
        ]));

        let result = tool.execute(input, &ctx).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("not found"));

        // Verify NEITHER file was changed (atomic: all-or-nothing)
        let content_a = tokio::fs::read_to_string(&file_a).await.unwrap();
        let content_b = tokio::fs::read_to_string(&file_b).await.unwrap();
        assert_eq!(content_a, "alpha beta");
        assert_eq!(content_b, "gamma delta");
    }

    #[tokio::test]
    async fn test_multi_edit_same_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("code.rs");
        tokio::fs::write(&file_path, "fn foo() { bar(); baz(); }")
            .await
            .unwrap();

        let tool = CodePatchTool::new();
        let ctx = make_ctx(dir.path());
        let input = make_input(serde_json::json!([
            {
                "file_path": "code.rs",
                "edits": [
                    { "old_string": "bar()", "new_string": "bar_v2()" },
                    { "old_string": "baz()", "new_string": "baz_v2()" }
                ]
            }
        ]));

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("1 file modified"));
        assert!(output.content.contains("2 edits applied"));

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "fn foo() { bar_v2(); baz_v2(); }");
    }

    #[tokio::test]
    async fn test_old_equals_new_rejected() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("same.txt");
        tokio::fs::write(&file_path, "no change here")
            .await
            .unwrap();

        let tool = CodePatchTool::new();
        let ctx = make_ctx(dir.path());
        let input = make_input(serde_json::json!([
            {
                "file_path": "same.txt",
                "edits": [
                    { "old_string": "no change", "new_string": "no change" }
                ]
            }
        ]));

        let result = tool.execute(input, &ctx).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("old_string equals new_string"));

        // File must be unmodified
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "no change here");
    }
}
