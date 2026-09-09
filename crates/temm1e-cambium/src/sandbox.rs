//! # Sandbox: isolated workspace for cambium code modifications.
//!
//! The sandbox is a dedicated full git clone of the production repository,
//! living at `~/.temm1e/cambium/sandbox/`. ALL cambium code generation,
//! compilation, and testing happens inside this sandbox. The production
//! workspace is NEVER touched.
//!
//! ## Why a dedicated clone (not a worktree):
//!
//! - Total isolation: separate git database, separate `target/`, separate
//!   working tree. Even a catastrophic bug in cambium cannot affect the
//!   production codebase.
//! - Persistent build cache: after the first ~2.5 min cold compile,
//!   incremental builds take seconds. Self-grow sessions produce small
//!   diffs, so this matters a lot.
//! - Inspectable: `cd ~/.temm1e/cambium/sandbox/` and look at exactly
//!   what Tem did. Read the git log. Run tests yourself.
//! - Clean merge path: Tem pushes a branch from sandbox to origin. User
//!   reviews. User merges. Same workflow as any developer.
//!
//! ## Production safety guarantees:
//!
//! - Sandbox is created in `~/.temm1e/cambium/sandbox/`, NEVER in the
//!   production workspace.
//! - Sandbox sync uses `git fetch origin` + `git reset --hard origin/main`
//!   — this only pulls from origin, never pushes.
//! - Sandbox push targets a branch named `cambium/<session-id>` —
//!   NEVER main, NEVER force-push.
//! - All commands are validated and timeouts are enforced.

use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

use temm1e_core::types::error::Temm1eError;

/// Default location for the cambium sandbox.
pub fn default_sandbox_root() -> Option<PathBuf> {
    Some(
        temm1e_core::config::data_dir()
            .join("cambium")
            .join("sandbox"),
    )
}

/// Manages the lifecycle of a cambium sandbox: a dedicated git clone
/// for code modifications, isolated from the production workspace.
#[derive(Debug, Clone)]
pub struct Sandbox {
    /// Absolute path to the sandbox root (the cloned repo directory).
    pub path: PathBuf,
    /// URL or path of the upstream repository to clone from.
    pub upstream: String,
    /// Default upstream branch to sync from (e.g., "main").
    pub default_branch: String,
    /// Maximum seconds for any git/cargo command in the sandbox.
    pub command_timeout_secs: u64,
}

/// Result of running a command inside the sandbox.
#[derive(Debug, Clone)]
pub struct CommandResult {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Sandbox {
    /// Create a new sandbox descriptor (does not initialize on disk).
    pub fn new(path: PathBuf, upstream: String, default_branch: String) -> Self {
        Self {
            path,
            upstream,
            default_branch,
            command_timeout_secs: 600,
        }
    }

    /// Returns true if the sandbox directory exists and contains a git repo.
    pub async fn exists(&self) -> bool {
        let git_dir = self.path.join(".git");
        tokio::fs::metadata(&git_dir).await.is_ok()
    }

