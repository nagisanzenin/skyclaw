//! Code grep tool — content search with regex across the workspace.

use async_trait::async_trait;
use regex::Regex;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use temm1e_core::types::error::Temm1eError;
use temm1e_core::{PathAccess, Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};

/// Default maximum number of results returned.
const DEFAULT_HEAD_LIMIT: usize = 250;

/// Directories to always skip during recursive walk.
const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "__pycache__"];

#[derive(Default)]
pub struct CodeGrepTool;

impl CodeGrepTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for CodeGrepTool {
    fn name(&self) -> &str {
        "code_grep"
    }

    fn description(&self) -> &str {
        "Search file contents with regex. Recursively walks the workspace directory, \
         skipping .git/, node_modules/, target/, and __pycache__/. Supports three output \
         modes: files_with_matches (default, file paths only), content (matching lines with \
         line numbers and optional context), and count (match counts per file). \
         Supports glob filtering and case-insensitive search."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for in file contents"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (default: workspace root)"
                },
                "glob": {
                    "type": "string",
                    "description": "File filter pattern (e.g., \"*.rs\", \"*.py\"). Simple extension matching."
                },
                "output_mode": {
                    "type": "string",
                    "description": "Output mode: \"files_with_matches\" (default), \"content\", or \"count\"",
                    "enum": ["files_with_matches", "content", "count"]
                },
                "head_limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 250)"
                },
                "context": {
                    "type": "integer",
                    "description": "Lines of context around matches (default: 0, only for content mode)"
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Case-insensitive search (default: false)"
                }
            },
            "required": ["pattern"]
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
        let pattern_str = input
            .arguments
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Temm1eError::Tool("Missing required parameter: pattern".into()))?;

        let case_insensitive = input
            .arguments
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let full_pattern = if case_insensitive {
            format!("(?i){}", pattern_str)
        } else {
            pattern_str.to_string()
        };

        let regex = Regex::new(&full_pattern).map_err(|e| {
            Temm1eError::Tool(format!("Invalid regex pattern '{}': {}", pattern_str, e))
        })?;

        let search_path = input
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| resolve_search_path(p, &ctx.workspace_path))
            .unwrap_or_else(|| ctx.workspace_path.clone());

        let glob_filter = input
            .arguments
            .get("glob")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let output_mode = input
            .arguments
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("files_with_matches");

        let head_limit = input
            .arguments
            .get("head_limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_HEAD_LIMIT);

        let context_lines = input
            .arguments
            .get("context")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(0);

        // Collect all files to search
        let files = collect_files(&search_path, &glob_filter).await;

        match output_mode {
            "files_with_matches" => search_files_with_matches(&regex, &files, head_limit).await,
            "content" => search_content(&regex, &files, head_limit, context_lines).await,
            "count" => search_count(&regex, &files, head_limit).await,
            other => Err(Temm1eError::Tool(format!(
                "Unknown output_mode '{}'. Expected: files_with_matches, content, or count",
                other
            ))),
        }
    }
}

/// Resolve a search path relative to the workspace.
fn resolve_search_path(path_str: &str, workspace: &Path) -> PathBuf {
    let path = Path::new(path_str);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    }
}

/// Check if a filename matches a simple glob pattern (e.g., "*.rs").
fn matches_glob(filename: &str, glob: &str) -> bool {
    if let Some(ext) = glob.strip_prefix("*.") {
        filename.ends_with(&format!(".{}", ext))
    } else {
        filename == glob
    }
}

/// Recursively collect all searchable files, skipping ignored directories.
async fn collect_files(root: &Path, glob_filter: &Option<String>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut dirs_to_visit: VecDeque<PathBuf> = VecDeque::new();
    dirs_to_visit.push_back(root.to_path_buf());

    while let Some(dir) = dirs_to_visit.pop_front() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let file_type = match entry.file_type().await {
                Ok(ft) => ft,
                Err(_) => continue,
            };

            let name = entry.file_name().to_string_lossy().to_string();

            if file_type.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    dirs_to_visit.push_back(entry.path());
                }
            } else if file_type.is_file() {
                if let Some(ref glob) = glob_filter {
                    if !matches_glob(&name, glob) {
                        continue;
                    }
                }
                files.push(entry.path());
            }
        }
    }

    files.sort();
    files
}

