//! Code glob tool — find files matching glob patterns within the workspace.

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use temm1e_core::types::error::Temm1eError;
use temm1e_core::{PathAccess, Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};

/// Maximum number of results returned to the model.
const MAX_RESULTS: usize = 500;

/// Directories to always skip during traversal.
const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    "target",
    ".next",
    ".nuxt",
    "dist",
    ".tox",
    ".eggs",
    ".venv",
    "venv",
];

#[derive(Default)]
pub struct CodeGlobTool;

impl CodeGlobTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for CodeGlobTool {
    fn name(&self) -> &str {
        "code_glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern. Supports * (any characters except /), \
         ** (recursive directory match), and ? (single character). \
         Results are sorted by modification time (newest first), limited to 500."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files (e.g., \"**/*.rs\", \"src/**/*.ts\", \"*.json\")"
                },
                "path": {
                    "type": "string",
                    "description": "Base directory to search in (relative to workspace or absolute). Defaults to workspace root."
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
        let pattern = input
            .arguments
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Temm1eError::Tool("Missing required parameter: pattern".into()))?;

        let base_str = input
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let base =
            crate::file::resolve_path(base_str, &ctx.workspace_path, crate::file::Operation::Read)?;

        // Compile the glob pattern into segments for matching
        let segments = parse_glob_pattern(pattern);

        // Walk the directory tree and collect matches
        let mut matches: Vec<(PathBuf, SystemTime)> = Vec::new();
        walk_and_match(&base, &segments, 0, &mut matches).await;

        let total = matches.len();

        // Sort by modification time, newest first
        matches.sort_by_key(|m| std::cmp::Reverse(m.1));

        // Limit results
        matches.truncate(MAX_RESULTS);

        if matches.is_empty() {
            return Ok(ToolOutput {
                content: format!("No files found matching pattern '{}'", pattern),
                is_error: false,
            });
        }

        // Build output with paths relative to workspace root
        let workspace = &ctx.workspace_path;
        let mut lines: Vec<String> = matches
            .iter()
            .map(|(path, _)| {
                path.strip_prefix(workspace)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| path.to_string_lossy().to_string())
            })
            .collect();

        if total > MAX_RESULTS {
            lines.push(format!(
                "[{} matches, showing first {}]",
                total, MAX_RESULTS
            ));
        }

        Ok(ToolOutput {
            content: lines.join("\n"),
            is_error: false,
        })
    }
}

// ---------------------------------------------------------------------------
// Glob pattern parsing and matching
// ---------------------------------------------------------------------------

/// A segment of a parsed glob pattern.
#[derive(Debug, Clone)]
enum GlobSegment {
    /// `**` — matches zero or more directory levels.
    DoubleStar,
    /// A literal segment that may contain `*` and `?` wildcards.
    Pattern(String),
}

/// Split a glob pattern string (e.g. `"src/**/*.rs"`) into segments.
fn parse_glob_pattern(pattern: &str) -> Vec<GlobSegment> {
    // Normalise separators to `/` for consistent splitting.
    let normalised = pattern.replace('\\', "/");
    let parts: Vec<&str> = normalised.split('/').filter(|s| !s.is_empty()).collect();
    let mut segments = Vec::with_capacity(parts.len());
    for part in parts {
        if part == "**" {
            segments.push(GlobSegment::DoubleStar);
        } else {
            segments.push(GlobSegment::Pattern(part.to_string()));
        }
    }
    segments
}

/// Check whether a file/directory name matches a single pattern segment
/// that may contain `*` and `?` wildcards.
///
/// Uses a simple recursive approach (sufficient for file name lengths).
fn matches_segment(name: &str, pattern: &str) -> bool {
    segment_match(name.as_bytes(), pattern.as_bytes())
}

fn segment_match(name: &[u8], pattern: &[u8]) -> bool {
    match (name.is_empty(), pattern.is_empty()) {
        (true, true) => return true,
        (_, true) => return false,
        (true, false) => {
            // Name exhausted — pattern must be all `*` to still match.
            return pattern.iter().all(|&b| b == b'*');
        }
        _ => {}
    }

    match pattern[0] {
        b'*' => {
            // `*` matches zero characters (advance pattern) or one character (advance name).
            segment_match(name, &pattern[1..]) || segment_match(&name[1..], pattern)
        }
        b'?' => {
            // `?` matches exactly one character.
            segment_match(&name[1..], &pattern[1..])
        }
        ch => {
            // Literal character — case-sensitive comparison.
            if name[0] == ch {
                segment_match(&name[1..], &pattern[1..])
            } else {
                false
            }
        }
    }
}