    /// Initialize the sandbox if it does not exist by cloning from upstream.
    /// If it already exists, this is a no-op (use `sync()` to update).
    pub async fn init(&self) -> Result<(), Temm1eError> {
        if self.exists().await {
            tracing::info!(
                target: "cambium",
                path = %self.path.display(),
                "Sandbox already initialized"
            );
            return Ok(());
        }

        // Ensure parent directory exists.
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                Temm1eError::Tool(format!(
                    "Failed to create sandbox parent {}: {e}",
                    parent.display()
                ))
            })?;
        }

        tracing::info!(
            target: "cambium",
            upstream = %self.upstream,
            path = %self.path.display(),
            "Cloning sandbox"
        );

        let result = self
            .run_command(
                "git",
                &["clone", &self.upstream, &self.path.to_string_lossy()],
                None,
            )
            .await?;

        if !result.success {
            return Err(Temm1eError::Tool(format!(
                "Sandbox clone failed: {}",
                result.stderr
            )));
        }

        Ok(())
    }

    /// Sync the sandbox to the latest upstream main: fetch + hard reset.
    /// This is read-only on the upstream — it never pushes.
    pub async fn sync(&self) -> Result<(), Temm1eError> {
        if !self.exists().await {
            return Err(Temm1eError::Tool(format!(
                "Sandbox does not exist at {}",
                self.path.display()
            )));
        }

        // git fetch origin
        let fetch = self.run_in_sandbox("git", &["fetch", "origin"]).await?;
        if !fetch.success {
            return Err(Temm1eError::Tool(format!(
                "git fetch failed: {}",
                fetch.stderr
            )));
        }

        // git checkout default_branch
        let checkout = self
            .run_in_sandbox("git", &["checkout", &self.default_branch])
            .await?;
        if !checkout.success {
            return Err(Temm1eError::Tool(format!(
                "git checkout failed: {}",
                checkout.stderr
            )));
        }

        // git reset --hard origin/<default_branch>
        let target = format!("origin/{}", self.default_branch);
        let reset = self
            .run_in_sandbox("git", &["reset", "--hard", &target])
            .await?;
        if !reset.success {
            return Err(Temm1eError::Tool(format!(
                "git reset failed: {}",
                reset.stderr
            )));
        }

        // git clean -fd to remove any leftover untracked files
        let clean = self.run_in_sandbox("git", &["clean", "-fd"]).await?;
        if !clean.success {
            return Err(Temm1eError::Tool(format!(
                "git clean failed: {}",
                clean.stderr
            )));
        }

        tracing::info!(
            target: "cambium",
            path = %self.path.display(),
            "Sandbox synced to upstream"
        );
        Ok(())
    }

    /// Create a new branch in the sandbox for a cambium session.
    /// Branch name will be prefixed with `cambium/`.
    pub async fn create_branch(&self, session_id: &str) -> Result<String, Temm1eError> {
        let branch = format!("cambium/{session_id}");
        let result = self
            .run_in_sandbox("git", &["checkout", "-b", &branch])
            .await?;
        if !result.success {
            return Err(Temm1eError::Tool(format!(
                "Failed to create branch {branch}: {}",
                result.stderr
            )));
        }
        Ok(branch)
    }

    /// Run `cargo check --workspace` in the sandbox.
    pub async fn cargo_check(&self) -> Result<CommandResult, Temm1eError> {
        self.run_in_sandbox("cargo", &["check", "--workspace"])
            .await
    }

    /// Run `cargo clippy` with deny-warnings in the sandbox.
    pub async fn cargo_clippy(&self) -> Result<CommandResult, Temm1eError> {
        self.run_in_sandbox(
            "cargo",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--",
                "-D",
                "warnings",
            ],
        )
        .await
    }

    /// Run `cargo fmt --check` in the sandbox.
    pub async fn cargo_fmt_check(&self) -> Result<CommandResult, Temm1eError> {
        self.run_in_sandbox("cargo", &["fmt", "--all", "--", "--check"])
            .await
    }

    /// Run `cargo test --workspace` in the sandbox.
    pub async fn cargo_test(&self) -> Result<CommandResult, Temm1eError> {
        self.run_in_sandbox("cargo", &["test", "--workspace"]).await
    }

    /// Get a list of files changed in the working directory (modified + untracked).
    pub async fn changed_files(&self) -> Result<Vec<PathBuf>, Temm1eError> {
        let result = self
            .run_in_sandbox("git", &["status", "--porcelain"])
            .await?;
        if !result.success {
            return Err(Temm1eError::Tool(format!(
                "git status failed: {}",
                result.stderr
            )));
        }
        let mut files = Vec::new();
        for line in result.stdout.lines() {
            // git status --porcelain format: XY <path>
            // XY is a 2-character status, then a space, then the path.
            if line.len() < 4 {
                continue;
            }
            let path_str = line[3..].trim();
            // Handle renames: "old -> new"
            if let Some(arrow) = path_str.find(" -> ") {
                files.push(PathBuf::from(&path_str[arrow + 4..]));
            } else {
                files.push(PathBuf::from(path_str));
            }
        }
        Ok(files)
    }

    /// Get diff stats: returns (lines_added, lines_removed) for the current
    /// uncommitted changes.
    pub async fn diff_stats(&self) -> Result<(usize, usize), Temm1eError> {
        let result = self.run_in_sandbox("git", &["diff", "--numstat"]).await?;
        if !result.success {
            return Err(Temm1eError::Tool(format!(
                "git diff failed: {}",
                result.stderr
            )));
        }
        let mut added = 0;
        let mut removed = 0;
        for line in result.stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Ok(a) = parts[0].parse::<usize>() {
                    added += a;
                }
                if let Ok(r) = parts[1].parse::<usize>() {
                    removed += r;
                }
            }
        }
        Ok((added, removed))
    }

    /// Stage all changes and create a commit on the current branch.
    /// Returns the new commit SHA.
    pub async fn commit_changes(&self, message: &str) -> Result<String, Temm1eError> {
        let add = self.run_in_sandbox("git", &["add", "-A"]).await?;
        if !add.success {
            return Err(Temm1eError::Tool(format!("git add failed: {}", add.stderr)));
        }
        let commit = self
            .run_in_sandbox("git", &["commit", "-m", message])
            .await?;
        if !commit.success {
            return Err(Temm1eError::Tool(format!(
                "git commit failed: {}",
                commit.stderr
            )));
        }
        let sha = self.run_in_sandbox("git", &["rev-parse", "HEAD"]).await?;
        if !sha.success {
            return Err(Temm1eError::Tool(format!(
                "git rev-parse failed: {}",
                sha.stderr
            )));
        }
        Ok(sha.stdout.trim().to_string())
    }

    /// Read a file from the sandbox.
    pub async fn read_file(&self, relative_path: &Path) -> Result<String, Temm1eError> {
        let full = self.path.join(relative_path);
        tokio::fs::read_to_string(&full)
            .await
            .map_err(|e| Temm1eError::Tool(format!("Failed to read {}: {e}", full.display())))
    }

    /// Write a file in the sandbox. Creates parent directories if needed.
    /// REJECTS any attempt to escape the sandbox via `..` segments.
    pub async fn write_file(&self, relative_path: &Path, content: &str) -> Result<(), Temm1eError> {
        // Reject path traversal attempts.
        if relative_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(Temm1eError::Tool(format!(
                "Path traversal rejected: {}",
                relative_path.display()
            )));
        }
        // Reject absolute paths.
        if relative_path.is_absolute() {
            return Err(Temm1eError::Tool(format!(
                "Absolute paths not allowed: {}",
                relative_path.display()
            )));
        }

        let full = self.path.join(relative_path);
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                Temm1eError::Tool(format!("Failed to create parent {}: {e}", parent.display()))
            })?;
        }
        tokio::fs::write(&full, content)
            .await
            .map_err(|e| Temm1eError::Tool(format!("Failed to write {}: {e}", full.display())))?;
        Ok(())
    }

    /// Delete the entire sandbox directory. Use with care.
    pub async fn destroy(&self) -> Result<(), Temm1eError> {
        if self.exists().await {
            tokio::fs::remove_dir_all(&self.path).await.map_err(|e| {
                Temm1eError::Tool(format!(
                    "Failed to remove sandbox {}: {e}",
                    self.path.display()
                ))
            })?;
        }
        Ok(())
    }

    /// Run a command in the sandbox directory.
    async fn run_in_sandbox(
        &self,
        program: &str,
        args: &[&str],
    ) -> Result<CommandResult, Temm1eError> {
        self.run_command(program, args, Some(&self.path)).await
    }

    /// Run a command with optional working directory.
    async fn run_command(
        &self,
        program: &str,
        args: &[&str],
        cwd: Option<&Path>,
    ) -> Result<CommandResult, Temm1eError> {
        let mut cmd = Command::new(program);
        cmd.args(args);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let timeout_dur = Duration::from_secs(self.command_timeout_secs);
        let output = match timeout(timeout_dur, cmd.output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                return Err(Temm1eError::Tool(format!("Failed to spawn {program}: {e}")));
            }
            Err(_) => {
                return Err(Temm1eError::Tool(format!(
                    "Command {program} timed out after {} seconds",
                    self.command_timeout_secs
                )));
            }
        };

        Ok(CommandResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn sandbox_descriptor_construction() {
        let path = PathBuf::from("/tmp/test-sandbox");
        let sb = Sandbox::new(
            path.clone(),
            "https://example.com/repo.git".to_string(),
            "main".to_string(),
        );
        assert_eq!(sb.path, path);
        assert_eq!(sb.upstream, "https://example.com/repo.git");
        assert_eq!(sb.default_branch, "main");
        assert_eq!(sb.command_timeout_secs, 600);
    }

    #[tokio::test]
    async fn nonexistent_sandbox_reports_not_exists() {
        let tmp = tempdir().unwrap();
        let sb = Sandbox::new(
            tmp.path().join("never-cloned"),
            "x".to_string(),
            "main".to_string(),
        );
        assert!(!sb.exists().await);
    }

    #[tokio::test]
    async fn write_file_rejects_path_traversal() {
        let tmp = tempdir().unwrap();
        let sb = Sandbox::new(
            tmp.path().to_path_buf(),
            "x".to_string(),
            "main".to_string(),
        );
        let result = sb.write_file(Path::new("../escape.txt"), "data").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("traversal"));
    }

    #[tokio::test]
    async fn write_file_rejects_absolute_path() {
        let tmp = tempdir().unwrap();
        let sb = Sandbox::new(
            tmp.path().to_path_buf(),
            "x".to_string(),
            "main".to_string(),
        );
        // `std::env::temp_dir()` returns an absolute path on every platform —
        // `/tmp/...` on Unix, `C:\Users\...\AppData\Local\Temp\...` on Windows.
        // A hardcoded `/etc/passwd` would test only Unix semantics because
        // `Path::is_absolute()` returns `false` for Unix-rooted paths on
        // Windows (Windows needs a drive letter or UNC prefix to be absolute).
        let absolute_outside_sandbox = std::env::temp_dir().join("temm1e_sandbox_test_target");
        let result = sb.write_file(&absolute_outside_sandbox, "data").await;
        assert!(
            result.is_err(),
            "expected sandbox to reject absolute path {}",
            absolute_outside_sandbox.display()
        );
        assert!(result.unwrap_err().to_string().contains("Absolute"));
    }

    #[tokio::test]
    async fn write_and_read_relative_file() {
        let tmp = tempdir().unwrap();
        let sb = Sandbox::new(
            tmp.path().to_path_buf(),
            "x".to_string(),
            "main".to_string(),
        );
        sb.write_file(Path::new("subdir/test.txt"), "hello")
            .await
            .unwrap();
        let read_back = sb.read_file(Path::new("subdir/test.txt")).await.unwrap();
        assert_eq!(read_back, "hello");
    }

    #[tokio::test]
    async fn destroy_removes_sandbox() {
        let tmp = tempdir().unwrap();
        let sandbox_path = tmp.path().join("to-destroy");
        tokio::fs::create_dir_all(&sandbox_path).await.unwrap();
        // Create a marker file inside
        tokio::fs::write(sandbox_path.join("marker"), "x")
            .await
            .unwrap();

        let sb = Sandbox::new(sandbox_path.clone(), "x".to_string(), "main".to_string());
        // Manually mark as a "git repo" so exists() returns true
        tokio::fs::create_dir_all(sandbox_path.join(".git"))
            .await
            .unwrap();
        assert!(sb.exists().await);

        sb.destroy().await.unwrap();
        assert!(!sb.exists().await);
        assert!(!sandbox_path.exists());
    }

    #[tokio::test]
    async fn destroy_nonexistent_is_noop() {
        let tmp = tempdir().unwrap();
        let sb = Sandbox::new(
            tmp.path().join("never-existed"),
            "x".to_string(),
            "main".to_string(),
        );
        let result = sb.destroy().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn default_sandbox_root_is_under_temm1e() {
        if let Some(root) = default_sandbox_root() {
            assert_eq!(
                root,
                temm1e_core::config::data_dir()
                    .join("cambium")
                    .join("sandbox")
            );
        }
    }
}