/// "files_with_matches" mode: return paths of files containing at least one match.
async fn search_files_with_matches(
    regex: &Regex,
    files: &[PathBuf],
    head_limit: usize,
) -> Result<ToolOutput, Temm1eError> {
    let mut results = Vec::new();
    let mut total_found: usize = 0;

    for file in files {
        let content = match tokio::fs::read_to_string(file).await {
            Ok(c) => c,
            Err(_) => continue, // skip binary / unreadable files
        };

        if regex.is_match(&content) {
            total_found += 1;
            if results.len() < head_limit {
                results.push(file.display().to_string());
            }
        }
    }

    let mut output = results.join("\n");
    if total_found > head_limit {
        output.push_str(&format!(
            "\n[{} matches, showing first {}]",
            total_found, head_limit
        ));
    }

    if output.is_empty() {
        output = "No matches found.".to_string();
    }

    Ok(ToolOutput {
        content: output,
        is_error: false,
    })
}

/// "content" mode: show matching lines with line numbers and optional context.
async fn search_content(
    regex: &Regex,
    files: &[PathBuf],
    head_limit: usize,
    context_lines: usize,
) -> Result<ToolOutput, Temm1eError> {
    let mut results = Vec::new();
    let mut total_found: usize = 0;
    let mut limit_reached = false;

    'outer: for file in files {
        let content = match tokio::fs::read_to_string(file).await {
            Ok(c) => c,
            Err(_) => continue,
        };

        let lines: Vec<&str> = content.lines().collect();
        let file_path = file.display().to_string();

        // Find all matching line indices
        let matching_indices: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| regex.is_match(line))
            .map(|(i, _)| i)
            .collect();

        if matching_indices.is_empty() {
            continue;
        }

        if context_lines == 0 {
            // No context: just emit matching lines
            for &idx in &matching_indices {
                total_found += 1;
                if results.len() < head_limit {
                    results.push(format!("{}:{}: {}", file_path, idx + 1, lines[idx]));
                } else {
                    limit_reached = true;
                    // Keep counting total but stop collecting
                }
            }
            if limit_reached {
                // Count remaining matches across remaining files
                for remaining_file in files
                    .iter()
                    .skip(files.iter().position(|f| f == file).unwrap_or(0) + 1)
                {
                    if let Ok(c) = tokio::fs::read_to_string(remaining_file).await {
                        for line in c.lines() {
                            if regex.is_match(line) {
                                total_found += 1;
                            }
                        }
                    }
                }
                break 'outer;
            }
        } else {
            // With context: build groups of non-overlapping ranges
            let groups = build_context_groups(&matching_indices, context_lines, lines.len());

            for (group_idx, (start, end)) in groups.iter().enumerate() {
                for (line_idx, line) in lines.iter().enumerate().take(*end + 1).skip(*start) {
                    let is_match = matching_indices.contains(&line_idx);
                    if is_match {
                        total_found += 1;
                    }
                    if results.len() < head_limit {
                        results.push(format!("{}:{}: {}", file_path, line_idx + 1, line));
                    } else if is_match {
                        limit_reached = true;
                    }
                }

                // Add separator between non-adjacent groups within the same file
                if group_idx + 1 < groups.len() && results.len() < head_limit {
                    results.push("--".to_string());
                }
            }

            if limit_reached {
                for remaining_file in files
                    .iter()
                    .skip(files.iter().position(|f| f == file).unwrap_or(0) + 1)
                {
                    if let Ok(c) = tokio::fs::read_to_string(remaining_file).await {
                        for line in c.lines() {
                            if regex.is_match(line) {
                                total_found += 1;
                            }
                        }
                    }
                }
                break 'outer;
            }
        }
    }

    let mut output = results.join("\n");
    if total_found > head_limit {
        output.push_str(&format!(
            "\n[{} matches, showing first {}]",
            total_found, head_limit
        ));
    }

    if output.is_empty() {
        output = "No matches found.".to_string();
    }

    Ok(ToolOutput {
        content: output,
        is_error: false,
    })
}