/// Recursively walk the directory tree starting at `dir`, matching entries
/// against `segments[seg_idx..]`.  Matched *files* are appended to `results`.
///
/// Uses `Box::pin` for the recursive async calls.
fn walk_and_match<'a>(
    dir: &'a Path,
    segments: &'a [GlobSegment],
    seg_idx: usize,
    results: &'a mut Vec<(PathBuf, SystemTime)>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        // Nothing left to match — we consumed all segments, so `dir` itself
        // was the final match target (handled by caller adding to results).
        if seg_idx >= segments.len() {
            return;
        }

        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(rd) => rd,
            Err(_) => return,
        };

        match &segments[seg_idx] {
            GlobSegment::DoubleStar => {
                // `**` matches zero or more directory levels.
                // Strategy: try matching the *remaining* segments starting at
                // the current directory (zero-level match) AND recurse into
                // every child directory with the same `**` still active.
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let path = entry.path();
                    let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);

                    if is_dir && IGNORED_DIRS.contains(&name.as_str()) {
                        continue;
                    }

                    // Try advancing past `**` — match this entry against the
                    // next non-`**` segment (if any).
                    let next = seg_idx + 1;
                    if next < segments.len() {
                        match &segments[next] {
                            GlobSegment::Pattern(pat) => {
                                if matches_segment(&name, pat) {
                                    if next + 1 >= segments.len() {
                                        // Terminal segment — only files match.
                                        if !is_dir {
                                            let mtime = entry
                                                .metadata()
                                                .await
                                                .ok()
                                                .and_then(|m| m.modified().ok())
                                                .unwrap_or(SystemTime::UNIX_EPOCH);
                                            results.push((path.clone(), mtime));
                                        }
                                    } else if is_dir {
                                        // More segments remain — recurse into this dir.
                                        walk_and_match(&path, segments, next + 1, results).await;
                                    }
                                }
                            }
                            GlobSegment::DoubleStar => {
                                // Consecutive `**/**` — just skip.
                                walk_and_match(dir, segments, next, results).await;
                                return;
                            }
                        }
                    } else {
                        // `**` is the last segment — everything below matches.
                        if !is_dir {
                            let mtime = entry
                                .metadata()
                                .await
                                .ok()
                                .and_then(|m| m.modified().ok())
                                .unwrap_or(SystemTime::UNIX_EPOCH);
                            results.push((path.clone(), mtime));
                        }
                    }

                    // Regardless, if it's a directory, continue the `**` recursion.
                    if is_dir {
                        walk_and_match(&path, segments, seg_idx, results).await;
                    }
                }
            }

            GlobSegment::Pattern(pat) => {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let path = entry.path();
                    let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);

                    if is_dir && IGNORED_DIRS.contains(&name.as_str()) {
                        continue;
                    }

                    if matches_segment(&name, pat) {
                        if seg_idx + 1 >= segments.len() {
                            // Terminal segment — only files count as results.
                            if !is_dir {
                                let mtime = entry
                                    .metadata()
                                    .await
                                    .ok()
                                    .and_then(|m| m.modified().ok())
                                    .unwrap_or(SystemTime::UNIX_EPOCH);
                                results.push((path.clone(), mtime));
                            }
                        } else if is_dir {
                            // More segments — recurse into matching directory.
                            walk_and_match(&path, segments, seg_idx + 1, results).await;
                        }
                    }
                }
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_name() {
        let tool = CodeGlobTool::new();
        assert_eq!(tool.name(), "code_glob");
    }

    #[test]
    fn test_schema() {
        let tool = CodeGlobTool::new();
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["pattern"].is_object());
        assert!(schema["properties"]["path"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("pattern")));
    }

    #[test]
    fn test_declarations() {
        let tool = CodeGlobTool::new();
        let decl = tool.declarations();
        assert_eq!(decl.file_access.len(), 1);
        assert!(matches!(&decl.file_access[0], PathAccess::Read(p) if p == "."));
        assert!(decl.network_access.is_empty());
        assert!(!decl.shell_access);
    }

    #[tokio::test]
    async fn test_find_rs_files() {
        let dir = tempdir().unwrap();
        let base = dir.path();

        // Create test files
        tokio::fs::write(base.join("main.rs"), "fn main() {}")
            .await
            .unwrap();
        tokio::fs::write(base.join("lib.rs"), "pub fn lib() {}")
            .await
            .unwrap();
        tokio::fs::write(base.join("readme.txt"), "hello")
            .await
            .unwrap();
        tokio::fs::write(base.join("data.json"), "{}")
            .await
            .unwrap();

        let tool = CodeGlobTool::new();
        let ctx = ToolContext {
            workspace_path: base.to_path_buf(),
            session_id: "test".into(),
            chat_id: "test".into(),
            read_tracker: None,
        };

        let input = ToolInput {
            name: "code_glob".into(),
            arguments: serde_json::json!({ "pattern": "*.rs" }),
        };

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);

        let lines: Vec<&str> = output.content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().any(|l| l.ends_with("main.rs")));
        assert!(lines.iter().any(|l| l.ends_with("lib.rs")));
        // Ensure non-rs files are excluded
        assert!(!lines.iter().any(|l| l.ends_with(".txt")));
        assert!(!lines.iter().any(|l| l.ends_with(".json")));
    }

    #[tokio::test]
    async fn test_recursive_glob() {
        let dir = tempdir().unwrap();
        let base = dir.path();

        // Create nested structure
        tokio::fs::create_dir_all(base.join("a/b/c")).await.unwrap();
        tokio::fs::write(base.join("top.txt"), "top").await.unwrap();
        tokio::fs::write(base.join("a/mid.txt"), "mid")
            .await
            .unwrap();
        tokio::fs::write(base.join("a/b/deep.txt"), "deep")
            .await
            .unwrap();
        tokio::fs::write(base.join("a/b/c/deepest.txt"), "deepest")
            .await
            .unwrap();
        // Non-matching file
        tokio::fs::write(base.join("a/b/skip.rs"), "fn skip() {}")
            .await
            .unwrap();

        let tool = CodeGlobTool::new();
        let ctx = ToolContext {
            workspace_path: base.to_path_buf(),
            session_id: "test".into(),
            chat_id: "test".into(),
            read_tracker: None,
        };

        let input = ToolInput {
            name: "code_glob".into(),
            arguments: serde_json::json!({ "pattern": "**/*.txt" }),
        };

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);

        let lines: Vec<&str> = output.content.lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().any(|l| l.ends_with("top.txt")));
        assert!(lines.iter().any(|l| l.ends_with("mid.txt")));
        assert!(lines.iter().any(|l| l.ends_with("deep.txt")));
        assert!(lines.iter().any(|l| l.ends_with("deepest.txt")));
        // .rs file must not appear
        assert!(!lines.iter().any(|l| l.ends_with(".rs")));
    }

    #[tokio::test]
    async fn test_empty_results() {
        let dir = tempdir().unwrap();
        let base = dir.path();

        // Create a file that will not match the pattern
        tokio::fs::write(base.join("hello.txt"), "hi")
            .await
            .unwrap();

        let tool = CodeGlobTool::new();
        let ctx = ToolContext {
            workspace_path: base.to_path_buf(),
            session_id: "test".into(),
            chat_id: "test".into(),
            read_tracker: None,
        };

        let input = ToolInput {
            name: "code_glob".into(),
            arguments: serde_json::json!({ "pattern": "**/*.zzz_nonexistent" }),
        };

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("No files found matching pattern"));
    }

    #[test]
    fn test_segment_matching() {
        // Basic wildcards
        assert!(matches_segment("main.rs", "*.rs"));
        assert!(matches_segment("lib.rs", "*.rs"));
        assert!(!matches_segment("main.txt", "*.rs"));
        assert!(matches_segment("a", "?"));
        assert!(!matches_segment("ab", "?"));
        assert!(matches_segment("test_file.rs", "test_*.rs"));
        assert!(matches_segment("file123.json", "file???.json"));
        // Literal match
        assert!(matches_segment("Cargo.toml", "Cargo.toml"));
        assert!(!matches_segment("cargo.toml", "Cargo.toml"));
    }

    #[test]
    fn test_parse_glob_pattern() {
        let segs = parse_glob_pattern("**/*.rs");
        assert_eq!(segs.len(), 2);
        assert!(matches!(&segs[0], GlobSegment::DoubleStar));
        assert!(matches!(&segs[1], GlobSegment::Pattern(p) if p == "*.rs"));

        let segs2 = parse_glob_pattern("src/lib.rs");
        assert_eq!(segs2.len(), 2);
        assert!(matches!(&segs2[0], GlobSegment::Pattern(p) if p == "src"));
        assert!(matches!(&segs2[1], GlobSegment::Pattern(p) if p == "lib.rs"));
    }
}
