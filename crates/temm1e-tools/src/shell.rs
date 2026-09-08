//! Shell tool — executes commands on the host via tokio::process::Command.

use async_trait::async_trait;
use temm1e_core::types::error::Temm1eError;
use temm1e_core::{Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};

/// Default command timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Maximum output size returned to the model (32 KB).
const MAX_OUTPUT_SIZE: usize = 32 * 1024;

/// Build a platform-appropriate `tokio::process::Command` that runs a single
/// user-supplied shell string.
///
/// - **Unix** (`macOS`, `Linux`, `BSD`): `sh -c <cmd>` — POSIX bytecode path,
///   byte-identical to the pre-Windows-support behavior.
/// - **Windows**: `powershell.exe -NoProfile -NonInteractive -Command <cmd>`
///   — PowerShell 5.1 ships preinstalled on every Windows 10 / 11 / Server
///   2016+ and cannot be uninstalled, so this path is always available.
///   `-NoProfile` skips user-profile loading (deterministic, faster),
///   `-NonInteractive` fails fast rather than prompting, `-Command` takes a
///   single string and does not require `-ExecutionPolicy Bypass` (policy
///   only gates `.ps1` files, never inline `-Command`). See GH-51.
///
/// Single-command form propagates native-executable exit codes correctly
/// (multi-statement edge cases can collapse to 1, which we still surface as
/// `is_error = true`). UTF-8 capture via `.output()` bypasses PowerShell's
/// pipeline-decoder, matching the Unix capture semantics.
fn build_shell_command(command: &str) -> tokio::process::Command {
    #[cfg(unix)]
    {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("powershell.exe");
        cmd.arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(command);
        cmd
    }
}

#[derive(Default)]
pub struct ShellTool;

impl ShellTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Execute a shell command on the host machine and return stdout/stderr. \
         Commands run in the session workspace directory with a 30-second timeout. \
         Use this for system tasks, file operations, package management, git, etc."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute (e.g., 'ls -la', 'cat file.txt', 'git status')"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 30, max: 300)"
                }
            },
            "required": ["command"]
        })
    }

    fn declarations(&self) -> ToolDeclarations {
        ToolDeclarations {
            file_access: Vec::new(),
            network_access: Vec::new(),
            shell_access: true,
        }
    }

    async fn execute(
        &self,
        input: ToolInput,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let command = input
            .arguments
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Temm1eError::Tool("Missing required parameter: command".into()))?;

        let timeout_secs = input
            .arguments
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .min(300);

        tracing::info!(command = %command, timeout = timeout_secs, "Executing shell command");

        let mut cmd = build_shell_command(command);
        cmd.current_dir(&ctx.workspace_path);

        let result = temm1e_core::process::run_bounded(
            &mut cmd,
            std::time::Duration::from_secs(timeout_secs),
            MAX_OUTPUT_SIZE,
        )
        .await;

        match result {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                let mut content = String::new();
                if !stdout.is_empty() {
                    content.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str("[stderr]\n");
                    content.push_str(&stderr);
                }

                if content.is_empty() {
                    content = format!(
                        "Command completed with exit code {}",
                        output.status.code().unwrap_or(-1)
                    );
                }

                // Truncate if too large (safe UTF-8 boundary)
                if content.len() > MAX_OUTPUT_SIZE {
                    let mut end = MAX_OUTPUT_SIZE;
                    while end > 0 && !content.is_char_boundary(end) {
                        end -= 1;
                    }
                    content.truncate(end);
                    content.push_str("\n... [output truncated]");
                }

                if output.truncated && !content.ends_with("... [output truncated]") {
                    content.push_str("\n... [output truncated]");
                }
                let is_error = !output.status.success();
                Ok(ToolOutput { content, is_error })
            }
            Err(temm1e_core::process::ProcessError::Io(e)) => Ok(ToolOutput {
                content: format!("Failed to execute command: {}", e),
                is_error: true,
            }),
            Err(temm1e_core::process::ProcessError::TimedOut) => Ok(ToolOutput {
                content: format!("Command timed out after {} seconds", timeout_secs),
                is_error: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use temm1e_core::ToolInput;

    fn ctx() -> ToolContext {
        ToolContext {
            workspace_path: PathBuf::from("."),
            session_id: "test".into(),
            chat_id: "test".into(),
            read_tracker: None,
        }
    }

    fn input(command: &str) -> ToolInput {
        ToolInput {
            name: "shell".into(),
            arguments: serde_json::json!({ "command": command }),
        }
    }

    /// Cross-platform: `echo hello` must round-trip on every supported host.
    /// On Unix this runs `sh -c "echo hello"`; on Windows it runs
    /// `powershell.exe -NoProfile -NonInteractive -Command "echo hello"`,
    /// where `echo` is an alias for `Write-Output`.
    #[tokio::test]
    async fn echo_roundtrips_on_host_shell() {
        let tool = ShellTool::new();
        let out = tool.execute(input("echo hello"), &ctx()).await.unwrap();
        assert!(!out.is_error, "echo should succeed: {}", out.content);
        assert!(
            out.content.contains("hello"),
            "echo output should contain 'hello', got: {:?}",
            out.content
        );
    }

    /// Non-zero exit code from the host shell must surface `is_error = true`.
    /// `exit 1` is portable across `sh` and PowerShell.
    #[tokio::test]
    async fn nonzero_exit_flags_is_error() {
        let tool = ShellTool::new();
        let out = tool.execute(input("exit 1"), &ctx()).await.unwrap();
        assert!(
            out.is_error,
            "exit 1 should flag is_error, got: {:?}",
            out.content
        );
    }

    /// Timeout path must return a deterministic timeout message rather than
    /// hang. Uses a 1-second timeout against a 60-second sleep.
    #[tokio::test]
    async fn timeout_trips_with_message() {
        let tool = ShellTool::new();
        let mut args = serde_json::Map::new();
        // `Start-Sleep -Seconds 60` works on PowerShell, `sleep 60` on sh.
        #[cfg(windows)]
        args.insert(
            "command".into(),
            serde_json::json!("Start-Sleep -Seconds 60"),
        );
        #[cfg(unix)]
        args.insert("command".into(), serde_json::json!("sleep 60"));
        args.insert("timeout".into(), serde_json::json!(1));
        let out = tool
            .execute(
                ToolInput {
                    name: "shell".into(),
                    arguments: serde_json::Value::Object(args),
                },
                &ctx(),
            )
            .await
            .unwrap();
        assert!(out.is_error, "timeout should flag is_error");
        assert!(
            out.content.contains("timed out"),
            "timeout content should mention timeout, got: {:?}",
            out.content
        );
    }
}
