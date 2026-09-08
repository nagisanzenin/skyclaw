//! Code snapshot tool — checkpoint/restore workspace state via git internals.
//!
//! Uses `git write-tree` / `git read-tree` to create lightweight snapshots
//! of the working directory without polluting the commit history.

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use temm1e_core::types::error::Temm1eError;
use temm1e_core::{PathAccess, Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};

/// Timeout for each git subprocess (30 seconds).
const GIT_TIMEOUT_SECS: u64 = 30;

/// Valid snapshot actions.
const VALID_ACTIONS: &[&str] = &["create", "restore", "list", "diff"];

/// A single snapshot entry persisted in the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotEntry {
    /// Short hash prefix (first 8 chars of tree hash).
    id: String,
    /// Full git tree hash.
    tree_hash: String,
    /// Human-readable name.
    name: String,
    /// ISO 8601 timestamp.
    timestamp: String,
}

/// Persistent store of all snapshots for a workspace.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SnapshotStore {
    snapshots: Vec<SnapshotEntry>,
}

#[derive(Default)]
pub struct CodeSnapshotTool;

impl CodeSnapshotTool {
    pub fn new() -> Self {
        Self
    }

    /// Run a git command in the workspace with a 30-second timeout.
    /// Returns stdout on success, or a `Temm1eError::Tool` on failure.
    async fn run_git(workspace: &std::path::Path, args: &[&str]) -> Result<String, Temm1eError> {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(GIT_TIMEOUT_SECS),
            tokio::process::Command::new("git")
                .args(args)
                .current_dir(workspace)
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                if output.status.success() {
                    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(Temm1eError::Tool(format!(
                        "git {} failed: {}",
                        args.first().unwrap_or(&""),
                        stderr.trim()
                    )))
                }
            }
            Ok(Err(e)) => Err(Temm1eError::Tool(format!("Failed to execute git: {}", e))),
            Err(_) => Err(Temm1eError::Tool(format!(
                "git {} timed out after {} seconds",
                args.first().unwrap_or(&""),
                GIT_TIMEOUT_SECS
            ))),
        }
    }

    /// Path to the snapshot store JSON file.
    fn store_path(workspace: &std::path::Path) -> std::path::PathBuf {
        workspace.join(".temm1e").join("snapshots.json")
    }

    /// Load the snapshot store from disk. Returns a default (empty) store if
    /// the file does not exist.
    fn load_store(workspace: &std::path::Path) -> Result<SnapshotStore, Temm1eError> {
        let path = Self::store_path(workspace);
        if !path.exists() {
            return Ok(SnapshotStore::default());
        }
        let data = std::fs::read_to_string(&path)
            .map_err(|e| Temm1eError::Tool(format!("Failed to read snapshot store: {}", e)))?;
        serde_json::from_str(&data)
            .map_err(|e| Temm1eError::Tool(format!("Failed to parse snapshot store: {}", e)))
    }

    /// Save the snapshot store to disk, creating the directory if needed.
    fn save_store(workspace: &std::path::Path, store: &SnapshotStore) -> Result<(), Temm1eError> {
        let path = Self::store_path(workspace);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Temm1eError::Tool(format!("Failed to create .temm1e directory: {}", e))
            })?;
        }
        let json = serde_json::to_string_pretty(store)
            .map_err(|e| Temm1eError::Tool(format!("Failed to serialize snapshot store: {}", e)))?;
        std::fs::write(&path, json)
            .map_err(|e| Temm1eError::Tool(format!("Failed to write snapshot store: {}", e)))?;
        Ok(())
    }

    /// Find a snapshot entry by ID prefix match.
    fn find_snapshot<'a>(store: &'a SnapshotStore, snapshot_id: &str) -> Option<&'a SnapshotEntry> {
        store
            .snapshots
            .iter()
            .find(|s| s.id.starts_with(snapshot_id) || snapshot_id.starts_with(&s.id))
    }

    /// Format the list of available snapshot IDs for error messages.
    fn format_available_ids(store: &SnapshotStore) -> String {
        if store.snapshots.is_empty() {
            "No snapshots available.".to_string()
        } else {
            let ids: Vec<String> = store
                .snapshots
                .iter()
                .map(|s| format!("{} ({})", s.id, s.name))
                .collect();
            format!("Available snapshots: {}", ids.join(", "))
        }
    }

    /// Create a new snapshot of the current workspace state.
    async fn action_create(
        workspace: &std::path::Path,
        name: Option<&str>,
    ) -> Result<ToolOutput, Temm1eError> {
        // Stage all current changes
        Self::run_git(workspace, &["add", "-A"]).await?;

        // Write the index as a tree object
        let tree_hash = Self::run_git(workspace, &["write-tree"]).await?;

        let id = if tree_hash.len() >= 8 {
            tree_hash[..8].to_string()
        } else {
            tree_hash.clone()
        };

        let timestamp = Utc::now().to_rfc3339();
        let snapshot_name = name
            .filter(|n| !n.is_empty())
            .map(|n| n.to_string())
            .unwrap_or_else(|| format!("snapshot-{}", timestamp));

        let entry = SnapshotEntry {
            id: id.clone(),
            tree_hash,
            name: snapshot_name.clone(),
            timestamp,
        };

        let mut store = Self::load_store(workspace)?;
        store.snapshots.push(entry);
        Self::save_store(workspace, &store)?;

        tracing::info!(
            snapshot_id = %id,
            snapshot_name = %snapshot_name,
            "Created code snapshot"
        );

        Ok(ToolOutput {
            content: format!("Snapshot '{}' created (id: {})", snapshot_name, id),
            is_error: false,
        })
    }

    /// Restore the workspace to a previously created snapshot.
    async fn action_restore(
        workspace: &std::path::Path,
        snapshot_id: &str,
    ) -> Result<ToolOutput, Temm1eError> {
        let store = Self::load_store(workspace)?;
        let entry = Self::find_snapshot(&store, snapshot_id).ok_or_else(|| {
            Temm1eError::Tool(format!(
                "Snapshot '{}' not found. {}",
                snapshot_id,
                Self::format_available_ids(&store)
            ))
        })?;

        let tree_hash = entry.tree_hash.clone();
        let name = entry.name.clone();
        let id = entry.id.clone();

        // Restore the tree into the index and check out all files
        Self::run_git(workspace, &["read-tree", &tree_hash]).await?;
        Self::run_git(workspace, &["checkout-index", "-a", "-f"]).await?;

        tracing::info!(
            snapshot_id = %id,
            snapshot_name = %name,
            "Restored code snapshot"
        );

        Ok(ToolOutput {
            content: format!("Restored to snapshot '{}' ({})", name, id),
            is_error: false,
        })
    }

    /// List all snapshots in the store.
    fn action_list(workspace: &std::path::Path) -> Result<ToolOutput, Temm1eError> {
        let store = Self::load_store(workspace)?;

        if store.snapshots.is_empty() {
            return Ok(ToolOutput {
                content: "No snapshots found.".to_string(),
                is_error: false,
            });
        }

        let mut lines = vec!["ID       | Name                           | Timestamp".to_string()];
        lines.push(
            "---------+--------------------------------+--------------------------".to_string(),
        );
        for entry in &store.snapshots {
            lines.push(format!(
                "{:<8} | {:<30} | {}",
                entry.id, entry.name, entry.timestamp
            ));
        }

        Ok(ToolOutput {
            content: lines.join("\n"),
            is_error: false,
        })
    }

    /// Show a diff between a snapshot and the current HEAD.
    async fn action_diff(
        workspace: &std::path::Path,
        snapshot_id: &str,
    ) -> Result<ToolOutput, Temm1eError> {
        let store = Self::load_store(workspace)?;
        let entry = Self::find_snapshot(&store, snapshot_id).ok_or_else(|| {
            Temm1eError::Tool(format!(
                "Snapshot '{}' not found. {}",
                snapshot_id,
                Self::format_available_ids(&store)
            ))
        })?;

        let tree_hash = entry.tree_hash.clone();

        let output = Self::run_git(
            workspace,
            &["diff-tree", "-r", "--stat", &tree_hash, "HEAD"],
        )
        .await?;

        let content = if output.is_empty() {
            format!("No differences between snapshot '{}' and HEAD.", entry.name)
        } else {
            output
        };

        Ok(ToolOutput {
            content,
            is_error: false,
        })
    }
}