/// Build context groups: merge overlapping ranges of [match_idx - context, match_idx + context].
fn build_context_groups(
    matching_indices: &[usize],
    context: usize,
    total_lines: usize,
) -> Vec<(usize, usize)> {
    let mut groups: Vec<(usize, usize)> = Vec::new();

    for &idx in matching_indices {
        let start = idx.saturating_sub(context);
        let end = (idx + context).min(total_lines.saturating_sub(1));

        if let Some(last) = groups.last_mut() {
            // Merge if overlapping or adjacent
            if start <= last.1 + 1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        groups.push((start, end));
    }

    groups
}

/// "count" mode: show match count per file.
async fn search_count(
    regex: &Regex,
    files: &[PathBuf],
    head_limit: usize,
) -> Result<ToolOutput, Temm1eError> {
    let mut results = Vec::new();
    let mut total_found: usize = 0;

    for file in files {
        let content = match tokio::fs::read_to_string(file).await {
            Ok(c) => c,
            Err(_) => continue,
        };

        let count = content.lines().filter(|line| regex.is_match(line)).count();

        if count > 0 {
            total_found += 1;
            if results.len() < head_limit {
                results.push(format!("{}: {}", file.display(), count));
            }
        }
    }

    let mut output = results.join("\n");
    if total_found > head_limit {
        output.push_str(&format!(
            "\n[{} matches, showing first {}]",
            total_found, head_limit
        ));
    }

    if output.is_empty() {
        output = "No matches found.".to_string();
    }

    Ok(ToolOutput {
        content: output,
        is_error: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::fs;

    fn make_ctx(workspace: &Path) -> ToolContext {
        ToolContext {
            channel: "cli".into(),
            workspace_path: workspace.to_path_buf(),
            session_id: "test-session".to_string(),
            chat_id: "test-chat".to_string(),
            read_tracker: None,
        }
    }

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            name: "code_grep".to_string(),
            arguments: args,
        }
    }

    #[test]
    fn test_name() {
        let tool = CodeGrepTool::new();
        assert_eq!(tool.name(), "code_grep");
    }

    #[test]
    fn test_schema() {
        let tool = CodeGrepTool::new();
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["pattern"].is_object());
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["properties"]["glob"].is_object());
        assert!(schema["properties"]["output_mode"].is_object());
        assert!(schema["properties"]["head_limit"].is_object());
        assert!(schema["properties"]["context"].is_object());
        assert!(schema["properties"]["case_insensitive"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "pattern");
    }

    #[tokio::test]
    async fn test_basic_search() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::write(
            root.join("hello.rs"),
            "fn main() {\n    println!(\"hello world\");\n}\n",
        )
        .await
        .unwrap();
        fs::write(root.join("other.txt"), "nothing here\n")
            .await
            .unwrap();

        let tool = CodeGrepTool::new();
        let ctx = make_ctx(root);
        let input = make_input(serde_json::json!({
            "pattern": "println"
        }));

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("hello.rs"));
        assert!(!output.content.contains("other.txt"));
    }

    #[tokio::test]
    async fn test_files_with_matches_mode() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::write(root.join("a.rs"), "let x = 42;\n").await.unwrap();
        fs::write(root.join("b.rs"), "let y = 99;\n").await.unwrap();
        fs::write(root.join("c.rs"), "no match\n").await.unwrap();

        let tool = CodeGrepTool::new();
        let ctx = make_ctx(root);
        let input = make_input(serde_json::json!({
            "pattern": "let",
            "output_mode": "files_with_matches"
        }));

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        // Should contain a.rs and b.rs but not c.rs
        assert!(output.content.contains("a.rs"));
        assert!(output.content.contains("b.rs"));
        assert!(!output.content.contains("c.rs"));
        // Should be file paths only, no line numbers
        assert!(!output.content.contains(":1:"));
    }

    #[tokio::test]
    async fn test_content_mode() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::write(
            root.join("sample.rs"),
            "line one\nfn search_me() {\n    body\n}\nline five\n",
        )
        .await
        .unwrap();

        let tool = CodeGrepTool::new();
        let ctx = make_ctx(root);
        let input = make_input(serde_json::json!({
            "pattern": "search_me",
            "output_mode": "content"
        }));

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        // Should show file:line_num: content format
        assert!(output.content.contains("sample.rs:2:"));
        assert!(output.content.contains("fn search_me()"));
    }

    #[tokio::test]
    async fn test_count_mode() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::write(
            root.join("multi.rs"),
            "foo bar\nfoo baz\nno match\nfoo qux\n",
        )
        .await
        .unwrap();
        fs::write(root.join("single.rs"), "foo once\nno match\n")
            .await
            .unwrap();

        let tool = CodeGrepTool::new();
        let ctx = make_ctx(root);
        let input = make_input(serde_json::json!({
            "pattern": "foo",
            "output_mode": "count"
        }));

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("multi.rs: 3"));
        assert!(output.content.contains("single.rs: 1"));
    }

    #[tokio::test]
    async fn test_case_insensitive() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::write(root.join("mixed.txt"), "foo\nFOO\nFoO\nbar\n")
            .await
            .unwrap();

        let tool = CodeGrepTool::new();
        let ctx = make_ctx(root);
        let input = make_input(serde_json::json!({
            "pattern": "FOO",
            "case_insensitive": true,
            "output_mode": "count"
        }));

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("mixed.txt: 3"));
    }

    #[tokio::test]
    async fn test_head_limit() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        // Create 10 files, each containing "target_pattern"
        for i in 0..10 {
            fs::write(root.join(format!("file_{:02}.txt", i)), "target_pattern\n")
                .await
                .unwrap();
        }

        let tool = CodeGrepTool::new();
        let ctx = make_ctx(root);
        let input = make_input(serde_json::json!({
            "pattern": "target_pattern",
            "output_mode": "files_with_matches",
            "head_limit": 3
        }));

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        // Should have truncation message
        assert!(output.content.contains("[10 matches, showing first 3]"));
        // Count actual file path lines (exclude the truncation message)
        let path_lines: Vec<&str> = output
            .content
            .lines()
            .filter(|l| !l.starts_with('['))
            .collect();
        assert_eq!(path_lines.len(), 3);
    }

    #[tokio::test]
    async fn test_glob_filter() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::write(root.join("code.rs"), "fn main() {}\n")
            .await
            .unwrap();
        fs::write(root.join("notes.txt"), "fn notes() {}\n")
            .await
            .unwrap();
        fs::write(root.join("style.css"), "fn style {}\n")
            .await
            .unwrap();

        let tool = CodeGrepTool::new();
        let ctx = make_ctx(root);
        let input = make_input(serde_json::json!({
            "pattern": "fn",
            "glob": "*.rs",
            "output_mode": "files_with_matches"
        }));

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("code.rs"));
        assert!(!output.content.contains("notes.txt"));
        assert!(!output.content.contains("style.css"));
    }

    #[tokio::test]
    async fn test_invalid_regex() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        let tool = CodeGrepTool::new();
        let ctx = make_ctx(root);
        let input = make_input(serde_json::json!({
            "pattern": "[invalid("
        }));

        let result = tool.execute(input, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("Invalid regex pattern"));
    }

    #[tokio::test]
    async fn test_content_mode_with_context() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::write(
            root.join("ctx.rs"),
            "line 1\nline 2\nMATCH here\nline 4\nline 5\nline 6\nline 7\n",
        )
        .await
        .unwrap();

        let tool = CodeGrepTool::new();
        let ctx = make_ctx(root);
        let input = make_input(serde_json::json!({
            "pattern": "MATCH",
            "output_mode": "content",
            "context": 1
        }));

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        // Should show line 2 (context before), line 3 (match), line 4 (context after)
        assert!(output.content.contains("ctx.rs:2:"));
        assert!(output.content.contains("ctx.rs:3:"));
        assert!(output.content.contains("ctx.rs:4:"));
        assert!(output.content.contains("MATCH here"));
        // Should NOT show lines outside the context window
        assert!(!output.content.contains("ctx.rs:1:"));
        assert!(!output.content.contains("ctx.rs:5:"));
    }

    #[tokio::test]
    async fn test_skips_ignored_dirs() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        // Create a file in an ignored directory
        fs::create_dir_all(root.join("node_modules")).await.unwrap();
        fs::write(root.join("node_modules/dep.js"), "findme\n")
            .await
            .unwrap();

        // Create a file in a normal directory
        fs::write(root.join("app.js"), "findme\n").await.unwrap();

        let tool = CodeGrepTool::new();
        let ctx = make_ctx(root);
        let input = make_input(serde_json::json!({
            "pattern": "findme",
            "output_mode": "files_with_matches"
        }));

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("app.js"));
        assert!(!output.content.contains("node_modules"));
    }

    #[tokio::test]
    async fn test_no_matches() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::write(root.join("empty_search.txt"), "nothing relevant\n")
            .await
            .unwrap();

        let tool = CodeGrepTool::new();
        let ctx = make_ctx(root);
        let input = make_input(serde_json::json!({
            "pattern": "zzz_nonexistent_zzz"
        }));

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert_eq!(output.content, "No matches found.");
    }
}