#[async_trait]
impl Tool for CodeSnapshotTool {
    fn name(&self) -> &str {
        "code_snapshot"
    }

    fn description(&self) -> &str {
        "Create, restore, list, or diff workspace snapshots using git internals. \
         Snapshots capture the full working tree state without creating commits. \
         Actions: create (save current state), restore (revert to a snapshot), \
         list (show all snapshots), diff (compare snapshot to HEAD)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Snapshot action: create, restore, list, diff",
                    "enum": VALID_ACTIONS
                },
                "name": {
                    "type": "string",
                    "description": "Human-readable snapshot name (for create action)"
                },
                "snapshot_id": {
                    "type": "string",
                    "description": "Snapshot ID to restore or diff (for restore/diff actions)"
                }
            },
            "required": ["action"]
        })
    }

    fn declarations(&self) -> ToolDeclarations {
        ToolDeclarations {
            file_access: vec![PathAccess::ReadWrite(".".into())],
            network_access: Vec::new(),
            shell_access: true,
        }
    }

    async fn execute(
        &self,
        input: ToolInput,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let action = input
            .arguments
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Temm1eError::Tool("Missing required parameter: action".into()))?;

        if !VALID_ACTIONS.contains(&action) {
            return Err(Temm1eError::Tool(format!(
                "Unknown action '{}'. Valid actions: {}",
                action,
                VALID_ACTIONS.join(", ")
            )));
        }

        let workspace = &ctx.workspace_path;

        match action {
            "create" => {
                let name = input.arguments.get("name").and_then(|v| v.as_str());
                Self::action_create(workspace, name).await
            }
            "restore" => {
                let snapshot_id = input
                    .arguments
                    .get("snapshot_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        Temm1eError::Tool(
                            "Missing required parameter: snapshot_id (for restore)".into(),
                        )
                    })?;
                Self::action_restore(workspace, snapshot_id).await
            }
            "list" => Self::action_list(workspace),
            "diff" => {
                let snapshot_id = input
                    .arguments
                    .get("snapshot_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        Temm1eError::Tool(
                            "Missing required parameter: snapshot_id (for diff)".into(),
                        )
                    })?;
                Self::action_diff(workspace, snapshot_id).await
            }
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Initialize a bare-minimum git repo in the given directory with an
    /// initial commit so that HEAD exists for diff operations.
    async fn init_git_repo(dir: &std::path::Path) {
        tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .await
            .expect("git init failed");

        tokio::process::Command::new("git")
            .args(["config", "user.email", "test@temm1e.local"])
            .current_dir(dir)
            .output()
            .await
            .expect("git config email failed");

        tokio::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .await
            .expect("git config name failed");

        // Create an initial file and commit so HEAD is valid
        std::fs::write(dir.join(".gitkeep"), "").expect("write .gitkeep failed");

        tokio::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir)
            .output()
            .await
            .expect("git add failed");

        tokio::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir)
            .output()
            .await
            .expect("git commit failed");
    }

    fn make_ctx(workspace: PathBuf) -> ToolContext {
        ToolContext {
            workspace_path: workspace,
            session_id: "test-session".to_string(),
            chat_id: "test-chat".to_string(),
            read_tracker: None,
        }
    }

    #[test]
    fn test_name() {
        let tool = CodeSnapshotTool::new();
        assert_eq!(tool.name(), "code_snapshot");
    }

    #[test]
    fn test_schema() {
        let tool = CodeSnapshotTool::new();
        let schema = tool.parameters_schema();
        assert!(schema.is_object());
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["name"].is_object());
        assert!(schema["properties"]["snapshot_id"].is_object());

        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("action")));
    }

    #[test]
    fn test_declarations() {
        let tool = CodeSnapshotTool::new();
        let decl = tool.declarations();
        assert!(decl.shell_access);
        assert!(decl.network_access.is_empty());
        assert_eq!(decl.file_access.len(), 1);
        assert!(matches!(&decl.file_access[0], PathAccess::ReadWrite(p) if p == "."));
    }

    #[tokio::test]
    async fn test_create_and_list() {
        let tmp = tempfile::tempdir().expect("tempdir failed");
        let dir = tmp.path().to_path_buf();
        init_git_repo(&dir).await;

        let tool = CodeSnapshotTool::new();
        let ctx = make_ctx(dir.clone());

        // Create a file so the snapshot has content
        std::fs::write(dir.join("hello.txt"), "world").expect("write failed");

        // Create snapshot
        let input = ToolInput {
            name: "code_snapshot".to_string(),
            arguments: serde_json::json!({
                "action": "create",
                "name": "my-checkpoint"
            }),
        };
        let output = tool.execute(input, &ctx).await.expect("create failed");
        assert!(!output.is_error);
        assert!(output.content.contains("my-checkpoint"));
        assert!(output.content.contains("created"));

        // List snapshots
        let input = ToolInput {
            name: "code_snapshot".to_string(),
            arguments: serde_json::json!({ "action": "list" }),
        };
        let output = tool.execute(input, &ctx).await.expect("list failed");
        assert!(!output.is_error);
        assert!(output.content.contains("my-checkpoint"));
        assert!(output.content.contains("ID"));
    }

    #[tokio::test]
    async fn test_create_and_restore() {
        let tmp = tempfile::tempdir().expect("tempdir failed");
        let dir = tmp.path().to_path_buf();
        init_git_repo(&dir).await;

        let tool = CodeSnapshotTool::new();
        let ctx = make_ctx(dir.clone());

        // Create original file
        let file_path = dir.join("data.txt");
        std::fs::write(&file_path, "original content").expect("write failed");

        // Create snapshot
        let input = ToolInput {
            name: "code_snapshot".to_string(),
            arguments: serde_json::json!({
                "action": "create",
                "name": "before-change"
            }),
        };
        let output = tool.execute(input, &ctx).await.expect("create failed");
        assert!(!output.is_error);

        // Extract the snapshot ID from the output
        let id = output
            .content
            .split("id: ")
            .nth(1)
            .and_then(|s| s.strip_suffix(')'))
            .expect("could not extract snapshot id")
            .to_string();

        // Modify the file
        std::fs::write(&file_path, "modified content").expect("write failed");
        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "modified content"
        );

        // Restore the snapshot
        let input = ToolInput {
            name: "code_snapshot".to_string(),
            arguments: serde_json::json!({
                "action": "restore",
                "snapshot_id": id
            }),
        };
        let output = tool.execute(input, &ctx).await.expect("restore failed");
        assert!(!output.is_error);
        assert!(output.content.contains("Restored"));
        assert!(output.content.contains("before-change"));

        // Verify file content is back to original
        let content = std::fs::read_to_string(&file_path).expect("read failed");
        assert_eq!(content, "original content");
    }

    #[tokio::test]
    async fn test_restore_not_found() {
        let tmp = tempfile::tempdir().expect("tempdir failed");
        let dir = tmp.path().to_path_buf();
        init_git_repo(&dir).await;

        let tool = CodeSnapshotTool::new();
        let ctx = make_ctx(dir);

        let input = ToolInput {
            name: "code_snapshot".to_string(),
            arguments: serde_json::json!({
                "action": "restore",
                "snapshot_id": "nonexist"
            }),
        };
        let result = tool.execute(input, &ctx).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("not found"));
    }

    #[tokio::test]
    async fn test_list_empty() {
        let tmp = tempfile::tempdir().expect("tempdir failed");
        let dir = tmp.path().to_path_buf();
        init_git_repo(&dir).await;

        let tool = CodeSnapshotTool::new();
        let ctx = make_ctx(dir);

        let input = ToolInput {
            name: "code_snapshot".to_string(),
            arguments: serde_json::json!({ "action": "list" }),
        };
        let output = tool.execute(input, &ctx).await.expect("list failed");
        assert!(!output.is_error);
        assert!(output.content.contains("No snapshots found"));
    }
}
